# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Langue de travail : **français** (docs, termes métier, commits, commentaires de code). Conserver ce registre.
> `AGENTS.md` reste la référence opérationnelle (conventions, anti-blocage processus) ; les docs `docs/*.md` sont la **source de vérité fonctionnelle** (voir §Documentation).

## Vue d'ensemble

Outil de consolidation financière **multi-entités / multi-devises** par la **méthode des flux**. Le code réel vit dans deux endroits ; le reste est documentation ou héritage :

- `prototype/rust/` — **le moteur** (crate `conso-engine`) : logique métier en Rust + DuckDB embarqué. C'est ici qu'on développe.
- `web/` — frontend **React + Vite + TypeScript + TanStack Table** (npm).
- `prototype/python/` — prototype d'origine, **référence historique** dont le Rust est le portage. Ne pas y développer ; il sert à comparer la sémantique.
- `simulations/` — scripts exploratoires Python.

## Commandes

Toutes les commandes Rust se lancent **depuis `prototype/rust/`**, les commandes web depuis `web/`.

```bash
# --- Rust (prototype/rust/) ---
cargo build --release          # compile DuckDB en C++ via feature "bundled" — long au 1er build
cargo test --release           # tests d'intégration (tests/pipeline.rs, tests/rules.rs)
cargo test --release pipeline  # un seul fichier de test
cargo test --release --test rules nom_du_test   # un seul test
cargo run --release            # binaire conso-engine : schéma → CSV → pipeline → validation
cargo run --release --bin conso-bench -- --rows 1000000   # benchmark gros volumes
cargo run --release --bin conso-server                    # serveur API (voir anti-blocage ci-dessous)

# --- Web (web/) ---
npm run dev        # dev server Vite (proxy /api -> localhost:3000)
npm run build      # tsc -b && vite build  (= la vérification de types)
npm run lint       # eslint
```

Il n'y a **pas de workspace Cargo** : le `Cargo.toml` est dans `prototype/rust/`. La base DuckDB de dev est `prototype/rust/conso.duckdb` (committée — supprimable pour repartir de zéro).

### ⚠️ Lancer le serveur (anti-blocage, Windows)

Le tool shell attend la fermeture des pipes stdout/stderr ; un process qui garde stdout ouvert (serveur, `npm run dev`) **bloque indéfiniment** en avant-plan. Règles (détail dans `AGENTS.md`) :

- Commandes qui terminent (`cargo build/test/run`, `conso-bench`, `npm run build`) → avant-plan normal.
- `conso-server` / `npm run dev` → **toujours** en arrière-plan via PowerShell `Start-Process -PassThru -WindowStyle Hidden -RedirectStandardOutput <log>`, stocker le PID, poller `/api/health` avec `Invoke-RestMethod`, puis `Stop-Process -Id $pid -Force`.
- Les **subagents/workers ne lancent jamais le serveur** : eux se limitent à `cargo build` + `cargo test`. Les tests HTTP runtime sont réservés à l'utilisateur principal.

Variables d'env du serveur : `CONSO_PORT` (3000), `CONSO_DB_PATH` (`conso.duckdb`), `CONSO_CSV_DIR` (`data`), `CONSO_WEB_DIR` (`../../web/dist`), `CONSO_FORCE_RESEED=1` (rejoue DROP + import CSV au démarrage). Au démarrage, les CSV ne sont réimportés que si la base est vierge ; sinon la base existante est conservée (les éditions UI survivent). Pour repartir des CSV à chaud : `POST /api/reset`.

## Architecture du moteur

### Pipeline en 3 étapes (niveaux de stockage)

Tout passe par une seule table de faits `fact_entry`, dont la colonne `level` matérialise l'avancement. Chaque étape lit un niveau et produit le suivant — toute la logique est **du SQL déclaratif** (une passe SQL par règle métier), pas du calcul ligne à ligne en Rust. Les variations de périmètre (entrée F01, sortie F98) ne sont **plus** une étape native : elles sont repensées en **règles** (l'ancien niveau `reclassified` et l'étape `B` ont été supprimés ; les lettres `A/C/D` sont conservées pour la continuité de nommage).

