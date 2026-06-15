"""Définition du schéma DuckDB : dimensions, tables satellites, fait.

Le modèle reprend `docs/MODELE_DONNEES.md` et `docs/FLUX_CONSO.md`.

4 niveaux de stockage des écritures (colonne ``level`` de ``fact_entry``) :
    corporate    → données saisies agrégées (devise fonctionnelle)
    reclassified → après reclassifications de périmètre (devise fonctionnelle)
    converted    → après conversion multi-devises (devise de présentation)
    consolidated → après application des méthodes (devise de présentation)

Une table de staging ``stg_entry`` reçoit la saisie brute (liasses CSV).
L'étape A lit cette table et produit le niveau *corporate*.
"""

from __future__ import annotations

import duckdb


# ─────────────────────────────────────────────────────────────────────────────
#  Ordre DDL — les dimensions d'abord, puis les satellites, puis le fait
# ─────────────────────────────────────────────────────────────────────────────

_DDL: list[str] = [
    # --- Séquence d'identifiants pour la table de faits ---
    "CREATE SEQUENCE IF NOT EXISTS seq_entry START 1;",

    # --- Dimensions (master data) ---
    """
    CREATE TABLE dim_scenario (
        code     TEXT PRIMARY KEY,
        libelle  TEXT,
        type     TEXT,        -- réel / budget / prévision
        statut   TEXT         -- ouvert / verrouillé
    );
    """,
    """
    CREATE TABLE dim_entity (
        code                 TEXT PRIMARY KEY,
        libelle              TEXT,
        devise_fonctionnelle TEXT,   -- code ISO (EUR, USD, GBP…)
        entite_parent        TEXT,   -- code entité parente (hiérarchie de groupe)
        statut               TEXT
    );
    """,
    """
    CREATE TABLE dim_period (
        code       TEXT PRIMARY KEY,
        libelle    TEXT,
        type       TEXT,          -- mois / trimestre / année / exercice
        date_debut DATE,
        date_fin   DATE,
        statut     TEXT           -- clôturé / ouvert
    );
    """,
    """
    CREATE TABLE dim_account (
        code             TEXT PRIMARY KEY,
        libelle          TEXT,
        classe           TEXT CHECK (classe IN ('bilan', 'resultat', 'equity', 'flux')),
        capitaux_propres BOOLEAN,       -- identifie les capitaux propres (mise en équivalence)
        compte_parent    TEXT           -- hiérarchie d'agrégation
    );
    """,
    """
    CREATE TABLE dim_flow (
        code             TEXT PRIMARY KEY,
        libelle          TEXT,
        taux_conversion  TEXT CHECK (taux_conversion IN ('close_n1', 'avg', 'close_n', 'terminal')),
        flux_ecart       TEXT           -- flux d'écart de conversion associé (NULL pour les terminaux)
    );
    """,
    """
    CREATE TABLE dim_currency (
        code_iso  TEXT PRIMARY KEY,
        libelle   TEXT,
        decimales INT
    );
    """,

    # --- Tables satellites (règles de consolidation) ---
    """
    CREATE TABLE sat_perimeter (
        entity          TEXT,
        scenario        TEXT,
        period          TEXT,          -- correspond au Entry_period (exercice clôturé)
        methode         TEXT CHECK (methode IN ('globale', 'proportionnelle', 'équivalence')),
        pct_interet     DECIMAL(10,4),
        pct_integration DECIMAL(10,4), -- % de contrôle (1.0 pour la globale)
        entree          BOOLEAN DEFAULT FALSE,
        sortie          BOOLEAN DEFAULT FALSE,
        PRIMARY KEY (entity, scenario, period)
    );
    """,
    """
    CREATE TABLE sat_exchange_rate (
        currency_source TEXT,   -- devise source (à convertir vers la présentation)
        period          TEXT,
        taux_close      DECIMAL(18,8),
        taux_moyen      DECIMAL(18,8),
        PRIMARY KEY (currency_source, period)
    );
    """,

    # --- Staging : saisie brute (format liasse CSV) ---
    """
    CREATE TABLE stg_entry (
        scenario     TEXT,
        entity       TEXT,
        entry_period TEXT,
        period       TEXT,
        account      TEXT,
        flow         TEXT,
        currency     TEXT,
        partner      TEXT,
        share        TEXT,
        analysis     TEXT,
        audit_id     TEXT,
        amount       DECIMAL(18,2)
    );
    """,

    # --- Table de faits : écritures aux 4 niveaux de stockage ---
    """
    CREATE TABLE fact_entry (
        id           INTEGER DEFAULT nextval('seq_entry'),
        scenario     TEXT,
        entity       TEXT,
        entry_period TEXT,
        period       TEXT,
        account      TEXT,
        flow         TEXT,
        currency     TEXT,
        partner      TEXT,
        share        TEXT,
        analysis     TEXT,
        audit_id     TEXT,
        level        TEXT CHECK (level IN ('corporate', 'reclassified', 'converted', 'consolidated')),
        amount       DECIMAL(18,2),
        PRIMARY KEY (id)
    );
    """,
]


def create_schema(con: duckdb.DuckDBPyConnection) -> None:
    """Crée toutes les tables (idempotent grâce aux IF NOT EXISTS sur la séquence)."""
    # On supprime d'abord pour garantir un état propre en cas de re-exécution.
    con.execute("DROP TABLE IF EXISTS fact_entry;")
    con.execute("DROP TABLE IF EXISTS stg_entry;")
    con.execute("DROP TABLE IF EXISTS sat_exchange_rate;")
    con.execute("DROP TABLE IF EXISTS sat_perimeter;")
    con.execute("DROP TABLE IF EXISTS dim_currency;")
    con.execute("DROP TABLE IF EXISTS dim_flow;")
    con.execute("DROP TABLE IF EXISTS dim_account;")
    con.execute("DROP TABLE IF EXISTS dim_period;")
    con.execute("DROP TABLE IF EXISTS dim_entity;")
    con.execute("DROP TABLE IF EXISTS dim_scenario;")
    con.execute("DROP SEQUENCE IF EXISTS seq_entry;")

    for stmt in _DDL:
        con.execute(stmt)
