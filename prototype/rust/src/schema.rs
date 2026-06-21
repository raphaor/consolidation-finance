//! Définition du schéma DuckDB : dimensions, tables satellites, fait.
//!
//! Miroir de `prototype/python/conso/schema.py`.
//! Modèle repris de `docs/MODELE_DONNEES.md` et `docs/FLUX_CONSO.md`.
//!
//! # Niveaux de stockage des écritures (colonne `level` de `fact_entry`)
//!
//! | level        | devisé         | étape de production         |
//! |-------------|----------------|-----------------------------|
//! | corporate    | fonctionnelle  | A. agrégation               |
//! | converted    | présentation   | C. conversion multi-devises |
//! | consolidated | présentation   | D. consolidation (méthodes) |
//!
//! Le niveau `reclassified` (ex-étape B, reclassification de périmètre) a été
//! supprimé : le périmètre passe par des règles au niveau corporate
//! (cf. docs/A_NOUVEAU.md §4). `fact_entry.level` n'accepte plus que 3 valeurs.
//!
//! Une table de staging `stg_entry` reçoit la saisie brute (liasses CSV).
//! L'étape A lit cette table et produit le niveau *corporate*.

// ─────────────────────────────────────────────────────────────────────────────
//  DDL — ordre : séquence, dimensions, satellites, staging, table de faits
//  Chaque constante contient un ordre SQL complète (CREATE TABLE / SEQUENCE).
// ─────────────────────────────────────────────────────────────────────────────

/// Séquence d'identifiants auto-incrémentés pour la table de faits.
pub const DDL_SEQ_ENTRY: &str = "CREATE SEQUENCE IF NOT EXISTS seq_entry START 1;";

// --- Config applicative (singleton d'instance) --------------------------------

/// 0. app_config : configuration de l'instance (clé/valeur).
///
/// Actuellement utilisée pour `pivot_currency` : la devise pivot unique de
/// l'instance, vers laquelle tous les taux de `sat_exchange_rate` convertissent.
/// Non exposée via masterdata CRUD (config système, pas une dimension éditable).
pub const DDL_APP_CONFIG: &str = "\
CREATE TABLE app_config (
    key   TEXT PRIMARY KEY,
    value TEXT
);";

// --- Nouvelles dimensions (référentiels pour dim_scenario) --------------------

/// 1a. dim_scenario_category : catégorie du scénario (REEL, BUDGET, PREVISION).
///
/// Remplace l'ancien champ libre `type` de `dim_scenario` : la catégorie devient
/// une véritable dimension référencée (cf. SPEC_SCENARIO_V2.md §4).
pub const DDL_DIM_SCENARIO_CATEGORY: &str = "\
CREATE TABLE dim_scenario_category (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 1b. dim_rate_set : jeux de taux (réels, budget…).
///
/// Un scénario référence un jeu de taux via `dim_scenario.rate_set`. Permet
/// d'avoir des taux différents (réels vs budget) pour un même cadre
/// (catégorie + période + devise). Cf. SPEC_SCENARIO_V2.md §2.
pub const DDL_DIM_RATE_SET: &str = "\
CREATE TABLE dim_rate_set (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 1d. dim_perimeter_set : jeux de périmètre (versions du périmètre de conso).
///
/// Symétrique de `dim_rate_set` : un scénario référence un jeu de périmètre via
/// `dim_scenario.perimeter_set`, et `sat_perimeter` est clé par
/// `(perimeter_set, entity, period)`. Permet de **réutiliser** un même périmètre
/// entre scénarios/variantes (cf. docs/QUESTIONS_OUVERTES.md Q35).
pub const DDL_DIM_PERIMETER_SET: &str = "\
CREATE TABLE dim_perimeter_set (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 1c. dim_variant : variante d'un même cadre (BASE, OPT1, PESSIMIST…).
///
/// Déclinaison du scénario avec des hypothèses différentes, sans changer le
/// cadre (catégorie + période + devise de présentation).
/// Cf. SPEC_SCENARIO_V2.md §3.
pub const DDL_DIM_VARIANT: &str = "\
CREATE TABLE dim_variant (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