```
stg_entry ──A. agrégation────▶ fact_entry[corporate]      (devise fonctionnelle)
          ──C. conversion────▶ fact_entry[converted]      (devise de présentation, écarts F80/F81)
          ──D. consolidation─▶ fact_entry[consolidated]   (× pct_integration selon méthode)
```

- Orchestration : `src/pipeline/mod.rs`. Les 3 étapes implémentent le trait `Step` ; `run_steps` les enchaîne et, **après chaque niveau** (corporate inclus) : carry à-nouveau (`a_nouveau.rs`) → injection des flux de staging par préfixe de nature (`staging.rs`) → reconstruction autoritaire des clôtures (`materialize_closures.rs`) → hook post-niveau (injection des règles au niveau ciblé).
- Un fichier par étape : `aggregate.rs`, `convert.rs`, `consolidate.rs` (+ `a_nouveau.rs`, `staging.rs`, `materialize_closures.rs`).
- `ConvertParams::load_params(con, consolidation_id)` hydrate les paramètres d'un run depuis `dim_consolidation` + `app_config` (devises de présentation/pivot, exercice N, rate_set, **taux d'ouverture porté par N** — plus de période N-1 dérivée). Pas de `Default` — un run dépend de la consolidation.

### Modèle de flux et clôtures (`docs/FLUX_CONSO.md`)

Chaque traitement génère des écritures taguées par un **code de flux** (`Flow`) : F00 ouverture, F01 entrée périmètre, F20 variation, F80/F81 écarts de conversion, F98 sortie périmètre, F99 clôture. La conversion applique taux clôture au bilan, taux moyen au résultat.

Une **clôture** est un flux auto-référentiel (`dim_flow.flux_de_report(C) = C`) reconstruit comme `C = Σ(X | flux_de_report(X) = C, X ≠ C)`. Aujourd'hui seule F99 l'est, mais la logique (`pipeline/materialize_closures.rs`) est **générique et pilotée par les données** (`dim_flow.flux_de_report`), jamais en dur sur "F99". `validate.rs` vérifie l'identité de reconstruction.

### Registre des dimensions (`src/dimensions.rs`)

Le moteur est **data-driven sur les dimensions**. Le registre central décrit les dimensions built-in (12) et les dimensions **custom** ajoutées par l'utilisateur via l'API. Trois catégories pilotent propagation / nullabilité / grain de clôture :

- `Fixed` — propagée, non pilotable, non nullable, dans le grain des clôtures.
- `Active` — propagée, pilotable, non nullable, dans le grain des clôtures.
- `Analytical` — propagée, pilotable, **nullable**, hors grain des clôtures (les dimensions custom sont toujours Analytical).

Beaucoup de logique (ordre des flux, scopes autorisés, whitelists SQL) dérive dynamiquement de ce registre / de `information_schema` plutôt que de constantes en dur.

### Moteur de règles (`src/rules.rs`) — exécuteur générique

C'est un **exécuteur générique**, PAS l'endroit où coder une règle métier (interco, participation…) en dur. Une règle est un JSON (`dim_rule.definition`) avec un `scope` (conditions sur `sat_perimeter`) et des `operations` (sélection à un niveau de `fact_entry` × coefficient × multiplicateur → écriture avec `destination` par dimension). Un *ruleset* (`dim_ruleset` + `dim_ruleset_item` ordonnés) enchaîne plusieurs règles ; `run_ruleset` l'exécute. La reconstruction des clôtures après chaque règle est **désactivée par défaut** (flag `RECONSTRUCT_CLOSURES_AFTER_RULE = false`, 2026-06-21 : le F99 relève de la transition de niveau, pas d'une reconstruction post-règle ; les règles gèrent F99 flux à flux).

**JSON schema** (spec complète : `docs/REGLES_CONSO.md` §4) :
- `SelectionCond` : `dim` + `op` + `val`, avec traversée optionnelle `via` (caractéristique N1) ou `ref` (référence directe patron B), mutuellement exclusives — filtre indirect par attribut du membre.
- `Destination` : 5 modes par dimension pilotable — `inherit` / `override` / `null` / `map` (caractéristique N1→N2) / `map_ref` (référence directe). `map` et `map_ref` exigent `target_dimension = dim` écrit (compatibilité de type) et font INNER JOIN (les membres sans valeur de référence ou non classés ne génèrent rien).

