#!/usr/bin/env python3
"""Point d'entrée du prototype de consolidation financière par les flux.

Enchaîne : création du schéma → seed des données → pipeline 4 étapes →
validation → restitution.

Usage :
    /home/raph/cf-clone/.venv/bin/python run.py
"""

from __future__ import annotations

import sys
from pathlib import Path

import duckdb

# Permet l'exécution directe (python run.py) sans installation du package.
sys.path.insert(0, str(Path(__file__).resolve().parent))

from conso.schema import create_schema
from conso.seed import seed_all
from conso.pipeline import run_pipeline
from conso.report import (
    bilan_par_flux,
    compare_levels,
    print_level_counts,
    print_validation,
)


def main() -> int:
    # DuckDB en mémoire : base éphémère, idéale pour un prototype.
    con = duckdb.connect(":memory:")

    try:
        print("╔" + "═" * 86 + "╗")
        print("║" + "  PROTOTYPE — Moteur de consolidation financière par les flux (DuckDB)".center(86) + "║")
        print("╚" + "═" * 86 + "╝")

        # 1. Schéma + données de test
        print("\n▶ Création du schéma et chargement des données de test…")
        create_schema(con)
        seed_all(con)
        n_stg = con.execute("SELECT COUNT(*) FROM stg_entry").fetchone()[0]
        print(f"   {n_stg} écritures brutes chargées dans stg_entry.")

        # 2. Pipeline 4 étapes
        print("\n▶ Exécution du pipeline (A→B→C→D)…")
        counts = run_pipeline(
            con,
            presentation_currency="EUR",
            current_period="2024",
            prev_period="2023",
        )
        for level, n in counts.items():
            print(f"   étape → {level:<13} {n:>4} lignes produites")

        # 3. Volumes par niveau
        print_level_counts(con)

        # 4. Bilan par flux (niveau consolidated)
        bilan_par_flux(con, level="consolidated")

        # 5. Comparaison des 4 niveaux sur un compte multi-devise représentatif
        compare_levels(con, account="400_Resultat")
        compare_levels(con, account="100_Capital")

        # 6. Validation des identités
        ok = print_validation(con)

        print("\n" + "═" * 88)
        print("  Fin du prototype.")
        print("═" * 88)
        return 0 if ok else 1
    finally:
        con.close()


if __name__ == "__main__":
    raise SystemExit(main())
