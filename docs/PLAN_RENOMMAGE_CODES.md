# Plan d'action — Codes renommables via clés techniques (option B1)

> Statut : **en cours** (branche `feat/renommage-codes`).
> Décision : **option B1** — chaque objet gagne un `id` technique immuable ;
> le `code` devient un libellé mutable. Argumentaire A vs B en §2 ; migration
> in-place en §7.

## 0. Reprise rapide (dernière session : 2026-06-27)

**Point de départ d'une prochaine session.** Donner : « Reprends le chantier
codes-renommables, branche `feat/renommage-codes`, voir
`docs/PLAN_RENOMMAGE_CODES.md` §0 ».

### Où on en est (2026-06-27 — état final)

**Étapes 0–7 entièrement terminées en code. Smoke-tests runtime à valider.**

- **Étape 5 terminée (2026-06-27bis)** : tables `car_<code>` → `car_<id>` et
  `lst_<code>` → `lst_<id>`. Scope réduit : renommages de tables uniquement
  (colonnes attributs N2 `c<id>` + colonnes custom `x<id>` différés).
  143 tests, 0 échec.
  - `surrogate.rs` : 3 nouvelles migrations idempotentes
    (`ensure_characteristic_attribute_ids`, `migrate_characteristic_tables_to_id`,
    `migrate_value_list_tables_to_id`).
  - `schema.rs` : `ensure_ids` déplacé avant `reapply` (chicken-and-egg corrigé).
  - `characteristics.rs`, `value_lists.rs` : `value_table(id)` + helpers `id_of`,
    `vtable_for`. Tous les sites d'appel mis à jour.
  - `references.rs` : `dynamic_references` + `target_master` id-aware.
  - `rules.rs`, `indicators.rs` : JOINs `car_{via}` → `car_{id}`.
  - `masterdata.rs` : `sql_name = car_{id}` / `lst_{id}`, `api_name` = code inchangé.
  - `server.rs` : 3 appels de migration au startup.
- **Smoke-tests OK (2026-06-27)** : CRUD rulesets, renommage jeu de règles,
  pipeline, perimeter/method, consolidations — aucun bug runtime détecté.
- Fait : **étapes 0, 1, 2, 3, 4, 5, 6, 7** entièrement.

### Prochaine étape : **smoke-test étape 5** + optionnel : `via` dans JSON

**À valider par l'utilisateur (serveur runtime) :**
- Créer une caractéristique, vérifier `car_1` créé.
- Créer une liste de valeurs, vérifier `lst_1` créé.
- Renommer un code de dimension → pas de blocage sur `car_*`/`lst_*`.
- POST /api/reset → `car_1`/`lst_1` survivent, reapply OK.

**Différé (scope réduit étape 5) :**

**✅ TERMINÉ (2026-06-27bis)** — scope réduit (tables seulement) :
- `car_<code>` → `car_<id>` ✅
- `lst_<code>` → `lst_<id>` ✅
- Attributs N2 `c<id>` : **différé**
- Colonnes custom `x<id>` : **différé**
- Référence directe `r<id>` : **différé**

**Ce que ça débloque :**
- `via` dans les JSON de règles/postes migrables vers ids (étape 6 résiduelle)
- Création de dimensions custom (chantier gelé §11)

### Détail étape 6 (JSON → ids)

**`src/json_migration.rs`** (nouveau module) :
- `normalize_rule_definition(con, json)` — scope.val + selection[*].val (hors
  via/ref/attr) + destination.override.value → ids entiers.
- `normalize_aggregate_definition(con, json)` — selection[*].val → ids.
- `normalize_indicator_expression(con, expr)` — `[code]` → `[id]` via
  `formula::operands`. Cherche agrégat puis indicateur.
- `migrate_json_to_ids(con)` — migration idempotente au démarrage (après
  `migrate_fact_entry_to_ids`). Parcourt dim_rule, dim_aggregate, dim_indicator.

**`src/rules.rs`** — moteur dual-mode :
- `Destination.value` : `Option<String>` → `Option<JsonValue>`.
- `parse_destination` "override" : accepte String (code legacy) ou Number (id).
- `dest_expr` override : `Number` → liaison directe `BigInt` sans sous-requête ;
  `String` → sous-requête `code→id` (chemin legacy inchangé).
- `exec_operation` sélection : val entier → `e.<dim>` direct (INTEGER=INTEGER) ;
  val string → `(SELECT code FROM table WHERE id = e.<dim>)` (chemin legacy).
- `validate_definition` override : skip la vérif d'existence pour les `Number`.
- 2 tests mis à jour / ajoutés.

**`src/indicators.rs`** — résolution par id :
- `IndicatorResolver.resolve` : si nom parseable en `i64` → lookup par id dans
  `dim_aggregate` puis `dim_indicator`. Fonctions `load_aggregate_def_by_id` et
  `load_indicator_expr_by_id` ajoutées.
- `create_aggregate`, `update_aggregate` : appellent `normalize_aggregate_definition`.
- `create_indicator`, `update_indicator` : appellent `normalize_indicator_expression`.

**`src/bin/server.rs`** :
- Appel de `migrate_json_to_ids` au démarrage.
- `create_rule`, `update_rule` : appellent `normalize_rule_definition` après validation.

