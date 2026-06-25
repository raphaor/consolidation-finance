# Plan d'action — Codes renommables via clés techniques (option B1)

> Statut : **en cours** (branche `feat/renommage-codes`).
> Décision : **option B1** — chaque objet gagne un `id` technique immuable ;
> le `code` devient un libellé mutable. Argumentaire A vs B en §2 ; migration
> in-place en §7.

## 0. Reprise rapide (dernière session : 2026-06-25)

**Point de départ d'une prochaine session.** Donner : « Reprends le chantier
codes-renommables, branche `feat/renommage-codes`, voir
`docs/PLAN_RENOMMAGE_CODES.md` §0 ».

### Où on en est
- **Le renommage fonctionne de bout en bout** (validé en UI : `variant` renommé
  en `V1`). C'est la preuve de l'approche : `POST /api/md/{table}/rename`, gardé
  par une vérification de sûreté, + bouton « Renommer » dans Master Data.
- Fait : **étapes 0, 1, 2** (golden / `id` partout / résolution code↔id),
  **étape 3 partielle** (FK `dim_consolidation` flippées : `variant`, `phase`,
  `perimeter_set`, `rate_set`), **étape 7** (renommage, livré en avance).
  **`rate_set` est désormais une dimension entièrement flippée** (renommable de
  bout en bout) : `sat_exchange_rate.rate_set` basculée en `id` (PK →
  reconstruction in-place), `convert.rs` joint sur l'id (résolu une fois dans la
  CTE `params`), seed/bench + import CSV + loader résolvent code→id. 2ᵉ dimension
  renommable après `variant`.
  **`perimeter_set` également** (3ᵉ) : `sat_perimeter.perimeter_set` en `id`
  (reconstruction). Stratégie différente — `ConvertParams.perimeter_set` passe en
  `i64` (au lieu de code) car ses seuls consommateurs sont les jointures
  `sat_perimeter` : aucun changement SQL dans `aggregate`/`consolidate`/`a_nouveau`,
  et `validate.rs`/`rules.rs` se **simplifient** (un JOIN `dim_perimeter_set` en
  moins, comparaison id=id directe).
- Robustesse : migration in-place par **reconstruction de table**, **import
  B1-aware**, **`CHECKPOINT`** anti-corruption WAL (cf. §7 + §10). Couverture
  **loader** ajoutée (`tests/loader.rs`) : le fresh-init serveur (`load_all`)
  n'était pas testé jusqu'ici — sa résolution code→id l'est désormais.
- État : **129 tests unit + 38 intégration verts, golden inchangé**.

### Modules clés (où regarder)
- `src/surrogate.rs` — `ensure_ids` (id sur chaque dim) +
  `migrate_consolidation_fk_to_id` + `migrate_sat_exchange_rate_fk_to_id` +
  `migrate_sat_perimeter_fk_to_id` (reconstructions in-place). Le **registre
  `SURROGATE_DIMS`**.
- `src/resolve.rs` — résolution code↔id (unitaire + cartes batch).
- `src/references.rs` — `Reference.target_display_column` + constructeur `ri()`
  (FK « id en stockage, code en contrat »). Patron de tout flip.
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
- **Rôle 3 (codes dans le JSON)** : non traité. La garde de `rename_code`
  s'appuie sur le **graphe de références uniquement** — pas encore de scan des
  codes dans `dim_rule.definition` / `dim_coefficient.expression` / indicateurs /
  `app_config`. À faire **avant** de rendre renommables les dimensions citées
  dans ces contenus (ex. `methode` est dans le scope d'une règle seedée).

### Prochaine étape (au choix de l'utilisateur)
Carte de coût des **dimensions de config** (cf. §8.3) — `rate_set` et
`perimeter_set` sont **faites** :
- **`ruleset`** : la plus propre des restantes (2 FK : `dim_consolidation.
  ruleset_code` + `dim_ruleset_item.ruleset_code` ; pas de rôle-3, pas de
  traversée native, pas de PK composite sur satellite).
- **`flow_scheme`** : vue `v_flow_behavior` + défauts en dur `RESULTAT`/`BILAN`
  + `sat_flow_scheme_item.scheme` (PK composite).
- **`sous_classe`** : `SENS_CASE` (server.rs, non testé) + réf. native traversable.
- **`method`** : nécessite d'abord le **rôle 3** (scope de règle `methode='globale'`).
- **`phase` / `exercice` / devises** : sur `fact_entry` → **étape 4** (la grosse).

Recommandation : `ruleset` (enchaînement le plus court, même profil que
`rate_set`), puis `flow_scheme` / `sous_classe`. L'**étape 4 (fact_entry)** reste
le jalon majeur (rend entités/comptes renommables).

### Hors chantier (ne pas committer)
Travail parallèle non-committé sur l'ergonomie des règles : `web/src/App.css`,
`web/src/pages/RulesPage.tsx`. **Laisser tel quel.**

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
   - ⚠️ **À smoke-tester par l'utilisateur** : `GET /api/consolidations` et le
     dropdown PipelinePage (server.rs non couvert par `cargo test` ; un bug
     `variant`/String y a déjà été trouvé puis corrigé).
   - ⏭️ **FK restantes du lot consolidation** : `exercice`, `presentation_currency`,
     `perimeter_period`, `rate_period` (→ **réordonner** seed + loader : `dim_period`
     et `dim_currency` avant `dim_consolidation`), `ruleset_code` (→ résoudre dans
     le handler `run`).
   - ⏭️ **FK entity/account** : `dim_entity.{devise_fonctionnelle (convert.rs),
     entite_parent}`, `dim_account.{sous_classe (SENS_CASE), flow_scheme (vue
     v_flow_behavior + défauts RESULTAT/BILAN)}`. Spécificité : ce sont des
     **références natives traversables** (`NATIVE_MASTER_REFS`/`map_ref`) → décider
     entre rendre la traversée id-aware ou retirer ces FK de la traversée.
4. **Basculer `fact_entry` en ids** : réécrire la frontière étape A + jointures
   pipeline + reports. *La grosse étape.*
5. **Nommer les objets dynamiques par id** (§4.3) → rôle 2 réglé.
6. **Basculer les JSON en ids** via l'éditeur (§6) → rôle 3 réglé.
7. ✅ **Endpoint `rename` + UI** (livré en avance) : `POST /api/md/{table}/rename`
   (`masterdata::rename_code`) + bouton « Renommer » dans MasterDataPage. **Gardé** :
   refuse si une référence cible encore le code (liste les blocages). Effectif dès
   qu'une dimension est entièrement flippée — aujourd'hui **`variant`**,
   **`rate_set`** et **`perimeter_set`**. S'étend
   automatiquement aux suivantes. ⚠️ Garde basée sur le graphe ; **pas encore** de
   scan des codes enfouis dans JSON/`app_config` (rôle 3) — à ajouter avant de
   rendre renommables les dimensions présentes dans ces contenus (ex. `methode`).
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
