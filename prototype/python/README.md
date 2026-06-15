# Prototype Python — Consolidation financière par les flux

Prototype du moteur de consolidation implémentant le pipeline complet à 4 étapes
sur **DuckDB embarqué**. C'est la maquette exécutable du modèle décrit dans
`docs/FLUX_CONSO.md` et `docs/MODELE_DONNEES.md`, pensée comme précurseur du
futur `crate engine` en Rust.

## Principes

- **Consolidation par les flux** : chaque traitement génère des écritures taguées
  par un code de flux (F00 ouverture, F20 variation, F80/F81 écarts de conversion,
  F01/F98 périmètre, F99 clôture). F99 n'est **jamais saisi** : c'est un solde
  **reconstruit** par l'identité `F99 = F00 + F01 + F20 + F80 + F81 + F98`.
- **4 niveaux de stockage** matérialisent l'état des données après chaque étape :
  `corporate` → `reclassified` → `converted` → `consolidated`.

## Pipeline (4 étapes)

| Étape | Opération | Entrée | Sortie |
|---|---|---|---|
| **A. Agrégation** | Cumul des écritures source par entité | `stg_entry` (saisie) | `corporate` (fonctionnel) |
| **B. Reclassification** | Variations de périmètre en fonctionnel : F00→F01 (entrée), collapse→F98 (sortie) | `corporate` | `reclassified` (fonctionnel) |
| **C. Conversion** | Taux de change + génération des écarts F80/F81 | `reclassified` | `converted` (EUR) |
| **D. Consolidation** | Méthodes globale / proportionnelle (équivalence hors MVP) | `converted` | `consolidated` (EUR) |

## Structure

```
conso/
├── schema.py    # DDL : dimensions, satellites, table de faits
├── seed.py      # Données de test (groupe multi-devise M / A / B)
├── pipeline.py  # Les 4 étapes A→B→C→D (SQL déclaratif)
├── validate.py  # Identité F99 = F00+F01+F20+F80+F81+F98
└── report.py    # Bilan par flux + comparaison des niveaux
run.py           # Point d'entrée
```

## Exécution

```bash
/home/raph/cf-clone/.venv/bin/python run.py
```

## Scénario de test

- **Mère M** : EUR, globale 100 %, périmètre continu.
- **Filiale A** : USD, globale 100 %, **entre** en N (F00→F01).
- **Filiale B** : GBP, globale 100 %, **sort** en N (F00+F20→F98).

Taux vers EUR — USD : close_n1=0.92, avg=0.95, close_n=0.90 ;
GBP : close_n1=1.15, avg=1.18, close_n=1.12.

## Notes de portage (Rust)

Chaque étape du pipeline est une fonction isolée qui exécute une passe SQL lisant
un niveau et écrivant le suivant. La logique métier (règles de reclassification,
choix du taux de conversion, calcul d'écart, application du % d'intégration) est
entièrement contenue dans le SQL — un portage en Rust via `duckdb-rs` consiste à
reproduire ces passes sans réécrire la logique dans un autre langage.
