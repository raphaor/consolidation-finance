"""Pipeline de consolidation en 4 étapes.

Chaque étape lit un niveau de stockage et produit le suivant. L'ordre A→B→C→D
corpond à la correspondance stockage ↔ traitement décrite dans
`docs/FLUX_CONSO.md` :

    A. Agrégation      stg_entry        → fact_entry [corporate]
    B. Reclassification corporate       → fact_entry [reclassified]
    C. Conversion      reclassified     → fact_entry [converted]
    D. Consolidation   converted        → fact_entry [consolidated]

Toute la logique est exprimée en SQL déclaratif (portage Rust direct via
duckdb-rs : une passe SQL par règle métier).
"""

from __future__ import annotations

import duckdb


# ─────────────────────────────────────────────────────────────────────────────
#  Étape A — Agrégation (→ niveau corporate)
# ─────────────────────────────────────────────────────────────────────────────

def step_a_aggregate(con: duckdb.DuckDBPyConnection) -> int:
    """Cumul des écritures source par entité.

    Lit la saisie brute (``stg_entry``), ne conserve que les flux sociaux
    d'origine (F00 et F20), agrège par (scenario, entity, entry_period,
    period, account, flow, currency) et stocke au niveau *corporate*.
    """
    con.execute(
        """
        INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, level, amount)
        SELECT
            scenario, entity, entry_period, period, account, flow, currency,
            'corporate' AS level,
            SUM(amount) AS amount
        FROM stg_entry
        WHERE flow IN ('F00', 'F20')
        GROUP BY scenario, entity, entry_period, period, account, flow, currency;
        """
    )
    return int(con.execute(
        "SELECT COUNT(*) FROM fact_entry WHERE level = 'corporate';"
    ).fetchone()[0])


# ─────────────────────────────────────────────────────────────────────────────
#  Étape B — Reclassification de périmètre (→ niveau reclassified)
# ─────────────────────────────────────────────────────────────────────────────
#  Travail en devise fonctionnelle (pas de conversion ici).
#    • Entité entrante  : F00 → F01 (l'ouverture de l'entrant est isolée en F01)
#    • Entité sortante  : collapse F00 + F20 → F98 (solde isolé en F98)
#    • Entité continue  : copie à l'identique
# ─────────────────────────────────────────────────────────────────────────────

def step_b_reclassify(con: duckdb.DuckDBPyConnection) -> int:
    """Reclasse les flux selon les variations de périmètre (en fonctionnel)."""

    con.execute(
        """
        INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, level, amount)
        SELECT
            scenario, entity, entry_period, period, account, flow, currency,
            'reclassified' AS level,
            SUM(amount)    AS amount
        FROM (
            -- 1) Entités continues : copie à l'identique
            SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
                   f.flow, f.currency, f.amount
            FROM fact_entry f
            JOIN sat_perimeter p
              ON p.entity = f.entity
             AND p.scenario = f.scenario
             AND p.period = f.entry_period
            WHERE f.level = 'corporate'
              AND NOT COALESCE(p.entree, FALSE)
              AND NOT COALESCE(p.sortie, FALSE)

            UNION ALL

            -- 2) Entités entrantes : F00 → F01, autres flux inchangés
            SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
                   CASE WHEN f.flow = 'F00' THEN 'F01' ELSE f.flow END AS flow,
                   f.currency, f.amount
            FROM fact_entry f
            JOIN sat_perimeter p
              ON p.entity = f.entity
             AND p.scenario = f.scenario
             AND p.period = f.entry_period
            WHERE f.level = 'corporate'
              AND COALESCE(p.entree, FALSE)
              AND NOT COALESCE(p.sortie, FALSE)

            UNION ALL

            -- 3) Entités sortantes : collapse F00 + F20 → F98 (par compte)
            SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
                   'F98' AS flow, f.currency, f.amount
            FROM fact_entry f
            JOIN sat_perimeter p
              ON p.entity = f.entity
             AND p.scenario = f.scenario
             AND p.period = f.entry_period
            WHERE f.level = 'corporate'
              AND COALESCE(p.sortie, FALSE)
              AND f.flow IN ('F00', 'F20')
        ) rec
        GROUP BY scenario, entity, entry_period, period, account, flow, currency;
        """
    )
    return int(con.execute(
        "SELECT COUNT(*) FROM fact_entry WHERE level = 'reclassified';"
    ).fetchone()[0])


# ─────────────────────────────────────────────────────────────────────────────
#  Étape C — Conversion multi-devises (→ niveau converted)
# ─────────────────────────────────────────────────────────────────────────────
#  Pour chaque ligne reclassifiée en devise ≠ présentation :
#    1. taux du flux via dim_flow.taux_conversion :
#         close_n1  → taux_close N-1
#         avg       → taux_moyen N
#         close_n   → taux_close N
#         terminal  → taux_close N (écart propre = 0)
#    2. montant converti = amount × taux
#    3. écart = amount × (taux_close_N − taux_du_flux), posté sur flux_ecart
#    4. lignes EUR : copie directe, aucun écart
#  Le niveau *converted* est exprimé en devise de présentation.
# ─────────────────────────────────────────────────────────────────────────────