// --- Dimensions (master data) -------------------------------------------------

/// 1. dim_scenario v2 : scénario de consolidation, objet composite.
///
/// Le scénario agrège toutes les références nécessaires à un run : catégorie,
/// période d'entrée, devise de présentation, variante, ruleset (nullable) et
/// jeu de taux. Le pivot, lui, est applicatif (`app_config.pivot_currency`).
/// `prev_period` n'est pas stocké : dérivé à l'exécution depuis `dim_period`.
/// Cf. SPEC_SCENARIO_V2.md §5.
pub const DDL_DIM_SCENARIO: &str = "\
CREATE TABLE dim_scenario (
    code                  TEXT PRIMARY KEY,
    libelle               TEXT,
    category              TEXT,   -- FK dim_scenario_category ('REEL', 'BUDGET'…)
    entry_period          TEXT,   -- FK dim_period ('2024')
    presentation_currency TEXT,   -- FK dim_currency ('EUR')
    variant               TEXT,   -- FK dim_variant ('BASE')
    ruleset_code          TEXT,   -- FK dim_ruleset (NULL = pas de règles)
    rate_set              TEXT,   -- FK dim_rate_set
    perimeter_set         TEXT,   -- FK dim_perimeter_set : jeu de périmètre du run (cf. Q35)
    statut                TEXT,   -- 'ouvert' / 'verrouillé'
    a_nouveau_scenario    TEXT    -- FK dim_scenario : conso N-1 figée dont on reporte l'ouverture (NULL = pas d'à-nouveau). Cf. docs/A_NOUVEAU.md §2.2
);";

/// 2. dim_entity : entité du groupe (hiérarchie, devise fonctionnelle).
pub const DDL_DIM_ENTITY: &str = "\
CREATE TABLE dim_entity (
    code                 TEXT PRIMARY KEY,
    libelle              TEXT,
    devise_fonctionnelle TEXT,   -- code ISO (EUR, USD, GBP…)
    entite_parent        TEXT,   -- code entité parente (hiérarchie de groupe)
    statut               TEXT
);";

/// 3. dim_period : période ou exercice comptable.
pub const DDL_DIM_PERIOD: &str = "\
CREATE TABLE dim_period (
    code       TEXT PRIMARY KEY,
    libelle    TEXT,
    type       TEXT,          -- mois / trimestre / année / exercice
    date_debut DATE,
    date_fin   DATE,
    statut     TEXT           -- clôturé / ouvert
);";

/// 4. dim_account : plan de compte du groupe (classe + sous-classe).
///
/// `sous_classe` référence `dim_sous_classe.code` (pas de FK dure en DuckDB pour
/// le proto — contrainte uniquement sémantique).
///
/// Le regroupement par nature (ex. `capitaux_propres`) et la hiérarchie
/// d'agrégation (compte parent) ne sont **plus codés en dur** ici : ils se
/// recréent à l'exécution, respectivement comme **caractéristique** (N1, cf.
/// [`crate::characteristics`]) et comme **référence directe** (patron B, cf.
/// [`crate::custom_references`]).
pub const DDL_DIM_ACCOUNT: &str = "\
CREATE TABLE dim_account (
    code               TEXT PRIMARY KEY,
    libelle            TEXT,
    classe             TEXT CHECK (classe IN ('bilan', 'resultat', 'flux')),
    sous_classe        TEXT,           -- référence dim_sous_classe.code
    flow_scheme        TEXT            -- référence dim_flow_scheme.code ; NULL = défaut dérivé de la classe (cf. pipeline::convert / docs/QUESTIONS_OUVERTES.md Q32)
);";