**Ce qui n'est pas encore migré** (hors scope étape 6) :
- `coefficient.type` (format `{"type": "code"}`) — priorité moindre.
- `via` (codes de caractéristiques, noms de tables `car_<code>`) — bloqué sur étape 5.
- `ref` (codes de références directes) — pas encore renommable.
- `app_config.pivot_currency` — toujours casté en garde (bloquant au rename devise).

### Dimensions renommables aujourd'hui
`variant`, `rate_set`, `perimeter_set`, `sous_classe`, `flow_scheme`, `method`,
**`rules` (dim_rule)**, **`rulesets` (dim_ruleset)** — sous réserve que les
JSON soient migrés (étape 6 garantit la migration au démarrage).

### ⚠️ Étape 4 — diagnostic réel (le « stale build » était une fausse piste)

L'hypothèse « artefact périmé » de la session précédente était **fausse** : le
binaire était bien à jour. Deux vrais bugs (2026-06-26) :

1. **`pipeline/staging.rs`** (`inject_by_prefix`) — `GROUP BY {col_list}` portait
   sur des noms nus ; DuckDB liait `phase` à la colonne brute `stg_entry.phase`
   (code) plutôt qu'à l'alias `_dphase.id AS phase`, laissant l'`id` ni groupé ni
   agrégé → `Binder Error: column "id" must appear in the GROUP BY`. Le panic
   pointait `indicators.rs:926` (le `unwrap` de `run_pipeline`), pas la vraie
   source. **Même risque latent dans `aggregate.rs`** (`step_a`).
   **Fix (les deux)** : isoler la résolution code→id dans une **sous-requête**,
   puis `GROUP BY {col_list}` sur des noms propres et non-ambigus de la
   sous-requête. (Ni le GROUP BY positionnel `1,2,3` — rejeté par DuckDB en
   INSERT…SELECT — ni le GROUP BY sur alias — ambigu — ne marchaient.)
2. **`indicators.rs`** (`build_aggregate_sql`, branche sélection directe) —
   comparait le code (`'700'`) à `fact_entry.account`, désormais un id INTEGER
   (B1). **Fix** : pour une dim à master data, joindre la master
   (`LEFT JOIN dim_x imdd_x ON imdd_x.id = e.x`) et filtrer sur sa colonne code
   (`imdd_x.code = ?`), comme les branches `via`/`ref`/`attr`. Test
   `poste_direct_compile_et_filter` mis à jour (assertion `imdd_account.code`).
3. **`rules.rs`** (`exec_operation`, branche sélection **directe**) — même classe :
   l'opérande `e.<dim>` comparait un code à une colonne id. **Fix** : pour une dim
   id-typée, remonter l'id → code via sous-requête
   `(SELECT <code_col> FROM <table> WHERE id = e.<dim>)` (couvre `=`/`IN`/`IS NULL`).
   `dest_expr` (override/map/map_ref) était déjà id-aware. NB : sous B1 une valeur
   d'`override` doit exister dans la master data (FK NOT NULL) — les natures
   synthétiques des tests sont désormais seedées.

### ⚠️ Étape 4 — diagnostic réel (le « stale build » était une fausse piste)

L'hypothèse « artefact périmé » de la session précédente était **fausse** : le
binaire était bien à jour. Deux vrais bugs (2026-06-26) :

1. **`pipeline/staging.rs`** (`inject_by_prefix`) — `GROUP BY {col_list}` portait
   sur des noms nus ; DuckDB liait `phase` à la colonne brute `stg_entry.phase`
   (code) plutôt qu'à l'alias `_dphase.id AS phase`, laissant l'`id` ni groupé ni
   agrégé → `Binder Error: column "id" must appear in the GROUP BY`. Le panic
   pointait `indicators.rs:926` (le `unwrap` de `run_pipeline`), pas la vraie
   source. **Même risque latent dans `aggregate.rs`** (`step_a`).
   **Fix (les deux)** : isoler la résolution code→id dans une **sous-requête**,
   puis `GROUP BY {col_list}` sur des noms propres et non-ambigus de la
   sous-requête. (Ni le GROUP BY positionnel `1,2,3` — rejeté par DuckDB en
   INSERT…SELECT — ni le GROUP BY sur alias — ambigu — ne marchaient.)
2. **`indicators.rs`** (`build_aggregate_sql`, branche sélection directe) —
   comparait le code (`'700'`) à `fact_entry.account`, désormais un id INTEGER
   (B1). **Fix** : pour une dim à master data, joindre la master
   (`LEFT JOIN dim_x imdd_x ON imdd_x.id = e.x`) et filtrer sur sa colonne code
   (`imdd_x.code = ?`), comme les branches `via`/`ref`/`attr`. Test
   `poste_direct_compile_et_filter` mis à jour (assertion `imdd_account.code`).
3. **`rules.rs`** (`exec_operation`, branche sélection **directe**) — même classe :
   l'opérande `e.<dim>` comparait un code à une colonne id. **Fix** : pour une dim
   id-typée, remonter l'id → code via sous-requête
   `(SELECT <code_col> FROM <table> WHERE id = e.<dim>)` (couvre `=`/`IN`/`IS NULL`).
   `dest_expr` (override/map/map_ref) était déjà id-aware. NB : sous B1 une valeur
   d'`override` doit exister dans la master data (FK NOT NULL) — les natures
   synthétiques des tests sont désormais seedées.

