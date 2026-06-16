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

## Tests de non-régression

Les tests d'intégration (`tests/pipeline.rs`) vérifient sur le jeu de seed
(groupe M/A/B) que le pipeline est cohérent. Chaque test ouvre une DuckDB **en
mémoire** (isolation totale). Ils contrôlent :

- les comptes produits par niveau (`corporate=16`, `reclassified=14`,
  `converted=19`, `consolidated=19`) ;
- les montants F99 attendus au niveau consolidated (`100=18 980.00`,
  `200=27 116.00`, `300=3 000.00`, `400=9 774.00`) ;
- l'identité de reconstruction via `validate` ;
- la présence/absence des écarts F80/F81 selon la devise (absents en devise
  fonctionnelle, présents en devise de présentation pour les entités non-EUR) ;
- la reproductibilité (second run après `DELETE FROM fact_entry`).

```bash
cargo test --release
```

## Benchmark de performance (gros volumes)

Le binaire `conso-bench` génère un jeu réaliste (60 entités, 200 comptes,
5 devises, ~10 % entrantes / sortantes) puis mesure chaque étape du pipeline
sur une DuckDB **fichier** (le cas réel). La génération se fait en SQL natif
(`range()` DuckDB), donc rien n'est matérialisé en Rust.

```bash
# petit run de validation (100k lignes)
cargo run --release --bin conso-bench -- --rows 100000

# run cible (1M+ lignes)
cargo run --release --bin conso-bench -- --rows 1000000
```

Options : `--rows <N>` (défaut 1 000 000), `--db <path>`
(défaut `$TEMP/conso_bench.duckdb`). Exemple de sortie attendue :

```
▶ stg_entry généré : 1000000 lignes en 1613 ms (620 k lignes/s généré)

═══════════════════════════════════════════════════════════════
  RAPPORT DE PERFORMANCE
═══════════════════════════════════════════════════════════════
  Étape (niveau)          Lignes    Durée (ms)   Débit (k/s)
  ──────────────────────────────────────────────────────────
  corporate                24000          93.8           256
  reclassified             22800          70.1           325
  converted                39600          93.9           422
  consolidated             39600         114.3           347
  ──────────────────────────────────────────────────────────
  TOTAL                  1000000         372.1          2688

  Temps total pipeline : 0.372 s
  Débit global         : 2688 k lignes stg/s  (2687806 lignes/s)

  Verdict F99 : ✓ OK — identité F99 + invariants tenus
═══════════════════════════════════════════════════════════════
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