/// 4c. dim_flow_scheme : schémas d'articulation des flux (catalogue).
///
/// Un schéma de flux décrit **l'articulation complète des flux** pour les comptes
/// qui le portent : taux de conversion, flux d'écart, flux de report de clôture,
/// flux d'à-nouveau — par flux (cf. `sat_flow_scheme_item`). Permet d'avoir des
/// comptes de **résultat** convertis au **taux moyen sans écart** (et sans report
/// d'à-nouveau) et des comptes de **bilan** au taux du flux **avec écart F80/F81**
/// (et report F99 → F00), à partir des mêmes codes de flux.
/// Cf. docs/QUESTIONS_OUVERTES.md Q32.
pub const DDL_DIM_FLOW_SCHEME: &str = "\
CREATE TABLE dim_flow_scheme (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 8i. sat_flow_scheme_item : articulation **complète** des flux d'un schéma.
///
/// Pour chaque `(schéma, flux)` : taux de conversion, flux d'écart, flux de
/// report de clôture, flux d'à-nouveau. **Complet** (pas de table éparse) : un
/// schéma doit définir **tous** les flux que portent les comptes qui l'utilisent
/// (sinon leurs clôtures/conversions disparaîtraient). C'est la source de vérité
/// du comportement des flux ; `dim_flow` n'est plus qu'une dimension nue. La
/// résolution par compte se fait via la vue [`v_flow_behavior`].
pub const DDL_SAT_FLOW_SCHEME_ITEM: &str = "\
CREATE TABLE sat_flow_scheme_item (
    scheme          TEXT,
    flow            TEXT,
    taux_conversion TEXT CHECK (taux_conversion IN ('close_n1', 'avg', 'close_n')),
    flux_ecart      TEXT,           -- flux d'écart associé (NULL = aucun écart)
    flux_de_report  TEXT,           -- flux de clôture où ce flux se reporte ; auto-référence (flow = flux_de_report) = clôture reconstruite
    flux_a_nouveau  TEXT,           -- flux d'ouverture qui reçoit ce solde à l'exercice suivant (F99 → F00) ; NULL sinon
    PRIMARY KEY (scheme, flow)
);";

/// 8j. v_flow_behavior : **vue** résolvant le comportement d'un flux **par compte**.
///
/// Joint chaque compte à son schéma de flux (`dim_account.flow_scheme`, ou à
/// défaut dérivé de la classe : `resultat` → `RESULTAT`, sinon `BILAN`) et expose
/// `(account, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau)`.
/// Source unique consommée par `pipeline::convert`, `materialize_closures` et
/// `pipeline::a_nouveau` (à la place de l'ex-`dim_flow.*`). Cf. Q32.
pub const DDL_V_FLOW_BEHAVIOR: &str = "\
CREATE VIEW v_flow_behavior AS
SELECT
    a.code           AS account,
    si.flow          AS flow,
    si.taux_conversion,
    si.flux_ecart,
    si.flux_de_report,
    si.flux_a_nouveau
FROM dim_account a
JOIN sat_flow_scheme_item si
  ON si.scheme = COALESCE(a.flow_scheme,
                          CASE WHEN a.classe = 'resultat' THEN 'RESULTAT' ELSE 'BILAN' END);";

/// 4b. dim_sous_classe : sous-classes de comptes (actif / passif / charges / produits).
///
/// Table de référence pour `dim_account.sous_classe`. La `classe` reprend la
/// même nomenclature que `dim_account.classe` (bilan / resultat / flux).
pub const DDL_DIM_SOUS_CLASSE: &str = "\
CREATE TABLE dim_sous_classe (
    code    TEXT PRIMARY KEY,
    libelle TEXT,
    classe  TEXT CHECK (classe IN ('bilan', 'resultat', 'flux'))
);";

/// 5. dim_flow : catalogue des flux (cf. docs/FLUX_CONSO.md §6).
///
/// **Dimension nue** (`code`, `libellé`) : tout le comportement d'un flux
/// (taux de conversion, flux d'écart, flux de report de clôture, flux
/// d'à-nouveau) est **déporté dans le schéma de flux** (`sat_flow_scheme_item`),
/// résolu par compte via [`v_flow_behavior`] (cf. Q32, décision 2026-06-21).
pub const DDL_DIM_FLOW: &str = "\
CREATE TABLE dim_flow (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 6. dim_currency : devise référentielle (code ISO, décimales).
pub const DDL_DIM_CURRENCY: &str = "\
CREATE TABLE dim_currency (
    code_iso  TEXT PRIMARY KEY,
    libelle   TEXT,
    decimales INT
);";

