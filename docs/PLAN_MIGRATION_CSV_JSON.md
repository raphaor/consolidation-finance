# Plan d'action — Migration du seed CSV vers JSON

> Statut : **T1 livrée** (branche `refactor/migration-csv-json`).
> Décision : remplacer le seed initial par CSV (`prototype/rust/data/*.csv`)
> par le mécanisme d'import/export JSON existant (`/api/export` +
> `/api/import/all`), supprimer tous les CSV vivants du repo, et neutraliser la
> recompilation Cargo parasite.

## Objectif (le « pourquoi »)

1. **Workflow utilisateur simplifié** : un reset de base doit pouvoir repartir
   d'un dump JSON (export d'une base précédente) plutôt que de zéro. C'est déjà
   le workflow constaté en pratique — le JSON est en plus **plus complet** que
   les CSV (couvre `dim_rule`, `dim_ruleset`, `dim_custom_dimension`,
   `dim_coefficient` que les CSV n'alimentent pas).
2. **Sortir les données du graphe de recompilation Cargo** : aujourd'hui, modifier
   un CSV déclenche une recompilation complète du crate (via la politique de
   repli du build script — voir §A). C'est pénible et sans valeur.
3. **Cohérence** : un seul mécanisme de persistance/restauration (JSON), pas deux
   (CSV pour le seed, JSON pour l'export).

**Critère de réussite** : (a) zéro fichier CSV vivant dans le repo ; (b) modifier
un fichier de données ne déclenche aucune recompilation Cargo ; (c) le cycle
`/api/export` → `/api/reset` → `/api/import/all` est sans perte, y compris sur
`dim_custom_reference`, `dim_characteristic*`, `dim_control*`, `dim_value_list`,
`dim_aggregate`, `dim_indicator` ; (d) tous les tests Rust + scripts Python
passent en mode JSON.

## Contexte technique (état des lieux avant chantier)

### A. Mécanisme exact de la recompilation parasite

Le crate `conso-engine` a un `build.rs` minimal qui ne fait que linker
`Rstrtmgr` sur Windows, **sans émettre de directive `cargo:rerun-if-changed=`**.
Cargo applique alors sa politique de repli documentée : *"rerun the build script
if any file in the package is changed"*. Tout changement dans `data/`,
`data_golden/`, `data/smoke/` relance donc `build.rs`, dont le fingerprint
change, ce qui recompile le crate.

**Il n'existe aucun `include_str!`/`include_bytes!` dans tout le crate** (vérifié
par recherche exhaustive). Les CSV sont lus à l'exécution par
`loader::load_all` (`src/loader.rs:378`) via `read_csv_auto` de DuckDB.

**Solution T1** : émettre `cargo:rerun-if-changed=build.rs` dans `build.rs`.
Cargo ne relancera alors le build script que si `build.rs` lui-même change — les
CSV deviennent invisibles pour le graphe de compilation.

### B. Qui consomme les CSV aujourd'hui

| Consommateur | Source | Fichiers |
|---|---|---|
| Boot serveur (base vierge) | `load_all + seed_demo_*` | `src/bin/server.rs:1640-1644` |
| `POST /api/reset` | `load_all + seed_demo_*` | `src/bin/server.rs:781-786` |
| Binaire `conso-engine` (CLI pipeline) | `load_all` | `src/main.rs:97` |
| Test Rust `tests/loader.rs` | `load_all` direct | `tests/loader.rs:23, 58` |
| Script Python `golden_test.py` | `conso-server` via `CONSO_CSV_DIR=data_golden` | `golden_test.py:431-451, 499` |
| Script Python `rules_test.py` | idem | `rules_test.py:483-484, 510` |
| Script Python `smoke_test.py` | idem via `CONSO_CSV_DIR=data` | `smoke_test.py:329, 355` |

**Tests Rust non concernés** : `tests/pipeline.rs`, `tests/rules.rs`,
`tests/a_nouveau.rs`, `tests/golden.rs` utilisent `seed_all` hardcodé Rust
(`src/seed.rs:804`), aucun CSV.

### C. Tables non couvertes par le JSON d'export (gap actuel)

La constante `TABLES` (`src/export.rs:40-67`) couvre 22 tables + 2 spéciales
(`dim_custom_dimension`, `dim_coefficient kind='user'`). Sont **absentes** :

| Table | Rôle | DDL |
|---|---|---|
| `dim_custom_reference` | registre des références directes (ex. `compte_parent`) | `schema.rs:436` |
| `dim_characteristic` | registre des caractéristiques N1 | `schema.rs:397` |
| `dim_characteristic_attribute` | attributs N2 typés | `schema.rs:409` |
| `car_<id>` (dynamique) | valeurs N1 par dimension hôte | créé par `characteristics::create_characteristic` |
| `dim_value_list` | registre des listes de valeurs | `schema.rs:455` |
| `lst_<id>` (dynamique) | valeurs des listes | créé par `value_lists::create_value_list` |
| `dim_control` | définitions de contrôles | `schema.rs:515` |
| `dim_control_set` | jeux de contrôles | `schema.rs:524` |
| `dim_control_set_item` | items de jeux | `schema.rs:532` |
| `dim_aggregate` | postes (sélections nommées) | `schema.rs:487` |
| `dim_indicator` | indicateurs/KPI | `schema.rs:501` |

**Conséquence** : aujourd'hui un cycle export → reset → import JSON perd déjà ces
tables. T2 corrige ce gap — c'est un prérequis sain avant toute suppression de
CSV.

### D. Inventaire des CSV vivants (à supprimer en T5)

| Répertoire | Rôle | Verdict |
|---|---|---|
| `prototype/rust/data/*.csv` (19 fichiers) | seed initial (référentiels stables) | Supprimer |
| `prototype/rust/data/account_parents.csv` | hiérarchie PCG (chargé hors `load_all` par `seed_demo_attributes`) | Supprimer |
| `prototype/rust/data/accounts_backup.csv` | **orphelin** (aucune référence code) | Supprimer |
| `prototype/rust/data/accounts_new.csv` | **orphelin** (aucune référence code) | Supprimer |
| `prototype/rust/data_golden/*.csv` | golden master pour `golden_test.py` / `rules_test.py` | Supprimer (T4 migre) |
| `prototype/rust/data/smoke/` | fixtures smoke tests (`conv_integ`, `interco`) | Supprimer (T4 migre) |
| `prototype/rust/dump_pipeline.csv` | sortie d'outil debug (`dump_pipeline.rs`), jamais relu | Supprimer + gitignore |

## Décisions prises

1. **Comportement du reset/boot sur base vierge** : `create_schema` seul si
   `CONSO_SEED_JSON` n'est pas défini ; `create_schema + import_bundle` sinon.
   Plus de `load_all + seed_demo_*` automatique.
2. **`seed_demo_*` au boot/reset** : **retirés**. Le JSON fait foi. Les fonctions
   restent dans `src/seed.rs` pour générer le JSON de référence et pour les
   tests hardcodés.
3. **Périmètre de suppression** : **zéro CSV vivant au final**. `data_golden/` et
   `data/smoke/` sont migrés vers des JSON dédiés dans
   `prototype/rust/tests/fixtures/`.
4. **Gap JSON** : **étendu** dans T2 pour couvrir toutes les tables persistantes
   (sauf `fact_entry` intentionnellement exclue comme dérivée).
5. **Localisation des JSON de référence** : `prototype/rust/tests/fixtures/`.
   Après T1, ces fichiers ne déclenchent plus de recompilation.

## Découpage en tâches

### T1 — Neutraliser la recompilation Cargo parasite

**Statut : LIVRÉE.**

- Ajout de `println!("cargo:rerun-if-changed=build.rs");` dans `build.rs`.
- Conserve le link `Rstrtmgr` sur Windows.
- Vérification : `cargo build` puis modification d'un CSV puis `cargo build` →
  "Finished" sans recompilation.

Indépendant du reste du chantier. Quick win à exploiter dès maintenant.

### T2 — Étendre l'export/import JSON aux tables manquantes

**Prérequis :** T1 (pour itérer sans recompilation pendant le dev).

**Fichier principal :** `src/export.rs`.

1. **Étendre `TABLES`** (`export.rs:40-67`) en respectant l'ordre des FK :
   - `dim_custom_reference` (registre références directes)
   - `dim_characteristic` puis `dim_characteristic_attribute`
   - `dim_value_list`
   - `dim_control`, `dim_control_set`, `dim_control_set_item`
   - `dim_aggregate`, `dim_indicator`
2. **Tables dynamiques** (`car_<id>`, `lst_<id>`) :
   - Dans `export_all`, itérer sur `dim_characteristic` et `dim_value_list` pour
     exporter chaque table sous une clé préfixée (ex. `"_car:42"`, `"_lst:7"`).
   - Dans `import_all`, reconstruire la table correspondante (si absente) via
     `characteristics::create_characteristic` / `value_lists::create_value_list`
     depuis son registre, avant l'insertion des valeurs.
3. **Bump `_meta.format`** : `conso-export-v2` → `conso-export-v3`
   (`export.rs:103-108`).
4. **Ordre d'import** dans `import_all` : `dim_custom_reference` **avant** les
   colonnes `r{id}` des tables maîtres ; registres caractéristiques/listes
   **avant** les tables `car_*`/`lst_*`.
5. **Test round-trip** dans `src/export.rs` (`#[cfg(test)]`) : `export → import
   → export` produit un paquet identique sur une base ayant `compte_parent`,
   une caractéristique user, une liste de valeurs, un contrôle user, un
   indicateur.

**Point de vigilance** : les tables `car_*`/`lst_*` survivent aujourd'hui au DROP
(`ALL_DROP` à `schema.rs:660-688` ne les contient pas). L'import JSON propre doit
les peupler explicitement — ne pas s'appuyer sur cet effet de bord.

### T3 — Mode boot/reset basé sur JSON

**Prérequis :** T2 (sinon perte des tables non couvertes).

**Fichiers :** `src/export.rs`, `src/bin/server.rs`, `src/main.rs`.

1. **Refactor** : extraire le cœur de `import_all` (`export.rs:215-303`) en
   ```rust
   pub fn import_bundle(
       con: &Connection,
       bundle: &Value,
       excluded: &[&str],
   ) -> Result<usize, AppError>
   ```
   Les handlers `import_all` et `import_preview` délèguent à cette fonction.
2. **Variable `CONSO_SEED_JSON`** : lue au boot serveur (`server.rs:1456`),
   exposée dans `AppState` à côté de `csv_dir`.
3. **Branche boot sur base vierge** (`server.rs:1640-1644`) :
   ```rust
   create_schema(&con).expect("✗ create_schema");
   if let Some(seed_json) = &state.seed_json {
       let bundle = fs::read_to_string(seed_json)...;
       import_bundle(&con, &serde_json::from_str(&bundle)?, &[])?;
   }
   ```
4. **`reset_handler`** (`server.rs:779-799`) : même logique.
5. **Retirer les appels** à `seed_demo_rules`, `seed_demo_controls`,
   `seed_demo_attributes` au boot/reset (choix "JSON-only"). Ces fonctions
   restent dans `src/seed.rs` pour la génération du JSON de référence (T4) et
   les tests hardcodés.
6. **`src/main.rs:97`** (binaire `conso-engine`) : aligner.
7. **Doc `print_help`** (`server.rs:1791-1795`) : documenter `CONSO_SEED_JSON`.

**Transition critique** : ne pas merger T3 sans avoir T4 prêt — sinon le serveur
ne peut plus bootstrapper une base exploitable (le pipeline de clôture
automatique `server.rs:1650-1658` n'aurait rien à consolider).

### T4 — Migration des tests vers JSON

**Prérequis :** T3.

1. **Générer les JSON de référence** (manuellement, via serveur temporaire en
   mode CSV) :
   - `prototype/rust/tests/fixtures/seed.json` ← export d'une base initialisée
     depuis `data/` + `seed_demo_*` (inclut donc `dim_rule`, `dim_control*`,
     `dim_custom_reference`, caractéristiques démo, hiérarchie `compte_parent`).
   - `prototype/rust/tests/fixtures/seed_golden.json` ← export depuis
     `data_golden/`.
   - `prototype/rust/tests/fixtures/seed_smoke_interco.json` ← export depuis
     `data/smoke/interco/`.
   - `prototype/rust/tests/fixtures/seed_smoke_conv_integ.json` ← export depuis
     `data/smoke/conv_integ/`.
2. **Adapter `tests/loader.rs`** : remplacer `load_all(&con, &data_dir())` par
   ```rust
   let bundle = serde_json::from_str(&fs::read_to_string(
       PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/seed.json")
   )?)?;
   import_bundle(&con, &bundle, &[])?;
   ```
   Les 2 assertions actuelles (résolution `rate_set`/`perimeter_set` code→id)
   restent valables car le JSON contient ces colonnes en codes texte.
3. **Adapter les scripts Python** :
   - Remplacer `CONSO_CSV_DIR` + `CONSO_FORCE_RESEED` par
     `CONSO_SEED_JSON=tests/fixtures/seed_<scope>.json`.
   - Alternative : garder `POST /api/reset` (qui devient schéma-seul) puis
     `POST /api/import/all` avec le contenu du fichier JSON.
4. **Vérifier** que `tests/pipeline.rs`, `tests/rules.rs`, `tests/a_nouveau.rs`,
   `tests/golden.rs` n'ont pas besoin de modifications (n'utilisent aucun CSV).

### T5 — Suppression effective des CSV + doc

**Prérequis :** T4 (sinon on casse les tests et le boot).

**Supprimer :**
- `prototype/rust/data/*.csv` (19 fichiers + `account_parents.csv`).
- `prototype/rust/data/accounts_backup.csv`, `accounts_new.csv` (orphelins).
- `prototype/rust/data_golden/*.csv` (tout).
- `prototype/rust/data/smoke/` (avec son `README.md`).
- `prototype/rust/dump_pipeline.csv` + ajout au `.gitignore`.

**Conserver :**
- `prototype/rust/src/seed.rs` (utilisé par `tests/pipeline.rs` etc. via
  `seed_all`, et pour la regen du JSON de référence).

**Doc à mettre à jour :**
- `CLAUDE.md` : remplacer `CONSO_CSV_DIR` par `CONSO_SEED_JSON`.
- `AGENTS.md` : idem (snippet `Start-Process`).
- `prototype/rust/README.md` : mentionner `tests/fixtures/seed.json` + workflow
  de regen.
- `docs/TECHNIQUE.md` si CSV mentionnés.
- `docs/CAS_CONSO_TEST.md` : migrer le contenu utile de
  `data/smoke/README.md` (description des cas `conv_integ` vs `interco`).

## Vérification finale (post-T5)

- `cargo build --release` ✓
- `cargo test` ✓ (tous les tests Rust, dont le round-trip T2 et `loader.rs`
  migré).
- `npm run build` côté web (s'il y a des références UI au reset).
- Démarrage serveur avec `CONSO_SEED_JSON` → consolidation auto fonctionne.
- Cycle `POST /api/export` → `POST /api/reset` → `POST /api/import/all` sans
  perte (y compris caractéristiques user, contrôle user, `compte_parent`,
  indicateur).

## Risques résiduels identifiés

| Risque | Mitigation |
|---|---|
| Tables dynamiques `car_*`/`lst_*` mal restaurées à l'import | Test round-trip T2 ; vérifier l'ordre de création |
| `seed.json` diverge du code si `seed_demo_*` évolue | Procédure de regen documentée dans `tests/fixtures/README.md` |
| Perte du pipeline auto au boot sur base vide (sans JSON) | Documenter : base vide = pas de seed, comportement attendu |
| Scripts Python cassés si chemin JSON relatif cassé | Utiliser `Path(__file__).parent / "fixtures" / ...` |

## Suivi d'avancement

| Tâche | Statut | Commit | Date |
|---|---|---|---|
| T1 — `rerun-if-changed` dans `build.rs` | LIVRÉE | (à remplir) | 2026-06-28 |
| T2 — Étendre export JSON | à faire | — | — |
| T3 — Mode boot/reset JSON | à faire | — | — |
| T4 — Migration tests vers JSON | à faire | — | — |
| T5 — Suppression CSV + doc | à faire | — | — |

## 0. Reprise rapide (point de départ d'une prochaine session)

Donner : « Reprends le chantier migration CSV→JSON, branche
`refactor/migration-csv-json`, voir `docs/PLAN_MIGRATION_CSV_JSON.md` §0 ».

T1 est livré. Prochaine étape : **T2** (étendre `export.rs`). Vérifier avant de
démarrer :
- `cargo build` est propre.
- `git log --oneline -5` montre le commit T1.
