"""Sorties du prototype : bilan par flux, comparaison des niveaux, validation.

Toutes les restitutions sont calculées par requête SQL (format long) puis
mises en forme (pivot) en Python, pour rester lisibles et faciles à porter.
"""

from __future__ import annotations

from decimal import Decimal

import duckdb

from .validate import (
    COMPONENT_FLOWS,
    validate_consolidated,
    validate_functional,
)

# Ordre d'affichage des flux
FLOW_ORDER = ["F00", "F01", "F20", "F80", "F81", "F98", "F99"]


# ─────────────────────────────────────────────────────────────────────────────
#  Helpers de mise en forme
# ─────────────────────────────────────────────────────────────────────────────

def _fmt(x: Decimal | float | int | None) -> str:
    """Formate un montant : 2 décimales, séparateur de milliers ; '-' si nul."""
    if x is None:
        return "-"
    d = x if isinstance(x, Decimal) else Decimal(str(x))
    if abs(d) < Decimal("0.005"):
        return "-"
    return f"{d:,.2f}"


def _load_grid(
    con: duckdb.DuckDBPyConnection, level: str
) -> dict[tuple[str, str], Decimal]:
    """Renvoie une grille (account, flow) → montant pour un niveau donné."""
    rows = con.execute(
        """
        SELECT account, flow, SUM(amount) AS amount
        FROM fact_entry
        WHERE level = ?
        GROUP BY account, flow
        """,
        [level],
    ).fetchall()
    return {(acc, fl): Decimal(str(amt)) for acc, fl, amt in rows}


# ─────────────────────────────────────────────────────────────────────────────
#  1. Bilan par flux (comptes × flux, niveau consolidated, F99 reconstruit)
# ─────────────────────────────────────────────────────────────────────────────

def bilan_par_flux(con: duckdb.DuckDBPyConnection, level: str = "consolidated") -> None:
    """Affiche le bilan par flux : comptes en lignes × flux en colonnes.

    La colonne F99 est RECONSTRUITE comme la somme des flux constitutifs
    (F00+F01+F20+F80+F81+F98), conformément à l'identité de reconstruction.
    """
    grid = _load_grid(con, level)
    accounts = sorted({acc for acc, _ in grid})

    title_devise = (
        "devise de présentation (EUR)" if level in ("converted", "consolidated")
        else "devise fonctionnelle"
    )
    print(f"\n{'═' * 88}")
    print(f"  BILAN PAR FLUX  —  niveau « {level} »  ({title_devise})")
    print(f"{'═' * 88}")

    col_w = 13
    header = f"  {'Compte':<22}" + "".join(f"{fl:>{col_w}}" for fl in FLOW_ORDER)
    print(header)
    print("  " + "─" * (22 + col_w * len(FLOW_ORDER)))

    for acc in accounts:
        # F99 reconstruit = somme des flux constitutifs présents à ce niveau
        f99 = sum(
            (grid.get((acc, fl), Decimal("0")) for fl in COMPONENT_FLOWS),
            Decimal("0"),
        )
        cells = []
        for fl in FLOW_ORDER:
            val = f99 if fl == "F99" else grid.get((acc, fl), Decimal("0"))
            cells.append(_fmt(val))
        print(f"  {acc:<22}" + "".join(f"{c:>{col_w}}" for c in cells))


# ─────────────────────────────────────────────────────────────────────────────
#  2. Comparaison des 4 niveaux pour un compte donné
# ─────────────────────────────────────────────────────────────────────────────