**Adaptations des tests d'intégration (fact_entry en ids)** : helpers résolvant
code→id (`a_nouveau::amt`, `pipeline::{flow_sum,flow_rows,amount_for,…}`, INSERTs
directs via sous-requêtes) ; `rules.rs` : vue **`vfe`** code-aware ciblée par
`ssum`/`scount` + `put` résolvant les codes + seed des natures d'override.

**Reste : smoke-test serveur par l'utilisateur.**

### Modules clés (où regarder)
- `src/surrogate.rs` — `ensure_ids` (id sur chaque dim) +
  `migrate_consolidation_fk_to_id` (9 FK, dont lot 2ᵉ vague) +
  `migrate_consolidation_fk_to_id_v2` (bases intermédiaires) +
  `migrate_entity_fk_to_id` (devise_fonctionnelle + entite_parent, préserve les ids) +
  `migrate_sat_exchange_rate_fk_to_id` + `migrate_sat_perimeter_fk_to_id` +
  `migrate_sat_flow_scheme_item_scheme_to_id` (reconstructions in-place) +
  `migrate_account_sous_classe_to_id` + `migrate_account_flow_scheme_to_id` +
  `migrate_sat_perimeter_methode_to_id` (methode TEXT→id INTEGER, B1 6ᵉ dim)
  (add+update+drop+rename, hors PK). Le **registre `SURROGATE_DIMS`**.
- `src/resolve.rs` — résolution code↔id (unitaire + cartes batch).
- `src/references.rs` — `Reference.target_display_column` + constructeur `ri()`
  (FK « id en stockage, code en contrat »). Patron de tout flip. +
  **`ref_code_contract(host_dim, col)`** : détecte les FK natives `ri()` pour la
  traversée id-aware du moteur de règles (cf. §8 étape 3).
- `src/masterdata.rs` — traduction code↔id aux frontières (`write_db_value`,
  `translate_rows_out`, `validate_references`, `get_references`, `table_schema`)
  + **`rename_code`** (+ route `/rename`).
- `src/export.rs` — `import_db_value` (import B1-aware).

### Patron pour flipper une FK (réutilisable)
1. `references.rs` : passer la réf. en `ri(table, col, target, display, required)`.
2. `schema.rs` : colonne `TEXT → INTEGER`.
3. `seed.rs` / `bench.rs` / tests : `(SELECT id FROM <target> WHERE code = ?)`.
4. Loader : automatique (build_insert_sql lit `ri()`).
5. **Lecteurs internes** : résoudre id→code (ou joindre sur id) — pipeline,
   reports, `validate.rs`, `rules.rs`, et **`server.rs`** (⚠ voir gotchas).
   - **Cas particulier — FK native traversable** (`sous_classe`, `flow_scheme`…) :
     la traversée de règles (`ref` / `map_ref`) lit la colonne hôte. Si elle est
     `ri()` (id), utiliser `ref_code_contract` + le mécanisme id-aware de `rules.rs`
     (JOIN cible sur id, lecture du code). Cf. §8 étape 3 + commit `f846d0b`.
6. **Migration in-place** : reconstruction si la colonne est dans une PK/UNIQUE
   (cf. `migrate_consolidation_fk_to_id`), sinon add temp + update + drop + rename.
7. `cargo test` (golden = filet) **+ smoke-test serveur par l'utilisateur**.

### Gotchas (durement appris)
- **`server.rs` n'est PAS couvert par `cargo test`** : chaque flip touchant un
  endpoint HTTP (`list_consolidations`, handler `run`, `/api/bilan`,
  `/api/entries`) doit être **smoke-testé par l'utilisateur**. Deux bugs runtime
  y ont échappé aux tests verts.
- **WAL DuckDB** : le DDL de migration (`ALTER … SET DEFAULT nextval`, CREATE/
  RENAME) n'est pas toujours rejouable → un arrêt non propre rend la base
  illisible. Toujours `CHECKPOINT` après migration/init/import/reset. Récup d'une
  base cassée : supprimer le `.duckdb.wal`.
- **`ALTER … SET DATA TYPE … USING (sous-requête)` est interdit** par DuckDB →
  migration par reconstruction de table.
- **Colonnes `ri()` dans une table importée par CSV** (`sat_exchange_rate` = 1ʳᵉ) :
  le loader (`build_insert_sql`) traduit code→id automatiquement, mais
  `import.rs::import_rates` (INSERT direct hors loader) et
  `validate_csv_references` doivent être rendus ri()-aware à la main. Le validateur
  compare le CSV à la **colonne de contrat** (`target_display_column` = code),
  pas à la colonne de stockage (`id`) — sinon tout est marqué invalide.
- **Colonne-clé `INTEGER` captée comme « mesure »** : `coefficients::perimeter_fields`
  whiteliste les colonnes numériques de `sat_perimeter`. Sous B1, `perimeter_set`
  est `INTEGER` → il serait capté comme faux opérande de coefficient. **Fix** :
  exclure les colonnes listées dans le graphe `references_for("sat_perimeter")`.
  Piège générique : toute colonne FK passant `TEXT→INTEGER` peut être avalée par
  un filtre « colonnes numériques » data-driven.
