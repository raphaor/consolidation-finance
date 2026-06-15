"""Chargement des données depuis des fichiers CSV via ``read_csv_auto``.

Contrairement à :mod:`conso.seed` (données codées en dur), ce module lit les
liasses et master data depuis un répertoire de fichiers CSV. DuckDB sachant
lire le CSV nativement, on s'appuie sur ``read_csv_auto`` : aucun parsing Python,
le moteur columnar fait tout le travail — y compris l'inférence de types
(BOOLEAN, DATE, DECIMAL…) et la gestion des cellules vides comme NULL.

Un fichier manquant n'est pas fatal : on émet un avertissement et on passe
au suivant, de façon à pouvoir alimenter la base avec un sous-ensemble des
tables (utile en cours de prototypage).
"""

from __future__ import annotations

import sys
from pathlib import Path

import duckdb

# ─────────────────────────────────────────────────────────────────────────────
#  Correspondance fichier CSV → table cible.
#  L'ordre suit la dépendance logique : dimensions → satellites → staging.
#  Les en-têtes CSV doivent être exactement les noms de colonnes de la table,
#  dans le même ordre que le DDL de :mod:`conso.schema`.
# ─────────────────────────────────────────────────────────────────────────────

CSV_TO_TABLE: list[tuple[str, str]] = [
    ("scenarios.csv",  "dim_scenario"),
    ("entities.csv",   "dim_entity"),
    ("periods.csv",    "dim_period"),
    ("accounts.csv",   "dim_account"),
    ("flows.csv",      "dim_flow"),
    ("currencies.csv", "dim_currency"),
    ("perimeter.csv",  "sat_perimeter"),
    ("rates.csv",      "sat_exchange_rate"),
    ("entries.csv",    "stg_entry"),
]


def _warn(msg: str) -> None:
    """Affiche un avertissement sur stderr (sans perturber le rapport stdout)."""
    print(f"⚠  {msg}", file=sys.stderr)


def load_all(con: duckdb.DuckDBPyConnection, data_dir: str | Path) -> None:
    """Charge tous les fichiers CSV du répertoire vers les tables DuckDB.

    Pour chaque couple (fichier, table) de :data:`CSV_TO_TABLE` :
        - si le fichier n'existe pas, avertissement et passage au suivant ;
        - sinon, ``INSERT INTO <table> SELECT * FROM read_csv_auto(<chemin>)``.

    Les en-têtes du CSV doivent correspondre, en nombre et en ordre, aux
    colonnes de la table cible (le ``SELECT *`` est apparié positionnellement).
    """
    data_path = Path(data_dir)

    for filename, table in CSV_TO_TABLE:
        csv_path = data_path / filename
        if not csv_path.exists():
            _warn(f"{csv_path} introuvable — table {table} laissée vide.")
            continue

        n_before = con.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        # Le chemin est bindé en paramètre (pas d'injection, pas d'échappement
        # à la main) ; seul le nom de la table est interpolé, et il vient d'une
        # constante interne.
        con.execute(
            f"INSERT INTO {table} SELECT * FROM read_csv_auto(?)",
            [str(csv_path)],
        )
        n_after = con.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        print(f"   {filename:<16} → {table:<18} {n_after - n_before:>3} lignes")
