"""Données de test — groupe multi-devise avec entrée et sortie de périmètre.

Reprend le scénario de `simulations/consolidation_sim.py` (approche A :
reclassification AVANT conversion) et l'adapte au modèle dimensionnel.

Groupe de test :
    Mère M        — EUR, intégration globale 100 %, périmètre continu
    Filiale A     — USD, intégration globale 100 %, ENTRE en N
    Filiale B     — GBP, intégration globale 100 %, SORT en N

Période traitée : Entry_period = Period = '2024' ; exercice précédent '2023'.
Devise de présentation : EUR.
"""

from __future__ import annotations

import duckdb

# ─────────────────────────────────────────────────────────────────────────────
#  Master data
# ─────────────────────────────────────────────────────────────────────────────

SCENARIOS = [
    # (code, libelle, type, statut)
    ("REEL", "Réel 2024", "réel", "ouvert"),
]

ENTITIES = [
    # (code, libelle, devise_fonctionnelle, entite_parent, statut)
    ("M", "Mère",       "EUR", None, "actif"),
    ("A", "Filiale A",  "USD", "M",  "actif"),
    ("B", "Filiale B",  "GBP", "M",  "actif"),
]

PERIODS = [
    # (code, libelle, type, date_debut, date_fin, statut)
    ("2023", "Exercice 2023", "exercice", "2023-01-01", "2023-12-31", "clôturé"),
    ("2024", "Exercice 2024", "exercice", "2024-01-01", "2024-12-31", "ouvert"),
]

ACCOUNTS = [
    # (code, libelle, classe, capitaux_propres, compte_parent)
    ("100_Capital",         "Capital",            "equity",   True,  None),
    ("200_Immobilisations", "Immobilisations",    "bilan",    False, None),
    ("300_Stocks",          "Stocks",             "bilan",    False, None),
    ("400_Resultat",        "Résultat",           "resultat", False, None),
]

# Catalogue des flux — cf. docs/FLUX_CONSO.md §6
FLOWS = [
    # (code, libelle, taux_conversion, flux_ecart)
    ("F00", "Ouverture",                "close_n1", "F80"),
    ("F01", "Entrée périmètre",         "close_n1", "F80"),
    ("F20", "Variation",                "avg",      "F81"),
    ("F80", "Écart conv. ouverture",    "terminal", None),
    ("F81", "Écart conv. variation",    "terminal", None),
    ("F98", "Sortie périmètre",         "terminal", None),
    ("F99", "Clôture",                  "close_n",  None),
]

CURRENCIES = [
    # (code_iso, libelle, decimales)
    ("EUR", "Euro",         2),
    ("USD", "Dollar US",    2),
    ("GBP", "Livre sterling", 2),
]

# ─────────────────────────────────────────────────────────────────────────────
#  Tables satellites
# ─────────────────────────────────────────────────────────────────────────────

# Périmètre : (entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie)
PERIMETER = [
    ("M", "REEL", "2024", "globale",        1.00, 1.00, False, False),
    ("A", "REEL", "2024", "globale",        1.00, 1.00, True,  False),  # ENTRE en N
    ("B", "REEL", "2024", "globale",        1.00, 1.00, False, True),   # SORT en N
]

# Taux de change vers EUR :
#   - period '2023' : taux clôture N-1 (utilisé par close_n1)
#   - period '2024' : taux clôture N (close_n / terminal) et taux moyen (avg)
RATES = [
    # (currency_source, period, taux_close, taux_moyen)
    ("USD", "2023", 0.92, None),   # close_n1
    ("USD", "2024", 0.90, 0.95),   # close_n=0.90, avg=0.95
    ("GBP", "2023", 1.15, None),   # close_n1
    ("GBP", "2024", 1.12, 1.18),   # close_n=1.12, avg=1.18
]

# ─────────────────────────────────────────────────────────────────────────────
#  Écritures brutes (saisie source) — flux sociaux F00 et F20 uniquement
# ─────────────────────────────────────────────────────────────────────────────
#  Colonnes : scenario, entity, entry_period, period, account, flow,
#             currency, partner, share, analysis, audit_id, amount
#  Note : pour démontrer l'agrégation (étape A), le F20 du résultat de M
#         est volontairement éclaté en deux lignes (500 + 300 = 800).
# ─────────────────────────────────────────────────────────────────────────────

_RAW = [
    # ── Mère M (EUR) — périmètre continu ──
    ("REEL", "M", "2024", "2024", "100_Capital",         "F00", "EUR", None, None, None, "S-M-001", 10000),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F00", "EUR", None, None, None, "S-M-002",  5000),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-003",   500),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-004",   300),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F00", "EUR", None, None, None, "S-M-005", 12000),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F20", "EUR", None, None, None, "S-M-006",   500),
    ("REEL", "M", "2024", "2024", "300_Stocks",          "F00", "EUR", None, None, None, "S-M-007",  3000),

    # ── Filiale A (USD) — ENTRE en N ──
    ("REEL", "A", "2024", "2024", "100_Capital",         "F00", "USD", None, None, None, "S-A-001",  5000),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F00", "USD", None, None, None, "S-A-002",  2000),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F20", "USD", None, None, None, "S-A-003",   300),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F00", "USD", None, None, None, "S-A-004",  8000),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F20", "USD", None, None, None, "S-A-005",   400),

    # ── Filiale B (GBP) — SORT en N ──
    ("REEL", "B", "2024", "2024", "100_Capital",         "F00", "GBP", None, None, None, "S-B-001",  4000),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F00", "GBP", None, None, None, "S-B-002",  1500),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F20", "GBP", None, None, None, "S-B-003",   200),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F00", "GBP", None, None, None, "S-B-004",  6000),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F20", "GBP", None, None, None, "S-B-005",   300),
]


def seed_all(con: duckdb.DuckDBPyConnection) -> None:
    """Insère toutes les données de test : master data, satellites et saisie brute."""
    con.executemany("INSERT INTO dim_scenario VALUES (?, ?, ?, ?)", SCENARIOS)
    con.executemany("INSERT INTO dim_entity VALUES (?, ?, ?, ?, ?)", ENTITIES)
    con.executemany("INSERT INTO dim_period VALUES (?, ?, ?, ?, ?, ?)", PERIODS)
    con.executemany("INSERT INTO dim_account VALUES (?, ?, ?, ?, ?)", ACCOUNTS)
    con.executemany("INSERT INTO dim_flow VALUES (?, ?, ?, ?)", FLOWS)
    con.executemany("INSERT INTO dim_currency VALUES (?, ?, ?)", CURRENCIES)

    con.executemany(
        "INSERT INTO sat_perimeter VALUES (?, ?, ?, ?, ?, ?, ?, ?)", PERIMETER
    )
    con.executemany(
        "INSERT INTO sat_exchange_rate (currency_source, period, taux_close, taux_moyen) "
        "VALUES (?, ?, ?, ?)",
        RATES,
    )

    con.executemany(
        "INSERT INTO stg_entry VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", _RAW
    )
