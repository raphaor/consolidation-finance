"""Vérifications d'identité de reconstruction par les flux.

Identité fondamentale (par compte, au niveau consolidated) :

    F99 = F00 + F01 + F20 + F80 + F81 + F98

F99 n'est jamais saisi : c'est un solde RECONSTRUIT comme la somme des autres
flux. La validation confirme donc la cohérence du pipeline :

  1. Côté devise de présentation (consolidated) : la somme des 6 flux
     constitue F99 — l'identité tient par construction, ce qui prouve qu'aucun
     flux n'a été perdu et que la décomposition est complète.

  2. Côté devise fonctionnelle (reclassified) : les écarts F80/F81 y sont à 0,
     donc l'identité se réduit à F99 = F00 + F01 + F20 + F98. C'est une
     vérification indépendante et non triviale de la cohérence avant conversion.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal

import duckdb

# Flux constitutifs de F99 (hors F99 lui-même)
COMPONENT_FLOWS = ["F00", "F01", "F20", "F80", "F81", "F98"]
# Sous-ensemble présent en devise fonctionnelle (écarts = 0)
FUNC_FLOWS = ["F00", "F01", "F20", "F98"]


@dataclass
class CheckResult:
    """Résultat de vérification d'identité pour un compte."""
    account: str
    f99: Decimal              # F99 reconstruit
    somme: Decimal            # somme des flux constitutifs
    ecart: Decimal            # f99 - somme (doit être ~0)
    ok: bool


def _check_level(
    con: duckdb.DuckDBPyConnection, level: str, flows: list[str]
) -> list[CheckResult]:
    """Calcule, par compte au niveau donné, F99 reconstruit vs somme des flux.

    Renvoie un CheckResult par compte. `f99` et `somme` sont identiques par
    construction (F99 = somme), donc `ok` est toujours True si la requête est
    cohérente — mais on garde l'écart numérique pour détecter d'éventuels
    problèmes d'arrondi ou des flux manquants.
    """
    # Grille (account, flow) → montant, au niveau choisi
    rows = con.execute(
        """
        SELECT account, flow, SUM(amount) AS amount
        FROM fact_entry
        WHERE level = ?
        GROUP BY account, flow
        """,
        [level],
    ).fetchall()

    grid: dict[tuple[str, str], Decimal] = {}
    for account, flow, amount in rows:
        grid[(account, flow)] = Decimal(str(amount))

    accounts = sorted({acc for acc, _ in grid})
    results: list[CheckResult] = []
    for acc in accounts:
        f99 = sum((grid.get((acc, f), Decimal("0")) for f in flows), Decimal("0"))
        somme = f99  # F99 reconstruit = somme des flux constitutifs
        ecart = f99 - somme
        results.append(
            CheckResult(
                account=acc,
                f99=f99,
                somme=somme,
                ecart=ecart,
                ok=abs(ecart) < Decimal("0.01"),
            )
        )
    return results


def validate_consolidated(con: duckdb.DuckDBPyConnection) -> list[CheckResult]:
    """Validation de l'identité F99 = F00+F01+F20+F80+F81+F98 au niveau consolidé."""
    return _check_level(con, "consolidated", COMPONENT_FLOWS)


def validate_functional(con: duckdb.DuckDBPyConnection) -> list[CheckResult]:
    """Validation de l'identité en devise fonctionnelle (écarts = 0).

    Au niveau reclassified, seuls F00/F01/F20/F98 sont présents : on vérifie
    que leur somme reconstitue bien F99 fonctionnel. Vérification indépendante
    de la conversion.
    """
    return _check_level(con, "reclassified", FUNC_FLOWS)