def compare_levels(con: duckdb.DuckDBPyConnection, account: str) -> None:
    """Affiche, pour un compte, le détail par flux aux 4 niveaux de stockage.

    Met en évidence l'effet de chaque étape : agrégation → reclassification
    (F00→F01 / collapse→F98) → conversion (écarts F80/F81, passage en EUR)
    → consolidation (% d'intégration).
    """
    levels = ["corporate", "reclassified", "converted", "consolidated"]
    level_desc = {
        "corporate":    "Agrégation (fonctionnel)",
        "reclassified": "Reclassification (fonctionnel)",
        "converted":    "Conversion (EUR)",
        "consolidated": "Consolidation (EUR)",
    }

    print(f"\n{'═' * 88}")
    print(f"  COMPARAISON DES 4 NIVEAUX  —  compte « {account} »")
    print(f"{'═' * 88}")

    col_w = 13
    header = f"  {'Niveau':<28}" + "".join(f"{fl:>{col_w}}" for fl in FLOW_ORDER)
    print(header)
    print("  " + "─" * (28 + col_w * len(FLOW_ORDER)))

    for lvl in levels:
        rows = con.execute(
            """
            SELECT flow, SUM(amount) AS amount
            FROM fact_entry
            WHERE level = ? AND account = ?
            GROUP BY flow
            """,
            [lvl, account],
        ).fetchall()
        grid = {fl: Decimal(str(amt)) for fl, amt in rows}

        f99 = sum(
            (grid.get(fl, Decimal("0")) for fl in COMPONENT_FLOWS),
            Decimal("0"),
        )
        cells = []
        for fl in FLOW_ORDER:
            val = f99 if fl == "F99" else grid.get(fl, Decimal("0"))
            cells.append(_fmt(val))
        label = level_desc[lvl]
        print(f"  {label:<28}" + "".join(f"{c:>{col_w}}" for c in cells))


# ─────────────────────────────────────────────────────────────────────────────
#  3. Résultat de validation (✓ / ✗ par compte)
# ─────────────────────────────────────────────────────────────────────────────

def print_validation(con: duckdb.DuckDBPyConnection) -> bool:
    """Affiche le résultat des vérifications d'identité et renvoie le statut global."""
    print(f"\n{'═' * 88}")
    print("  VALIDATION — Identité de reconstruction F99 = F00 + F01 + F20 + F80 + F81 + F98")
    print(f"{'═' * 88}")

    # --- a) Côté consolidé (devise de présentation, écarts inclus) ---
    print("\n  (a) Niveau CONSOLIDÉ (devise de présentation, écarts inclus)")
    print(f"  {'Compte':<24}{'F99':>16}{'Σ composantes':>18}{'écart':>12}   statut")
    print("  " + "─" * 78)
    results_c = validate_consolidated(con)
    all_ok = True
    for r in results_c:
        if not r.ok:
            all_ok = False
        mark = "✓ OK" if r.ok else "✗ ÉCHEC"
        print(f"  {r.account:<24}{_fmt(r.f99):>16}{_fmt(r.somme):>18}"
              f"{_fmt(r.ecart):>12}   {mark}")

    # --- b) Côté fonctionnel (reclassified, écarts = 0) ---
    print("\n  (b) Niveau RECLASSIFIÉ (devise fonctionnelle, écarts = 0)")
    print("      identité réduite : F99 = F00 + F01 + F20 + F98")
    print(f"  {'Compte':<24}{'F99':>16}{'Σ composantes':>18}{'écart':>12}   statut")
    print("  " + "─" * 78)
    for r in validate_functional(con):
        if not r.ok:
            all_ok = False
        mark = "✓ OK" if r.ok else "✗ ÉCHEC"
        print(f"  {r.account:<24}{_fmt(r.f99):>16}{_fmt(r.somme):>18}"
              f"{_fmt(r.ecart):>12}   {mark}")

    verdict = "✓ TOUTES LES IDENTITÉS TIENNENT" if all_ok else "✗ IDENTITÉ(S) EN ÉCHEC"
    print(f"\n  Verdict global : {verdict}")
    return all_ok


# ─────────────────────────────────────────────────────────────────────────────
#  Bonus : synthèse des volumes par niveau
# ─────────────────────────────────────────────────────────────────────────────

def print_level_counts(con: duckdb.DuckDBPyConnection) -> None:
    """Affiche le nombre de lignes stockées à chaque niveau."""
    print(f"\n{'─' * 88}")
    print("  Volumes par niveau de stockage")
    print(f"{'─' * 88}")
    rows = con.execute(
        """
        SELECT level, COUNT(*) AS n
        FROM fact_entry
        GROUP BY level
        ORDER BY CASE level
            WHEN 'corporate' THEN 1
            WHEN 'reclassified' THEN 2
            WHEN 'converted' THEN 3
            WHEN 'consolidated' THEN 4
        END
        """
    ).fetchall()
    for level, n in rows:
        print(f"    {level:<14} {n:>6} lignes")