/// 6b. dim_nature : nature des écritures (liasse, ajustement…).
///
/// La nature est une dimension **obligatoire** de toutes les écritures :
/// deux écritures de natures différentes ne sont jamais agrégées. Elle est
/// préservée à travers toutes les étapes du pipeline (cf. `pipeline::*`).
pub const DDL_DIM_NATURE: &str = "\
CREATE TABLE dim_nature (
    code    TEXT PRIMARY KEY,
    libelle TEXT,
    rules   TEXT
);";

/// 6c. dim_method : méthodes de consolidation (globale, proportionnelle,
/// mise en équivalence…).
///
/// Le flag `consolidated` distingue les méthodes **intégrées** (true :
/// l'écriture est reprise au niveau `consolidated`, pondérée par
/// `pct_integration`) des méthodes **non intégrées** (false : mise en
/// équivalence, exclue du MVP). L'étape D (cf. `pipeline::consolidate`)
/// filtre par `JOIN dim_method m ON m.code = p.methode WHERE m.consolidated`.
/// Ajouter une méthode consolidée = insérer une ligne ici, sans toucher au SQL.
pub const DDL_DIM_METHOD: &str = "\
CREATE TABLE dim_method (
    code         TEXT PRIMARY KEY,
    libelle      TEXT,
    consolidated BOOLEAN
);";

// --- Tables satellites (règles de consolidation) ------------------------------

/// 7. sat_perimeter : composition du périmètre par (entity, scenario, period).
///
/// Définit la méthode d'intégration (globale / proportionnelle / équivalence),
/// les pourcentages d'intérêt et d'intégration, et les variations de périmètre
/// (entrée / sortie) pour l'exercice courant.
pub const DDL_SAT_PERIMETER: &str = "\
CREATE TABLE sat_perimeter (
    perimeter_set   TEXT,          -- FK dim_perimeter_set : version du périmètre (cf. Q35)
    entity          TEXT,
    period          TEXT,          -- correspond au Entry_period (exercice clôturé)
    methode         TEXT,          -- FK dim_method.code (intégrité via references.rs, pas de CHECK : les méthodes sont pilotables)
    pct_interet     DECIMAL(10,4),
    pct_integration DECIMAL(10,4), -- % de contrôle (1.0 pour la globale)
    entree          BOOLEAN DEFAULT FALSE,
    sortie          BOOLEAN DEFAULT FALSE,
    PRIMARY KEY (perimeter_set, entity, period)
);";

/// 8. sat_exchange_rate : taux de change vers la devise **pivot**.
///
/// Tous les taux convertissent `currency_source` → `pivot_currency` (lue dans
/// `app_config`). Pour passer en devise de présentation, l'étape C calcule un
/// cross-rate : `taux(fonctionnelle → pivot) / taux(présentation → pivot)`.
/// La PK inclut `rate_set` : un même couple (source, période) peut exister
/// dans plusieurs jeux de taux (réels vs budget). Cf. SPEC_SCENARIO_V2.md §1, §2.
pub const DDL_SAT_EXCHANGE_RATE: &str = "\
CREATE TABLE sat_exchange_rate (
    rate_set        TEXT,        -- FK dim_rate_set
    currency_source TEXT,        -- devise source (convertie vers le pivot)
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    PRIMARY KEY (rate_set, currency_source, period)
);";

// --- Règles de consolidation (bibliothèque + jeux) -----------------------------

/// 8b. dim_rule : bibliothèque centrale des règles de consolidation.
///
/// `definition` contient un JSON décrivant le scope (conditions sur le périmètre)
/// et les opérations à appliquer (sélection, coefficient, multiplicateur,
/// destination). Cf. `rules::run_ruleset` pour l'exécution.
pub const DDL_DIM_RULE: &str = "\
CREATE TABLE dim_rule (
    code        TEXT PRIMARY KEY,
    libelle     TEXT,
    definition  TEXT          -- JSON : scope + operations
);";