- **Rôle 3 (codes dans le JSON)** : ✅ garde livrée — `masterdata::scan_json_blockers`
  scanne `dim_rule.definition` (scope/selection/destination/coeff/via),
  `dim_aggregate.definition`, `dim_indicator.expression`, `app_config.pivot_currency`
  avant tout renommage. C'est une **garde** (bloque le rename si le code est présent)
  pas encore une **migration** (les JSON stockent encore des codes — étape 6, future).
  `rules.rs` : `scope_effective_val` résout généricament les codes de scope vers ids
  pour toute colonne `ri()` de `sat_perimeter` (mécanique générique, non hardcodée).
- **⚠ Smoke-test en attente** : CRUD `/api/md/perimeter` (methode affiché en code),
  renommage d'une méthode non citée dans les règles, `GET /api/consolidations`.

### Prochaine étape (au choix de l'utilisateur)
**Étapes 0–4, 6, 7 terminées + toutes les FK dim→dim flippées (`ruleset` inclus).**
Reste le **smoke-test serveur** par l'utilisateur (server.rs non couvert par
`cargo test`) pour la FK `ruleset` : CRUD `/api/rulesets`, renommer un jeu de règles,
lancer le pipeline avec un ruleset.
Ensuite : **étape 5** (objets dynamiques nommés par id : `car_<id>`, `lst_<id>`,
rôle 2) — débloque la création de dimensions (chantier gelé §11).

### Hors chantier (ne pas committer)
Travail parallèle sur le frontend (`web/`) — ergonomie des règles, formules,
indicateurs. Évolue en continu côté utilisateur. **Le staging du chantier exclut
toujours `web/`** (on ne committe que `docs/` + `prototype/rust/`).

## 1. Le problème

Presque toutes les master data utilisent leur `code` (ou `code_iso`) comme **clé
primaire textuelle**, et tous les liens du modèle pointent vers cette clé par
**valeur** (pas de FK dures DuckDB, donc aucun `ON UPDATE CASCADE`). Donc :

- l'UI verrouille le champ `code` en édition (`MasterDataPage.tsx` :
  `locked = isEdit && col.pk`) ;
- même déverrouillé, un `UPDATE … SET code = …` laisserait **orphelines** toutes
  les lignes référençant l'ancien code.

`dim_consolidation` a déjà migré vers une PK technique `id` + clé naturelle
`UNIQUE` (`schema.rs`) : sous B1 ce patron devient **la règle**, plus l'exception.

## 2. Décision : pourquoi B1 plutôt que A

Deux stratégies ont été pesées :

- **A — renommage en cascade** : le `code` reste PK ; une opération « renommer »
  propage le changement partout (graphe de références + DDL des objets dynamiques
  + réécriture des JSON). Ciblé, incrémental, mais le renommage reste une
  opération lourde et fragile (codes enfouis dans les JSON).
- **B1 — clés techniques** : `id` auto immuable comme PK, `code` mutable `UNIQUE`,
  **tout le physique référence l'`id`** (FK, colonnes de faits, JSON, noms
  d'objets dynamiques). Renommer = `UPDATE dim_x SET code=? WHERE id=?`, point.

B1 est retenu car, avec les contraintes acceptées :

- **les JSON sont des fichiers d'archivage** lisibles par l'application seulement
  (pas besoin de lisibilité humaine) → on peut y stocker des ids ;
- **les jointures de résolution code↔id pour l'affichage sont acceptées** ;
- objectif **scalable** assumé (prototype aujourd'hui, cible : 50+ entités,
  millions de lignes).

B1 rend les **trois rôles** d'un code gratuits (cf. §3), aligne le modèle sur le
standard dimensionnel (clé technique stable dans la table de faits), et — bonus
scalabilité — **les jointures entières sont plus rapides que sur texte** : le
pipeline en régime devient plus rapide. Le coût (jointures de résolution) est
concentré sur l'**affichage** et l'**import**, pas sur le calcul de consolidation.

L'option A est conservée pour mémoire en **annexe §9**.

## 3. Les trois rôles d'un code (ce que B1 doit couvrir)