**Modules connexes** :
- `characteristics.rs` — caractéristiques N1 (regroupement, table `car_<code>`) + attributs N2 typés. Registres `dim_characteristic` / `dim_characteristic_attribute` (survivent au reset). Routes `/api/meta/characteristics*`. Consommé par `map` et sélection `via`.
- `custom_references.rs` — références directes (patron B, colonne sur master data pointant vers une dimension, y compris elle-même : hiérarchie `compte_parent`). Registre `dim_custom_reference` (survit au reset). Routes `/api/meta/references-custom*`. Consommé par `map_ref` et sélection `ref`.
- `references.rs` — graphe central des références : `REFERENCES` (const statique) + `dynamic_references` (caractéristiques + patron B fusionnés) = `all_references`. Source unique de validation à l'écriture + dropdowns UI via `/api/meta/references`.

**Sécurité SQL** : les identifiants (noms de colonnes/dimensions, niveaux, `via`/`attr`/`ref`) sont validés contre des whitelists dérivées du registre / `information_schema` ; les valeurs passent par des `?` paramétrés. Ne jamais interpoler un identifiant venant du JSON utilisateur.

### Serveur (`src/bin/server.rs`)

Axum, état partagé `Arc<Mutex<Connection>>` (`src/state.rs`). Sert l'API JSON et, si `CONSO_WEB_DIR` existe, le frontend statique. Routes : `/api/health`, `/api/levels`, `/api/bilan`, `/api/compte-resultat`, `/api/entries`, `/api/consolidations`, `POST /api/run` (applique le ruleset porté par `dim_consolidation.ruleset_code`, intercalé par niveau — il n'y a **pas** de route `/api/rules/run` standalone), `POST /api/reset`, CRUD règles/rulesets (`/api/rules`, `/api/rulesets`), + CRUD master data / dimensions / import (`masterdata.rs`, `dimensions.rs`, `import.rs`).

### Frontend (`web/src/`)

Pages dans `pages/` (Import, MasterData, Pipeline, Rapports, Ecritures, Rules), client API centralisé dans `api.ts`, types partagés dans `types.ts`. En dev, `/api` est proxifié vers `localhost:3000` (`vite.config.ts`).

## Règles métier — invariants à respecter

- **Ne pas inventer de règle de consolidation.** Tout traitement non listé comme **natif** dans `EXPRESSION_DE_BESOIN.md` §3.4 doit passer par l'éditeur de règles. Ne pas coder de logique interco/participation en dur dans le moteur.
- **Précision décimale** : les montants utilisent `rust_decimal::Decimal` (sérialisé en *nombre* JSON via la feature `serde-float`), jamais `f64`. La finance n'accepte pas le flottant binaire.
- **Perf = critère de validation.** Cible : large (50+ entités, millions de lignes). Préférer le SQL ensembliste DuckDB à toute matérialisation en Rust. Éviter l'architecture spéculative (sécurité, multi-format) tant que non listée comme objectif.
- Avant d'implémenter, vérifier `docs/QUESTIONS_OUVERTES.md` : toute question `BLOC` ou `TÔT` non tranchée doit être soumise à l'utilisateur d'abord.
- Style de commit observé : `<type>: <sujet court>` (`docs:`, `refactor:`, `feat:`).

## Documentation (source de vérité fonctionnelle)

- `EXPRESSION_DE_BESOIN.md` — doc principal (volontairement court).
- `docs/QUESTIONS_OUVERTES.md` — décisions à prendre (priorisées `BLOC`/`TÔT`/`POST`/`HORS`, ID `Qn`).
- `docs/MODELE_DONNEES.md` — sémantique des champs CSV, dimensions, satellites.
- `docs/FLUX_CONSO.md` — catalogue des flux F00–F99.
- `docs/REGLES_CONSO.md` — spécification de l'éditeur de règles.
- `docs/TECHNIQUE.md` — architecture et justifications de la stack.
- `docs/README.md` — index de la doc (vivant vs archive).
- `docs/archive/` — **historique non maintenu** : specs d'implémentation livrées (`specs-livrees/` : registry, propagation, règles, UI règles, scénario v2) et analyses/revues ponctuelles (`analyses/`). Référence de conception, pas spec courante.