/// 8c. dim_ruleset : jeu de règles ordonné (références vers dim_rule).
pub const DDL_DIM_RULESET: &str = "\
CREATE TABLE dim_ruleset (
    code        TEXT PRIMARY KEY,
    libelle     TEXT
);";

/// 8d. dim_ruleset_item : items ordonnés d'un jeu (lien vers dim_rule).
///
/// La PK (ruleset_code, ordre) garantit l'unicité de l'ordre dans un jeu.
pub const DDL_DIM_RULESET_ITEM: &str = "\
CREATE TABLE dim_ruleset_item (
    ruleset_code TEXT,
    ordre        INTEGER,
    rule_code    TEXT,
    PRIMARY KEY (ruleset_code, ordre)
);";

/// 8e. dim_custom_dimension : registre des dimensions custom (cf. `dimensions`).
///
/// Les dimensions ajoutées par l'utilisateur sont toutes de catégorie
/// `Analytical` (donc nullables). Leurs colonnes physiques sont ajoutées à
/// `fact_entry` / `stg_entry` via `ALTER TABLE ADD COLUMN` à la création.
///
/// `CREATE TABLE IF NOT EXISTS` : la table doit survivre à un reset complet
/// (sinon le registre et les colonnes seraient perdus). `ALL_DROP` ne la
/// supprime pas et `create_schema` ré-applique les `ALTER TABLE ADD COLUMN`
/// après re-création du schéma.
pub const DDL_DIM_CUSTOM_DIMENSION: &str = "\
CREATE TABLE IF NOT EXISTS dim_custom_dimension (
    name  TEXT PRIMARY KEY,
    label TEXT NOT NULL
);";

/// 8f. dim_characteristic : registre des **caractéristiques N1** (regroupements).
///
/// Une caractéristique N1 classe les membres d'une dimension de base (ex.
/// `comportement` sur les comptes). Sa création crée une table de valeurs
/// `car_<code>` et ajoute une colonne `<code>` sur la master data de la
/// dimension de base (cf. `crate::characteristics`). Comme
/// `dim_custom_dimension`, ce registre **survit au reset** (CREATE IF NOT EXISTS,
/// hors `ALL_DROP`) ; `create_schema` ré-applique ensuite les colonnes perdues.
pub const DDL_DIM_CHARACTERISTIC: &str = "\
CREATE TABLE IF NOT EXISTS dim_characteristic (
    code           TEXT PRIMARY KEY,
    libelle        TEXT,
    base_dimension TEXT NOT NULL
);";

/// 8g. dim_characteristic_attribute : registre des **attributs N2** d'une N1.
///
/// Chaque attribut N2 est une colonne de la table de valeurs `car_<char>`,
/// déclarée comme référence vers la dimension `target_dimension`. Survit au
/// reset comme le registre N1.
pub const DDL_DIM_CHARACTERISTIC_ATTRIBUTE: &str = "\
CREATE TABLE IF NOT EXISTS dim_characteristic_attribute (
    characteristic_code TEXT NOT NULL,
    name                TEXT NOT NULL,
    libelle             TEXT,
    target_dimension    TEXT NOT NULL,
    PRIMARY KEY (characteristic_code, name)
);";

/// 8h. dim_custom_reference : registre des **références directes** (patron B).
///
/// Une référence directe ajoute une colonne `<column_name>` sur la master data
/// de la dimension `host_dimension`, déclarée comme référence vers
/// `target_dimension` (y compris elle-même : hiérarchie). C'est la version
/// pilotable du patron historiquement codé en dur (`dim_account.compte_parent`,
/// `dim_entity.entite_parent`). Comme les registres ci-dessus, il **survit au
/// reset** (CREATE IF NOT EXISTS, hors `ALL_DROP`) ; `create_schema` ré-applique
/// ensuite les colonnes perdues (cf. `crate::custom_references::reapply`).
pub const DDL_DIM_CUSTOM_REFERENCE: &str = "\
CREATE TABLE IF NOT EXISTS dim_custom_reference (
    host_dimension   TEXT NOT NULL,
    column_name      TEXT NOT NULL,
    target_dimension TEXT NOT NULL,
    PRIMARY KEY (host_dimension, column_name)
);";