| Rôle | Description | Traitement sous B1 |
|---|---|---|
| **1 — valeur FK / faits** | `code` cité comme valeur dans une colonne (FK dim→dim, colonnes de `fact_entry`/`stg_entry`, satellites) | l'`id` ne change jamais → renommage **sans effet** sur ces colonnes |
| **2 — nom d'objet SQL** | `code` insère dans un nom de table/colonne : `car_<code>`, `lst_<code>`, colonne de dimension custom / caractéristique / réf. directe | objets **nommés d'après l'`id`** (`car_<id>`…) → le nom ne dépend plus du code |
| **3 — code enfoui dans JSON / expression** | `dim_rule.definition`, `dim_coefficient.expression`, `dim_aggregate.definition`, `dim_indicator.expression`/`grain`, `app_config.pivot_currency` | les JSON stockent des **ids** (l'éditeur traduit code↔id) → renommage **sans effet** |

> Insight central : **on n'encode jamais le `code` mutable dans quoi que ce soit
> de physique.** Le `code` n'existe qu'en colonne `code` de sa dimension + à
> l'affichage (résolu par jointure).

## 4. Schéma cible (B1)

### 4.1 Dimensions
Chaque dimension : `id` (séquence dédiée) PK, `code` mutable `UNIQUE NOT NULL`,
FK en `_id`.

```sql
CREATE SEQUENCE seq_dim_entity START 1;
CREATE TABLE dim_entity (
    id                       INTEGER DEFAULT nextval('seq_dim_entity') PRIMARY KEY,
    code                     TEXT NOT NULL UNIQUE,   -- mutable
    libelle                  TEXT,
    devise_fonctionnelle_id  INTEGER,  -- FK -> dim_currency.id
    entite_parent_id         INTEGER,  -- FK -> dim_entity.id (auto-réf)
    statut                   TEXT
);
```

### 4.2 Table de faits et staging
`fact_entry` : colonnes dimensionnelles à master data → `_id` entiers
(`entity_id`, `account_id`, `flow_id`, `currency_id`, `nature_id`, `partner_id`,
`share_id`, `period_id`, `entry_period_id`, `phase_id` ; `consolidation_id` déjà
un id). `analysis`/`analysis2` (dims libres, sans master data) restent en texte.

**`stg_entry` reste en codes** : zone d'atterrissage brute des liasses CSV (qui
arrivent en codes). La **résolution code→id se fait à une seule frontière** :
l'étape A (`aggregate.rs`), staging → `fact_entry[corporate]`. Donc renommer un
code ne touche jamais la grosse table de faits.

### 4.3 Objets dynamiques nommés par id (rôle 2)
Les registres gagnent un `id` ; le nom physique en dérive et le registre mappe
`id ↔ code ↔ nom physique` :

| Objet | Avant | Après |
|---|---|---|
| Caractéristique | `car_<code>` + colonne `<code>` | `car_<id>` + colonne `c<id>` |
| Liste de valeurs | `lst_<code>` | `lst_<id>` |
| Dimension custom | colonne `<name>` | colonne `x<id>` |
| Référence directe | colonne `<column_name>` | colonne `r<id>` |

`reapply` / `apply_custom_columns` (ré-application post-reset) lisent alors des
ids, jamais des codes.

### 4.4 Codes réservés
Sous B1, le moteur ne référence plus les master data par code mais par id résolu
→ `RESULTAT`/`BILAN`, `F99`, coefficients natifs, devise pivot **redeviennent
renommables**. Restent immuables seulement les **enums** non-master-data
(`classe ∈ {bilan,resultat,flux}`, `taux_conversion`).

## 5. Couche de résolution code↔id (module central à écrire)