def step_c_convert(
    con: duckdb.DuckDBPyConnection,
    presentation_currency: str = "EUR",
    current_period: str = "2024",
    prev_period: str = "2023",
) -> int:
    """Convertit les écritures en devise de présentation et génère les écarts."""

    params = {
        "presentation_currency": presentation_currency,
        "current_period": current_period,
        "prev_period": prev_period,
    }

    con.execute(
        """
        WITH conv AS (
            SELECT
                f.scenario, f.entity, f.entry_period, f.period, f.account,
                f.flow, f.currency, f.amount,
                fl.taux_conversion,
                fl.flux_ecart,
                -- Taux applicable au flux (1.0 si déjà en devise de présentation)
                CASE
                    WHEN f.currency = $presentation_currency THEN 1.0
                    WHEN fl.taux_conversion = 'close_n1' THEN r_n1.taux_close
                    WHEN fl.taux_conversion = 'avg'      THEN r_n.taux_moyen
                    WHEN fl.taux_conversion IN ('close_n', 'terminal')
                        THEN r_n.taux_close
                END AS taux_flux,
                -- Taux de clôture N (référence pour le calcul d'écart)
                CASE
                    WHEN f.currency = $presentation_currency THEN 1.0
                    ELSE r_n.taux_close
                END AS taux_close_n
            FROM fact_entry f
            JOIN dim_flow fl ON fl.code = f.flow
            LEFT JOIN sat_exchange_rate r_n
                   ON r_n.currency_source = f.currency
                  AND r_n.period = $current_period
            LEFT JOIN sat_exchange_rate r_n1
                   ON r_n1.currency_source = f.currency
                  AND r_n1.period = $prev_period
            WHERE f.level = 'reclassified'
        )
        INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, level, amount)
        -- Montants convertis (tous flux, exprimés en devise de présentation)
        SELECT scenario, entity, entry_period, period, account, flow,
               $presentation_currency AS currency,
               'converted' AS level,
               amount * taux_flux AS amount
        FROM conv
        UNION ALL
        -- Lignes d'écart (devise ≠ présentation, flux porteur d'un flux_ecart, écart ≠ 0)
        SELECT scenario, entity, entry_period, period, account, flux_ecart AS flow,
               $presentation_currency AS currency,
               'converted' AS level,
               amount * (taux_close_n - taux_flux) AS amount
        FROM conv
        WHERE currency <> $presentation_currency
          AND flux_ecart IS NOT NULL
          AND ABS(amount * (taux_close_n - taux_flux)) >= 0.005;
        """,
        params,
    )
    return int(con.execute(
        "SELECT COUNT(*) FROM fact_entry WHERE level = 'converted';"
    ).fetchone()[0])


# ─────────────────────────────────────────────────────────────────────────────
#  Étape D — Consolidation (→ niveau consolidated)
# ─────────────────────────────────────────────────────────────────────────────
#  Application des méthodes de consolidation (natif MVP) :
#    • globale         : copie à 100 % (pct_integration = 1.0)
#    • proportionnelle : amount × pct_integration
#    • équivalence     : EXCLUE du MVP (non traitée)
# ─────────────────────────────────────────────────────────────────────────────

def step_d_consolidate(con: duckdb.DuckDBPyConnection) -> int:
    """Applique la méthode d'intégration de chaque entité."""

    con.execute(
        """
        INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, level, amount)
        SELECT
            f.scenario, f.entity, f.entry_period, f.period, f.account, f.flow, f.currency,
            'consolidated' AS level,
            f.amount * COALESCE(p.pct_integration, 1.0) AS amount
        FROM fact_entry f
        JOIN sat_perimeter p
          ON p.entity = f.entity
         AND p.scenario = f.scenario
         AND p.period = f.entry_period
        WHERE f.level = 'converted'
          AND p.methode IN ('globale', 'proportionnelle');  -- équivalence hors MVP
        """
    )
    return int(con.execute(
        "SELECT COUNT(*) FROM fact_entry WHERE level = 'consolidated';"
    ).fetchone()[0])


# ─────────────────────────────────────────────────────────────────────────────
#  Orchestration
# ─────────────────────────────────────────────────────────────────────────────

def run_pipeline(
    con: duckdb.DuckDBPyConnection,
    presentation_currency: str = "EUR",
    current_period: str = "2024",
    prev_period: str = "2023",
) -> dict[str, int]:
    """Enchaîne les 4 étapes et renvoie le nombre de lignes par niveau."""
    counts = {
        "corporate": step_a_aggregate(con),
        "reclassified": step_b_reclassify(con),
        "converted": step_c_convert(
            con,
            presentation_currency=presentation_currency,
            current_period=current_period,
            prev_period=prev_period,
        ),
        "consolidated": step_d_consolidate(con),
    }
    return counts