// --- Staging : saisie brute (format liasse CSV) -------------------------------

/// 9. stg_entry : saisie brute — mêmes dimensions que fact_entry sans `level`,
/// **plus** une colonne `source` non-dimensionnelle (métadonnée de provenance,
/// NON propagée par le pipeline).
pub const DDL_STG_ENTRY: &str = "\
CREATE TABLE stg_entry (
    scenario     TEXT,
    entity       TEXT,
    entry_period TEXT,
    period       TEXT,
    account      TEXT,
    flow         TEXT,
    currency     TEXT,
    nature       TEXT NOT NULL,
    partner      TEXT,
    share        TEXT,
    analysis     TEXT,
    analysis2    TEXT,
    source       TEXT,            -- métadonnée non-dimensionnelle : provenance de
                                  -- la ligne (réf. liasse source, etc.). Hors
                                  -- registre des dimensions → non propagée.
    amount       DECIMAL(18,2)
);";

// --- Table de faits : écritures aux 4 niveaux de stockage ---------------------

/// 10. fact_entry : table de faits — écritures consolidées aux 4 niveaux.
pub const DDL_FACT_ENTRY: &str = "\
CREATE TABLE fact_entry (
    id           INTEGER DEFAULT nextval('seq_entry'),
    scenario     TEXT,
    entity       TEXT,
    entry_period TEXT,
    period       TEXT,
    account      TEXT,
    flow         TEXT,
    currency     TEXT,
    nature       TEXT NOT NULL,
    partner      TEXT,
    share        TEXT,
    analysis     TEXT,
    analysis2    TEXT,
    level        TEXT CHECK (level IN ('corporate', 'converted', 'consolidated')),
    amount       DECIMAL(18,2),
    PRIMARY KEY (id)
);";

// ─────────────────────────────────────────────────────────────────────────────
//  Liste ordonnée du DDL complet — utile pour `create_schema()`.
// ─────────────────────────────────────────────────────────────────────────────

/// Toutes les ordres DDL dans l'ordre de création (dimensions → satellites → fait).
///
/// `DDL_DIM_CUSTOM_DIMENSION` précède `DDL_STG_ENTRY` / `DDL_FACT_ENTRY` : il
/// faut que la table registre existe quand `create_schema` exécute les
/// `ALTER TABLE ADD COLUMN` pour ré-appliquer les customs survivantes.
///
/// Ordre des nouvelles tables (SPEC_SCENARIO_V2.md §8.1) :
/// - `app_config` et `dim_rate_set` **avant** `sat_exchange_rate` (FK logique).
/// - `dim_scenario_category`, `dim_variant`, `dim_ruleset` **avant**
///   `dim_scenario`.
pub const ALL_DDL: &[&str] = &[
    DDL_SEQ_ENTRY,
    DDL_APP_CONFIG,
    DDL_DIM_SCENARIO_CATEGORY,
    DDL_DIM_RATE_SET,
    DDL_DIM_PERIMETER_SET,
    DDL_DIM_VARIANT,
    DDL_DIM_SCENARIO,
    DDL_DIM_ENTITY,
    DDL_DIM_PERIOD,
    DDL_DIM_ACCOUNT,
    DDL_DIM_SOUS_CLASSE,
    DDL_DIM_FLOW,
    DDL_DIM_FLOW_SCHEME,
    DDL_DIM_CURRENCY,
    DDL_DIM_NATURE,
    DDL_DIM_METHOD,
    DDL_SAT_PERIMETER,
    DDL_SAT_EXCHANGE_RATE,
    DDL_SAT_FLOW_SCHEME_ITEM,
    DDL_V_FLOW_BEHAVIOR,
    DDL_DIM_RULE,
    DDL_DIM_RULESET,
    DDL_DIM_RULESET_ITEM,
    DDL_DIM_CUSTOM_DIMENSION,
    DDL_DIM_CHARACTERISTIC,
    DDL_DIM_CHARACTERISTIC_ATTRIBUTE,
    DDL_DIM_CUSTOM_REFERENCE,
    DDL_STG_ENTRY,
    DDL_FACT_ENTRY,
];