- **Écriture** (API, CSV, sauvegarde de règle) : codes entrants → ids. Helpers
  `resolve_id(con, dim, code)` **et version batch** `resolve_many` (jamais de
  requête par ligne à l'import).
- **Lecture** (reports, grilles, chargement de règle) : ids → codes/libellés par
  jointure.
- **Graphe `references.rs`** : les cibles passent de `(dim_x, "code")` à
  `(dim_x, "id")` ; `value_exists` devient id-based. La validation à l'écriture
  résout code→id puis vérifie l'existence de l'id. Dropdowns UI : la dimension
  renvoie `id+code+libellé`, affiche le code, soumet l'id.

## 6. Impacts par couche

- **Pipeline** : `aggregate.rs` = la seule frontière code→id. `convert.rs`,
  `consolidate.rs`, `a_nouveau.rs`, `staging.rs`, `materialize_closures.rs` :
  jointures `… ON dim_x.code = f.col` → `f.col_id = dim_x.id` (et regroupements
  sur ids, plus rapides). `v_flow_behavior`, pivot currency : résolus par id.
- **Reports** (`report.rs`, `bilan`, `compte-resultat`, `entries`) : `JOIN` des
  dimensions pour projeter `code`+`libellé`, regrouper sur id, afficher le code.
  C'est le coût de jointure permanent (accepté).
- **Règles / formules** : `selection.val`, `destination.value`,
  `Coefficient::Named`, réfs poste↔indicateur, `via`/`ref`/`attr` → **ids**.
  `level` reste un enum. L'éditeur UI traduit code↔id à l'ouverture/sauvegarde.
- **Import CSV** (`import.rs`, `loader.rs`) : reste en codes côté fichier ;
  résolution batch à l'ingestion vers staging (codes) puis vers faits (ids) à
  l'étape A.
- **Frontend** : grilles et dropdowns affichent les codes (résolus), soumettent
  des ids ; nouvelle action « Renommer le code » (modal), distincte de l'édition.

## 7. Migration in-place (préserver les objets existants)

**Exigence** : ne pas repartir du seed ; **traduire la base courante**
(`conso.duckdb` + éditions utilisateur). La migration est une routine de
démarrage **idempotente et versionnée** (jalon dans `app_config`, ex.
`schema_version`), sur le modèle des migrations idempotentes déjà présentes
(`coefficients::ensure_schema`, `custom_references::migrate_native`).

Algorithme de traduction, par dimension puis par référence :

1. **Allouer les ids** : `ALTER TABLE dim_x ADD COLUMN id …` + remplir via
   séquence, ajouter `UNIQUE(code)`. (À ce stade, l'ancien `code` PK coexiste.)
2. **Construire la table de correspondance** `code → id` par dimension (en
   mémoire ou table temporaire).
3. **Traduire les FK et les faits** : pour chaque colonne référençante, `ADD
   COLUMN <col>_id`, `UPDATE … SET <col>_id = (map du code)` par jointure sur la
   dimension cible, puis `DROP COLUMN <col>` (texte). Couvre dim→dim,
   `fact_entry`, satellites. `stg_entry` reste en codes (cf. §4.2).
4. **Renommer les objets dynamiques** : `car_<code>`→`car_<id>`,
   `lst_<code>`→`lst_<id>`, colonnes custom → `x<id>`/`c<id>`/`r<id>` ; mettre à
   jour les registres (`dim_characteristic`, `dim_value_list`,
   `dim_custom_dimension`, `dim_custom_reference`, `dim_characteristic_attribute`)
   avec les ids.
5. **Traduire les JSON** : parcourir chaque `dim_rule.definition`,
   `dim_coefficient.expression`, `dim_aggregate.definition`,
   `dim_indicator.expression`/`grain` et remplacer les codes par les ids via les
   maps — en **réutilisant les parsers** de `rules.rs` / `formula.rs` (jamais de
   regex naïve). Idem `app_config.pivot_currency` → `pivot_currency_id`.
6. **Marquer** `schema_version` pour rendre la migration non rejouable.

Tout en **une transaction** ; rollback complet sur erreur. Filet de sécurité :
les **tests golden** (§8) tournent avant/après pour garantir une sortie
consolidée identique.

## 8. Feuille de route incrémentale

Chaque étape est livrable, testée, et garde le pipeline **iso-résultat** (tests
golden : la sortie consolidée ne bouge pas d'un centime). Chaque étape *backfill*
depuis les données existantes (jamais de reseed).

0. ✅ **Tests golden** : `tests/golden.rs` fige la sortie consolidée du seed
   (projection métier `BUSINESS_SELECT`, invariante à la bascule code→id).
1. ✅ **Ajouter `id`** à chaque dimension, sans rien casser (`src/surrogate.rs`,
   `ensure_ids` ; ids non consommés ; FK/faits restent en codes). Non-breaking.
   Unicité par séquence (pas d'index : DuckDB bloque alors `DROP COLUMN`) ; la
   PK sur `id` viendra à l'étape 3/4.
2. ✅ **Couche de résolution** code↔id (`src/resolve.rs` : `resolve_id`/`code_of`
   + cartes batch) + tests.
3. **Basculer les FK dim→dim** vers l'`id`. ⬅️ *en cours — mécanisme générique
   livré, flips FK par FK*
   - ✅ **Mécanisme option A** (contrat externe = code, stockage = id) : champ
     `references::Reference::target_display_column` + constructeur `ri()` ;
     traduction code↔id aux frontières dans `masterdata` (`write_db_value`,
     `translate_rows_out`, validation, dropdowns, health) ; résolution à l'import
     (`loader`). Une FK flippée = `ri()` + colonne `TEXT→INTEGER` + résolution au
     seed/loader + (si lue par un consommateur interne) résolution là-bas.
    - ✅ **FK flippées** : `dim_consolidation.{variant, phase, perimeter_set,
      rate_set}`. Lecteurs résolus : `load_params` (JOIN, pipeline reste code-based),
      `list_consolidations`, `validate.rs`, `rules.rs` (interco/coefficients), helpers
      de test. Round-trip code↔id testé.
    - ✅ **Dimension `rate_set` entièrement flippée** : `sat_exchange_rate.rate_set`
      (PK → `migrate_sat_exchange_rate_fk_to_id`, reconstruction) + `convert.rs`
      (jointures sur id, résolu dans la CTE `params`) + import CSV (`import_rates`)
       + loader. → `rate_set` est **renommable** (2ᵉ dimension après `variant`).
     - ✅ **Dimension `perimeter_set` entièrement flippée** : `sat_perimeter.
       perimeter_set` (PK → `migrate_sat_perimeter_fk_to_id`, reconstruction).
       Variante stratégique : `ConvertParams.perimeter_set` basculé en `i64`
       (au lieu de code) — ses consommateurs (`aggregate`/`consolidate`/
       `a_nouveau`) joignent `sat_perimeter` directement, **sans modif SQL** ;
       `validate.rs`/`rules.rs` se simplifient (sous-requête id=id, un JOIN
       `dim_perimeter_set` en moins). → `perimeter_set` est **renommable** (3ᵉ).
       ⚠ Effet de bord : `coefficients::perimeter_fields` captait `perimeter_set`
       (devenu INTEGER) comme opérande — corrigé en excluant les colonnes-clés.
     - ✅ **Dimension `sous_classe` entièrement flippée** : `dim_account.sous_classe`
       en `id` (migration add+update+drop+rename, hors PK — préserve les colonnes
       custom runtime). → `sous_classe` est **renommable** (4ᵉ).
       **Mécanisme id-aware (réutilisable)** : 1ʳᵉ FK native traversable flippée.
       `references::ref_code_contract(host_dim, col)` détecte les FK `ri()` ;
       `rules.rs` ajoute un `JOIN <cible> ON cible.id = hôte.<col>` (alias
       `mdrt_<ref>` / `smdrt_<ref>`) et lit/écrit le **code** au lieu de la colonne
       id. Sites : sélection `ref` (JOIN + opérande), destination `map_ref`
       (JOIN + `dest_expr`). La validation reste sur la colonne code (`target_master`).
       Préalable Q44 : `SENS_CASE` retiré, `sens` user-driven sur `dim_sous_classe`.
     - ✅ **Dimension `flow_scheme` entièrement flippée** : Q45 (2026-06-26) — la vue
       `v_flow_behavior` perd son `COALESCE(…, CASE classe)` et devient un `LEFT JOIN`
       sur `a.flow_scheme` (option **(b)** : compte sans schéma toléré mais silencieusement
       exclu de la conversion/clôture). Flip des **deux** FK vers `dim_flow_scheme` :
       `dim_account.flow_scheme` (hors PK → `migrate_account_flow_scheme_to_id`,
       add+update+drop+rename) et `sat_flow_scheme_item.scheme` (PK composite →
       `migrate_sat_flow_scheme_item_scheme_to_id`, reconstruction). Seed/bench peuplent
       `flow_scheme` (bilan/flux → BILAN, resultat → RESULTAT) → golden **stable**.
       `flow_scheme` reste dans `NATIVE_MASTER_REFS` : traversée id-aware des règles
       via `ref_code_contract` (test `selection_via_flow_scheme_id_aware`). → `flow_scheme`
       est **renommable** (5ᵉ).
     - ⚠️ **À smoke-tester par l'utilisateur** : `GET /api/consolidations`, le dropdown
       PipelinePage, et les Master Data `accounts` / `flow_schemes` / `flow_scheme_items`
       (server.rs non couvert par `cargo test` ; un bug `variant`/String y a déjà été
       trouvé puis corrigé).
     - ✅ **FK restantes du lot consolidation** : `exercice`, `presentation_currency`,
       `perimeter_period`, `rate_period`, `ruleset_code` flippées en `id`.
       `dim_period` et `dim_currency` réordonnées avant `dim_consolidation` dans seed
       + loader. `migrate_consolidation_fk_to_id` étendu à 9 FK ;
       `migrate_consolidation_fk_to_id_v2` pour bases intermédiaires (v1 déjà jouée).
     - ✅ **FK `dim_entity`** : `devise_fonctionnelle` → `dim_currency.id` et
       `entite_parent` → `dim_entity.id` (auto-référence préservant les ids).
       Migration `migrate_entity_fk_to_id` (reconstruction + `id` préservé).
     - ✅ **Rôle 3 — garde JSON** : `scan_json_blockers` dans `masterdata.rs` scanne
       `dim_rule.definition` (scope, selection, destination, coefficient, via),
       `dim_aggregate.definition` (selection, via), `dim_indicator.expression` ([code]),
       `app_config.pivot_currency`. Intégré dans `rename_code` après la garde graphe.
       4 tests unitaires (method/account/aggregate/currency). Couvre le prérequis de `method`.
     - ✅ **`method`** (6ᵉ renommable) : `sat_perimeter.methode` → `dim_method.id`
       (`migrate_sat_perimeter_methode_to_id`, add+update+drop+rename).
       Scope des règles id-aware : `resolve_sat_perimeter_val_to_id` + `scope_effective_val`
       dans `rules.rs` (générique pour toute colonne ri() de sat_perimeter).
       `consolidate.rs` : `ON m.id = per.methode`. Garde rôle 3 = le seul frein
       résiduel (ex. `methode='globale'` dans un JSON de règle non encore migrée).
       **Après étape 6** : la migration JSON traduit les codes de scope (`methode`) en
       ids au démarrage → `method` est **pleinement renommable** après migration.
       ⚠️ À smoke-tester : CRUD perimeter, renommage `globale`, `GET /api/consolidations`.
     - ✅ **`ruleset`** (session 2026-06-27bis) : `dim_ruleset_item.ruleset_code`
       TEXT → INTEGER (`migrate_ruleset_item_fk_to_id`, PK composite → reconstruction).
       `references.rs` : `rq()` → `ri()`. Sous-requête id dans `rules.rs` +
       `server.rs` (5 handlers). Export/import B1-aware automatique. Tests verts.
       ⚠️ Smoke-test serveur en attente.
4. ✅ **Basculer `fact_entry` en ids** : frontière étape A (`aggregate.rs`) +
   `staging.rs` (résolution code→id en sous-requête pour un GROUP BY propre) +
   jointures pipeline + reports + lecteurs (`indicators.rs`, `rules.rs` sélection
   directe id→code). *La grosse étape.* Suite `cargo test` verte (golden inclus).
5. **Nommer les objets dynamiques par id** (§4.3) → rôle 2 réglé.
6. ✅ **Basculer les JSON en ids** → rôle 3 réglé (session 2026-06-27).
   Voir détail complet dans §0. Résumé : `json_migration.rs` (nouveau module),
   moteur dual-mode dans `rules.rs` + `indicators.rs`, migration idempotente
   au démarrage, normalisation à la sauvegarde.
   **Reste hors scope** : `coefficient.type`, `via` (bloqué étape 5), `ref`,
   `app_config.pivot_currency`.
7. ✅ **Endpoint `rename` + UI** (livré en avance) : `POST /api/md/{table}/rename`
   (`masterdata::rename_code`) + bouton « Renommer » dans MasterDataPage. **Gardé** :
   refuse si une référence cible encore le code (liste les blocages). Effectif dès
   qu'une dimension est entièrement flippée. **Session 2026-06-27** : `dim_rule` et
   `dim_ruleset` ajoutées dans `TABLES` → bouton « Renommer » opérationnel dans
   `RulesPage.tsx` pour les règles et jeux de règles. Cascade correcte :
   `dim_ruleset_item.rule_code` et `.ruleset_code` (`rq()` → mis à jour) ;
   `dim_consolidation.ruleset_code` (`ri()` → non affecté).
   Robustesse base : `CHECKPOINT` après migrations/import/reset/rename (sinon WAL
   DuckDB irrejouable au redémarrage). Import B1-aware (restaure un export en codes).
8. **Retirer les chemins code-based** résiduels + finaliser la migration
   in-place versionnée (§7).

## 9. Annexe — option A (rejetée, pour mémoire)
Garder le `code` en PK + opération « renommer en cascade » : `UPDATE` de toutes
les colonnes du graphe `references.rs`, `ALTER TABLE RENAME` des objets dynamiques
(rôle 2), et réécriture (ou blocage) des JSON (rôle 3). Plus simple à court terme
mais le renommage reste lourd et fragile, et ne prépare pas la scalabilité visée.

## 10. Risques & vigilance
- **Résolution batch obligatoire** à l'import et au report — jamais de requête
  par ligne, sinon la scalabilité visée est perdue.
- **Une seule source de vérité** pendant les étapes 3–4 : tant que `fact_entry`
  est en codes, l'`id` est « shadow » ; bascule franche, pas de double-écriture
  durable.
- **Traduction des JSON** (rôle 3) = principale source de bug silencieux : passer
  par les parsers, couvrir par tests.
- **Migration transactionnelle** : un état partiel corrompt l'intégrité.
- **`analysis`/`analysis2`** : dims libres → restent en texte (renommage sans
  objet) ; à adosser un jour à des listes de valeurs si besoin.
- **Tests golden** dès l'étape 0 comme filet sur toute la migration.

## 11. Chantier adjacent GELÉ — création de dimensions (ne pas démarrer avant B1)

> Note ajoutée le 2026-06-25 par le chantier UI (refonte Référentiel). **À lire
> avant de reprendre B1.**

**Demande utilisateur** : étendre la page « Dimensions » pour créer une dimension
custom qui, au choix —
- (a) **emprunte** les valeurs d'une autre dimension (comme `partner`/`share` →
  `entity`, ou un futur `devise_transaction` → `currency`) ;
- (b) a sa **propre table de valeurs**, administrée dans Master data ;
- (c) porte ses propres **attributs** (caractéristiques / emprunts), selon les
  principes de l'interface actuelle.

Aujourd'hui `dimensions::create_custom` ne crée qu'une dimension **libre**
(colonne `TEXT`, catégorie Analytical, **sans référence**).

**Décision : gelé tant que B1 est en déploiement.** Le risque d'interférence est
élevé — la feature touche précisément les modules que B1 fait basculer code→id :

| Brique de la feature | Module B1 percuté | Étape B1 |
|---|---|---|
| Dimension empruntée = nouvelle **référence** dans le graphe | `references.rs` (`ri()`, contrat code/stockage id) | étape 3 (en cours) |
| Traductions à l'écriture/lecture/validation/dropdowns | `masterdata.rs` (`write_db_value`, `translate_rows_out`…) | étape 3 |
| **Table propre** / **attributs** = objets dynamiques (`car_<code>`, `lst_<code>`, colonne `<name>`) | nommage par id (`car_<id>`, `x<id>`/`c<id>`/`r<id>`) | **étape 5** |
| Dimension empruntée = **axe de faits** référençant une autre dim (comme `partner`) | FK `fact_entry` code→id | **étape 4 (la grosse)** |
| Extension de `create_custom` / `delete_custom` | `dimensions.rs` | transverse |

Construire maintenant en **code-based** obligerait à tout refaire en **id-based**
ensuite, avec des conflits de fusion sur `references.rs` / `masterdata.rs` /
`dimensions.rs` — les fichiers les plus chauds de B1.

**Condition de déblocage** : reprendre cette feature **après** que B1 ait livré au
moins **étape 4 (fact_entry en ids)** et **étape 5 (objets dynamiques nommés par
id)**. La création s'appuiera alors directement sur les conventions id-based
(références via `ri()`, objets `*_<id>`), sans double travail.

**Déjà livré côté UI (sans risque B1, déjà committé sur cette branche)** : page
« Dimensions » (groupe Référentiel) listant les axes + colonne « Valeurs depuis »
(révèle l'emprunt `partner`/`share` → `entities`) ; création de dimensions
**libres uniquement** ; vues lecture seule `partner`/`share` dans Master data.
Commits `feat(web)` récents. C'est l'**étape UI** ; l'**étape moteur** (a/b/c
ci-dessus) est ce qui est gelé.
