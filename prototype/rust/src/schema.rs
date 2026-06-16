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
    partner      TEXT,
    share        TEXT,
    analysis     TEXT,
    audit_id     TEXT,
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
    partner      TEXT,
    share        TEXT,
    analysis     TEXT,
    audit_id     TEXT,
    level        TEXT CHECK (level IN ('corporate', 'reclassified', 'converted', 'consolidated')),
    amount       DECIMAL(18,2),
    PRIMARY KEY (id)
);";

// ─────────────────────────────────────────────────────────────────────────────
//  Liste ordonnée du DDL complet — utile pour `create_schema()`.
// ─────────────────────────────────────────────────────────────────────────────

/// Toutes les ordres DDL dans l'ordre de création (dimensions → satellites → fait).
pub const ALL_DDL: &[&str] = &[
    DDL_SEQ_ENTRY,
    DDL_DIM_SCENARIO,
    DDL_DIM_ENTITY,
    DDL_DIM_PERIOD,
    DDL_DIM_ACCOUNT,
    DDL_DIM_SOUS_CLASSE,
    DDL_DIM_FLOW,
    DDL_DIM_CURRENCY,
    DDL_SAT_PERIMETER,
    DDL_SAT_EXCHANGE_RATE,
    DDL_STG_ENTRY,
    DDL_FACT_ENTRY,
];

/// Ordres de suppression (DROP) dans l'ordre inverse des dépendances.
///
/// Permet de repartir d'un état propre avant de recréer le schéma
/// (idempotence en cas de re-exécution).
pub const ALL_DROP: &[&str] = &[
    "DROP TABLE IF EXISTS fact_entry;",
    "DROP TABLE IF EXISTS stg_entry;",
    "DROP TABLE IF EXISTS sat_exchange_rate;",
    "DROP TABLE IF EXISTS sat_perimeter;",
    "DROP TABLE IF EXISTS dim_currency;",
    "DROP TABLE IF EXISTS dim_flow;",
    "DROP TABLE IF EXISTS dim_sous_classe;",
    "DROP TABLE IF EXISTS dim_account;",
    "DROP TABLE IF EXISTS dim_period;",
    "DROP TABLE IF EXISTS dim_entity;",
    "DROP TABLE IF EXISTS dim_scenario;",
    "DROP SEQUENCE IF EXISTS seq_entry;",
];

/// Crée toutes les tables (idempotent).
///
/// Supprime d'abord les tables existantes (cf. `ALL_DROP`) puis exécute
/// le DDL complet (cf. `ALL_DDL`). Miroir de `conso/schema.py::create_schema`.
pub fn create_schema(con: &duckdb::Connection) -> duckdb::Result<()> {
    for stmt in ALL_DROP {
        con.execute(stmt, [])?;
    }
    for stmt in ALL_DDL {
        con.execute(stmt, [])?;
    }
    Ok(())
}
