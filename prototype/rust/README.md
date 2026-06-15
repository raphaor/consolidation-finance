# Scaffold Rust — Moteur de consolidation financière

Portage Rust du prototype Python (`../python/`).

## Objectif

Valider que la stack **Rust + DuckDB** compile et s'exécute sur **ARM64**
(Raspberry Pi 5, aarch64), et préparer la structure de modules pour le portage
complet du moteur de consolidation.

## Stack

| Composant  | Version       | Rôle                                    |
|------------|---------------|-----------------------------------------|
| Rust       | 1.93.1        | Langage du moteur                       |
| duckdb-rs  | 1.10503.1     | Bindings Rust pour DuckDB (`bundled`)   |
| DuckDB     | 1.1.x (C++)   | Moteur analytique columnar embarqué     |

La feature `bundled` de `libduckdb-sys` compile DuckDB en C++ depuis les
sources — aucun paquet système requis, binaire 100 % autonome.

## Structure des modules

```
src/
├── lib.rs              — déclare les modules + ré-exports publics
├── main.rs             — point d'entrée (DDL + seed + pipeline + validation)
├── schema.rs           — DDL complet (CREATE TABLE) — miroir de conso/schema.py
├── seed.rs             — données de test — miroir de conso/seed.py
├── pipeline/
│   ├── mod.rs          — orchestration des 4 étapes + ConvertParams
│   ├── aggregate.rs    — Étape A : agrégation → level='corporate'
│   ├── reclassify.rs   — Étape B : reclassification de périmètre → 'reclassified'
│   ├── convert.rs      — Étape C : conversion multi-devises → 'converted'
│   └── consolidate.rs  — Étape D : consolidation (méthodes) → 'consolidated'
├── validate.rs         — vérification F99 = F00 + F01 + F20 + F80 + F81 + F98
└── report.rs           — bilan par flux, comparaison des niveaux
```

## Modèle de données

- **6 dimensions** : dim_scenario, dim_entity, dim_period, dim_account,
  dim_flow, dim_currency
- **2 satellites** : sat_perimeter (méthodes + variations de périmètre),
  sat_exchange_rate (taux de change)
- **1 table de faits** : fact_entry (écritures aux 4 niveaux de stockage)
- **1 table de staging** : stg_entry (saisie brute CSV)

## Pipeline 4 étapes

```
stg_entry ──A──▶ fact_entry[corporate]     (agrégation, devise fonctionnelle)
             ──B──▶ fact_entry[reclassified]  (reclassification périmètre)
             ──C──▶ fact_entry[converted]    (conversion + écarts F80/F81)
             ──D──▶ fact_entry[consolidated] (× pct_integration)
```

## Construction et exécution

```bash
# Compilation (compile DuckDB en C++ — prévoir ~5-10 min sur Pi 5)
cargo build --release

# Exécution
cargo run --release
```

## État du portage

| Module        | État       | Notes                                             |
|---------------|------------|---------------------------------------------------|
| schema.rs     | ✅ Complet | DDL complet, traduit du Python                    |
| seed.rs       | ✅ Complet | Données de test portées                           |
| pipeline/*    | ✅ Complet | SQL des 4 étapes porté + `params!` adaptés        |
| validate.rs   | ✅ Fonctionnel | f64 au lieu de Decimal (à migrer vers rust_decimal) |
| report.rs     | ✅ Fonctionnel | Restitutions portées                              |

### À faire pour le portage complet

1. **Précision décimale** : migrer `f64` → `rust_decimal::Decimal` dans
   `validate.rs` et `report.rs` (le prototype Python utilise `decimal.Decimal`).
2. **Persistance fichier** : `Connection::open("conso.duckdb")` au lieu de
   `open_in_memory()` pour la production.
3. **Lecture CSV** : utiliser la fonction `read_csv` native de DuckDB pour
   charger `stg_entry` depuis des fichiers.
4. **Tests** : ajouter des tests unitaires (`#[cfg(test)]`) par étape.