/// Ordres de suppression (DROP) dans l'ordre inverse des dépendances.
///
/// `dim_custom_dimension` n'est **pas** droppée : le registre des dimensions
/// custom survive à un reset (et `create_schema` ré-applique ensuite les
/// `ALTER TABLE ADD COLUMN` correspondants sur `fact_entry` / `stg_entry`).
pub const ALL_DROP: &[&str] = &[
    "DROP VIEW IF EXISTS v_flow_behavior;",
    "DROP TABLE IF EXISTS fact_entry;",
    "DROP TABLE IF EXISTS stg_entry;",
    "DROP TABLE IF EXISTS dim_ruleset_item;",
    "DROP TABLE IF EXISTS dim_ruleset;",
    "DROP TABLE IF EXISTS dim_rule;",
    "DROP TABLE IF EXISTS sat_flow_scheme_item;",
    "DROP TABLE IF EXISTS sat_exchange_rate;",
    "DROP TABLE IF EXISTS sat_perimeter;",
    "DROP TABLE IF EXISTS dim_method;",
    "DROP TABLE IF EXISTS dim_nature;",
    "DROP TABLE IF EXISTS dim_currency;",
    "DROP TABLE IF EXISTS dim_flow_scheme;",
    "DROP TABLE IF EXISTS dim_flow;",
    "DROP TABLE IF EXISTS dim_sous_classe;",
    "DROP TABLE IF EXISTS dim_account;",
    "DROP TABLE IF EXISTS dim_period;",
    "DROP TABLE IF EXISTS dim_entity;",
    "DROP TABLE IF EXISTS dim_scenario;",
    "DROP TABLE IF EXISTS dim_variant;",
    "DROP TABLE IF EXISTS dim_perimeter_set;",
    "DROP TABLE IF EXISTS dim_rate_set;",
    "DROP TABLE IF EXISTS dim_scenario_category;",
    "DROP TABLE IF EXISTS app_config;",
    "DROP SEQUENCE IF EXISTS seq_entry;",
];

/// Crée toutes les tables (idempotent) en préservant les dimensions custom.
///
/// Étapes :
/// 1. Sauvegarde des customs existantes depuis `dim_custom_dimension`
///    (la table survit aux resets — sinon vecteur vide).
/// 2. DROP de toutes les tables **sauf** `dim_custom_dimension`.
/// 3. CREATE de toutes les tables (incluant `dim_custom_dimension` via
///    `IF NOT EXISTS`).
/// 4. Ré-applique les customs survivantes : `ALTER TABLE ADD COLUMN` sur
///    `fact_entry` et `stg_entry` + re-INSERT dans le registre.
pub fn create_schema(con: &duckdb::Connection) -> duckdb::Result<()> {
    // 1. Sauvegarder les customs survivantes.
    let saved_customs = crate::dimensions::load_customs(con).unwrap_or_default();

    // 2. DROP (sans toucher à dim_custom_dimension).
    for stmt in ALL_DROP {
        con.execute(stmt, [])?;
    }

    // 3. CREATE (dim_custom_dimension utilise CREATE TABLE IF NOT EXISTS).
    for stmt in ALL_DDL {
        con.execute(stmt, [])?;
    }

    // 4. Ré-appliquer les colonnes custom survivantes.
    crate::dimensions::apply_custom_columns(con, &saved_customs)?;

    // 5. Ré-appliquer les colonnes de rattachement des caractéristiques N1
    //    (perdues au DROP des tables de dimension de base ; les tables de
    //    valeurs `car_<code>` survivent au reset, donc ne sont pas recréées).
    crate::characteristics::reapply(con)?;

    // 6. Ré-appliquer les colonnes des références directes (patron B), perdues
    //    elles aussi au DROP des dimensions hôtes (le registre survit au reset).
    crate::custom_references::reapply(con)?;

    Ok(())
}
