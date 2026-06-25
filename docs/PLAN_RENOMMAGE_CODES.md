# Plan d'action — Codes renommables via clés techniques (option B1)

> Statut : **plan validé, non implémenté** (branche `feat/renommage-codes`).
> Décision prise : **option B1** — chaque objet gagne un `id` technique immuable ;
> le `code` devient un libellé mutable. Voir §2 pour l'argumentaire A vs B.
> Contrainte forte : la bascule se fait par **migration in-place qui traduit les
> objets existants** (on ne repart **pas** du seed par défaut ; on préserve la
> base courante et les éditions utilisateur). Voir §7.

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
   qu'une dimension est entièrement flippée — aujourd'hui **`variant`**. S'étend
   automatiquement aux suivantes. ⚠️ Garde basée sur le graphe ; **pas encore** de
   scan des codes enfouis dans JSON/`app_config` (rôle 3) — à ajouter avant de
   rendre renommables les dimensions présentes dans ces contenus.
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
