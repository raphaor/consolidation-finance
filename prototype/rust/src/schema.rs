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
//! | reclassified | fonctionnelle  | B. reclassification         |
//! | converted    | présentation   | C. conversion multi-devises |
//! | consolidated | présentation   | D. consolidation (méthodes) |
//!
//! Une table de staging `stg_entry` reçoit la saisie brute (liasses CSV).
//! L'étape A lit cette table et produit le niveau *corporate*.

// ─────────────────────────────────────────────────────────────────────────────
//  DDL — ordre : séquence, dimensions, satellites, staging, table de faits
//  Chaque constante contient un ordre SQL complète (CREATE TABLE / SEQUENCE).
// ─────────────────────────────────────────────────────────────────────────────

/// Séquence d'identifiants auto-incrémentés pour la table de faits.
pub const DDL_SEQ_ENTRY: &str = "CREATE SEQUENCE IF NOT EXISTS seq_entry START 1;";

// --- Dimensions (master data) -------------------------------------------------

/// 1. dim_scenario : scénario de consolidation (réel / budget / prévision).
pub const DDL_DIM_SCENARIO: &str = "\
CREATE TABLE dim_scenario (
    code     TEXT PRIMARY KEY,
    libelle  TEXT,
    type     TEXT,        -- réel / budget / prévision
    statut   TEXT         -- ouvert / verrouillé
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

/// 4. dim_account : plan de compte du groupe (classe + hiérarchie).
///
/// `sous_classe` référence `dim_sous_classe.code` (pas de FK dure en DuckDB pour
/// le proto — contrainte uniquement sémantique). `technical_grouping` permet de
/// regrouper des comptes par nature (ex. `capitaux_propres` pour la mise en
/// équivalence), indépendamment de la classe comptable.
pub const DDL_DIM_ACCOUNT: &str = "\
CREATE TABLE dim_account (
    code               TEXT PRIMARY KEY,
    libelle            TEXT,
    classe             TEXT CHECK (classe IN ('bilan', 'resultat', 'flux')),
    sous_classe        TEXT,           -- référence dim_sous_classe.code
    technical_grouping TEXT,           -- regroupement par nature (ex. capitaux_propres)
    compte_parent      TEXT            -- hiérarchie d'agrégation
);";

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

/// 5. dim_flow : catalogue des flux de consolidation (cf. docs/FLUX_CONSO.md §6).
///
/// Modèle de flux :
///   F00 = Ouverture              (taux close_n1, écart → F80, reporte à F99)
///   F01 = Entrée périmètre       (taux close_n1, écart → F80, reporte à F99)
///   F20 = Variation              (taux avg,      écart → F81, reporte à F99)
///   F80 = Écart conv. ouverture  (terminal,      reporte à F99)
///   F81 = Écart conv. variation  (terminal,      reporte à F99)
///   F98 = Sortie périmètre       (terminal,      reporte à F99)
///   F99 = Clôture                (close_n, auto-référentiel → clôture reconstruite)
///
/// Reconstruction (cf. `pipeline::materialize_closures`) : pour chaque clôture C
/// (flux auto-référentiel : `flux_de_report(C) = C`), `C = Σ(X | flux_de_report(X)
/// = C et X ≠ C)`. Aujourd'hui seule F99 est une clôture ; la logique est
/// générique et pilotée par `dim_flow.flux_de_report`.
pub const DDL_DIM_FLOW: &str = "\
CREATE TABLE dim_flow (
    code             TEXT PRIMARY KEY,
    libelle          TEXT,
    taux_conversion  TEXT CHECK (taux_conversion IN ('close_n1', 'avg', 'close_n', 'terminal')),
    flux_ecart       TEXT,           -- flux d'écart de conversion associé (NULL pour les terminaux)
    flux_de_report   TEXT DEFAULT 'F99'   -- flux dans lequel ce flux se reporte ; auto-référence = clôture reconstruite
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

// --- Tables satellites (règles de consolidation) ------------------------------

/// 7. sat_perimeter : composition du périmètre par (entity, scenario, period).
///
/// Définit la méthode d'intégration (globale / proportionnelle / équivalence),
/// les pourcentages d'intérêt et d'intégration, et les variations de périmètre
/// (entrée / sortie) pour l'exercice courant.
pub const DDL_SAT_PERIMETER: &str = "\
CREATE TABLE sat_perimeter (
    entity          TEXT,
    scenario        TEXT,
    period          TEXT,          -- correspond au Entry_period (exercice clôturé)
    methode         TEXT CHECK (methode IN ('globale', 'proportionnelle', 'équivalence')),
    pct_interet     DECIMAL(10,4),
    pct_integration DECIMAL(10,4), -- % de contrôle (1.0 pour la globale)
    entree          BOOLEAN DEFAULT FALSE,
    sortie          BOOLEAN DEFAULT FALSE,
    PRIMARY KEY (entity, scenario, period)
);";

/// 8. sat_exchange_rate : taux de change vers la devise de présentation.
pub const DDL_SAT_EXCHANGE_RATE: &str = "\
CREATE TABLE sat_exchange_rate (
    currency_source TEXT,   -- devise source (à convertir vers la présentation)
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    PRIMARY KEY (currency_source, period)
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

// --- Staging : saisie brute (format liasse CSV) -------------------------------

/// 9. stg_entry : saisie brute — même structure que fact_entry sans la colonne `level`.
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
    level        TEXT CHECK (level IN ('corporate', 'reclassified', 'converted', 'consolidated')),
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
pub const ALL_DDL: &[&str] = &[
    DDL_SEQ_ENTRY,
    DDL_DIM_SCENARIO,
    DDL_DIM_ENTITY,
    DDL_DIM_PERIOD,
    DDL_DIM_ACCOUNT,
    DDL_DIM_SOUS_CLASSE,
    DDL_DIM_FLOW,
    DDL_DIM_CURRENCY,
    DDL_DIM_NATURE,
    DDL_SAT_PERIMETER,
    DDL_SAT_EXCHANGE_RATE,
    DDL_DIM_RULE,
    DDL_DIM_RULESET,
    DDL_DIM_RULESET_ITEM,
    DDL_DIM_CUSTOM_DIMENSION,
    DDL_STG_ENTRY,
    DDL_FACT_ENTRY,
];

/// Ordres de suppression (DROP) dans l'ordre inverse des dépendances.
///
/// `dim_custom_dimension` n'est **pas** droppée : le registre des dimensions
/// custom survive à un reset (et `create_schema` ré-applique ensuite les
/// `ALTER TABLE ADD COLUMN` correspondants sur `fact_entry` / `stg_entry`).
pub const ALL_DROP: &[&str] = &[
    "DROP TABLE IF EXISTS fact_entry;",
    "DROP TABLE IF EXISTS stg_entry;",
    "DROP TABLE IF EXISTS dim_ruleset_item;",
    "DROP TABLE IF EXISTS dim_ruleset;",
    "DROP TABLE IF EXISTS dim_rule;",
    "DROP TABLE IF EXISTS sat_exchange_rate;",
    "DROP TABLE IF EXISTS sat_perimeter;",
    "DROP TABLE IF EXISTS dim_nature;",
    "DROP TABLE IF EXISTS dim_currency;",
    "DROP TABLE IF EXISTS dim_flow;",
    "DROP TABLE IF EXISTS dim_sous_classe;",
    "DROP TABLE IF EXISTS dim_account;",
    "DROP TABLE IF EXISTS dim_period;",
    "DROP TABLE IF EXISTS dim_entity;",
    "DROP TABLE IF EXISTS dim_scenario;",
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

    Ok(())
}
