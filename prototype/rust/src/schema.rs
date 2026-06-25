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

/// Séquence d'identifiants dédiée à `stg_entry` (distincte de `seq_entry` pour
/// éviter toute collision d'id avec `fact_entry`). Permet l'édition/suppression
/// unitaire des saisies manuelles (`Source = 'MANUAL'`) via `POST/PUT/DELETE
/// /api/entries`.
pub const DDL_SEQ_STG_ENTRY: &str = "CREATE SEQUENCE IF NOT EXISTS seq_stg_entry START 1;";

/// Séquence d'identifiants auto-incrémentés pour `dim_consolidation` (PK
/// technique — l'identité métier est la clé naturelle à 5 éléments). Remplace
/// l'ancien `code` textuel de `dim_scenario`.
pub const DDL_SEQ_CONSOLIDATION: &str = "CREATE SEQUENCE IF NOT EXISTS seq_consolidation START 1;";

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

/// 1. dim_consolidation (ex dim_scenario) : définition de consolidation, objet composite.
///
/// Une consolidation est définie par sa **clé naturelle à 5 éléments** —
/// (phase, exercice, jeu de périmètre, variante, devise de présentation) —
/// matérialisée par une contrainte UNIQUE. `id` est une PK technique auto
/// (l'ancien `code` textuel disparaît). Les saisies ne référencent pas la
/// consolidation : elles sont au grain **remontée** (phase + exercice) via
/// `stg_entry.phase`. `fact_entry` référence la consolidation par `consolidation_id`.
///
/// `perimeter_period` et `rate_period` rendent explicites les périodes du
/// périmètre et des taux (défaut = exercice). Le taux N-1 (`close_n1`) vient de
/// `sat_exchange_rate.taux_ouverture` porté par la période de taux.
pub const DDL_DIM_CONSOLIDATION: &str = "\
CREATE TABLE dim_consolidation (
    id                          INTEGER DEFAULT nextval('seq_consolidation') PRIMARY KEY,
    libelle                     TEXT,
    -- Clé naturelle (identité métier) :
    phase                       INTEGER,-- FK dim_scenario_category.id (clé technique B1 ; contrat code 'REEL')
    exercice                    TEXT,   -- FK dim_period ('2024') — sélectionne la remontée
    perimeter_set               INTEGER,-- FK dim_perimeter_set.id (clé technique B1 ; contrat code)
    variant                     INTEGER,-- FK dim_variant.id (clé technique, chantier B1 ; contrat externe = code 'BASE')
    presentation_currency       TEXT,   -- FK dim_currency ('EUR')
    -- Hors clé (paramètres de traitement) :
    perimeter_period            TEXT,   -- FK dim_period (défaut = exercice)
    rate_set                    INTEGER,-- FK dim_rate_set.id (clé technique B1 ; contrat code)
    rate_period                 TEXT,   -- FK dim_period (défaut = exercice)
    ruleset_code                TEXT,   -- FK dim_ruleset (NULL = pas de règles)
    a_nouveau_consolidation_id  INTEGER, -- FK dim_consolidation : conso N-1 figée dont on reporte l'ouverture (NULL = pas d'à-nouveau). Cf. docs/A_NOUVEAU.md §2.2
    statut                      TEXT,   -- 'brouillon' / 'ouvert' / 'verrouillé'
    UNIQUE (phase, exercice, perimeter_set, variant, presentation_currency)
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
    classe  TEXT CHECK (classe IN ('bilan', 'resultat', 'flux')),
    -- Sens comptable user-driven (Q44) : 'C' créditeur (passif, produits),
    -- 'D' débiteur (actif, charges). NULL = exclu des totaux signés des rapports.
    -- Remplace le CASE en dur `SENS_CASE` (server.rs) — les rapports signent via
    -- un JOIN sur cette colonne. Rend `sous_classe` renommable (plus de dur).
    sens    TEXT CHECK (sens IN ('C', 'D'))
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
    perimeter_set   INTEGER,       -- FK dim_perimeter_set.id (clé technique B1 ; contrat code)
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
    rate_set        INTEGER,     -- FK dim_rate_set.id (clé technique B1 ; contrat code)
    currency_source TEXT,        -- devise source (convertie vers le pivot)
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    taux_ouverture  DECIMAL(18,8), -- taux d'ouverture de N (= clôture N-1). Porté par N : résout `close_n1` sans période antérieure ni à-nouveau (1ʳᵉ consolidation possible).
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
///
/// Colonne `native` : les lignes marquées `TRUE` sont peuplées automatiquement
/// par `custom_references::seed_native` depuis le catalogue statique
/// `references::NATIVE_MASTER_REFS` (FK natives des master data). Elles sont
/// verrouillées (non éditables/supprimables via l'API) car elles reflètent le DDL.
pub const DDL_DIM_CUSTOM_REFERENCE: &str = "\
CREATE TABLE IF NOT EXISTS dim_custom_reference (
    host_dimension   TEXT NOT NULL,
    column_name      TEXT NOT NULL,
    target_dimension TEXT NOT NULL,
    native           BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (host_dimension, column_name)
);";

/// 8k. dim_value_list : registre des **listes de valeurs** (référentiels).
///
/// Une liste de valeurs est une nomenclature `code/libellé` autonome
/// (`lst_<code>`), réutilisable comme cible d'un attribut N2 de caractéristique,
/// mais qui n'est **pas une dimension** (aucune colonne sur `fact_entry` /
/// `stg_entry`). Comme les autres registres pilotables, elle **survit au reset**
/// (CREATE IF NOT EXISTS, hors `ALL_DROP`) ; les tables `lst_<code>` survivent
/// elles aussi (jamais droppées) et n'ont aucune colonne à ré-appliquer
/// (cf. `crate::value_lists`).
pub const DDL_DIM_VALUE_LIST: &str = "\
CREATE TABLE IF NOT EXISTS dim_value_list (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);";

/// 8l. dim_coefficient : bibliothèque de **coefficients** (moteur de formules,
/// volet 1 — cf. `docs/FORMULES.md` §3, [Q43]).
///
/// Chaque coefficient est une **formule nommée** (`expression`, langage type
/// Excel) compilée au grain d'une écriture de règle vers `(SQL, CoeffJoins)`
/// (cf. `crate::coefficients`). `kind` distingue les coefficients **natifs**
/// (`builtin`, seedés depuis `coefficients::BUILTINS`) des coefficients
/// **utilisateur** (`user`). Comme les autres registres pilotables, la table
/// **survit au reset** (CREATE IF NOT EXISTS, hors `ALL_DROP`) : les coefficients
/// utilisateur sont des actifs persistants. Les natifs sont (re)seedés à chaque
/// `create_schema` (idempotent, INSERT OR IGNORE).
pub const DDL_DIM_COEFFICIENT: &str = "\
CREATE TABLE IF NOT EXISTS dim_coefficient (
    code       TEXT PRIMARY KEY,
    libelle    TEXT,
    expression TEXT NOT NULL,
    kind       TEXT NOT NULL DEFAULT 'user'
);";

/// 8m. dim_aggregate : **postes** (briques agrégées) du moteur d'indicateurs
/// (volet 2 — cf. `docs/FORMULES.md` §4, [Q43]).
///
/// Un poste est une **sélection nommée** sur `fact_entry` : `level` + `definition`
/// JSON (`{level, selection:[…]}`), agrégée en un montant. Consommé par
/// `crate::indicators` (compilé en `SUM(amount) FILTER (WHERE …)`). Survit au
/// reset comme les autres registres pilotables (hors `ALL_DROP`). Pas de natifs.
pub const DDL_DIM_AGGREGATE: &str = "\
CREATE TABLE IF NOT EXISTS dim_aggregate (
    code       TEXT PRIMARY KEY,
    libelle    TEXT,
    level      TEXT NOT NULL,
    definition TEXT NOT NULL
);";

/// 8n. dim_indicator : **indicateurs / KPI** — formules combinant des postes.
///
/// `expression` (langage `formula.rs`) + `grain` (JSON : dimensions de
/// restitution) + `format` (%, ratio, nombre…). Compilé en une requête au grain
/// par `crate::indicators`. **Jamais** réinjecté dans `fact_entry` (couche
/// dérivée). Survit au reset.
pub const DDL_DIM_INDICATOR: &str = "\
CREATE TABLE IF NOT EXISTS dim_indicator (
    code       TEXT PRIMARY KEY,
    libelle    TEXT,
    expression TEXT NOT NULL,
    grain      TEXT,
    format     TEXT
);";

// --- Staging : saisie brute (format liasse CSV) -------------------------------

/// 9. stg_entry : saisie brute — au grain **remontée** (phase + entry_period).
/// `phase` remplace l'ancien `scenario` : les saisies ne référencent plus une
/// consolidation mais la remontée (phase + exercice), partagée entre toutes les
/// consolidations qui la consomment. Mêmes autres dimensions que `fact_entry`
/// sans `level`/`consolidation_id`, **plus** une colonne `source` non-
/// dimensionnelle (métadonnée de provenance, NON propagée par le pipeline).
///
/// `id` (PK auto-incrémentée via `seq_stg_entry`) : rend chaque ligne stable
/// pour l'édition/suppression unitaire via l'API REST. Les imports CSV ne
/// fournissent pas d'id (DEFAULT nextval).
pub const DDL_STG_ENTRY: &str = "\
CREATE TABLE stg_entry (
    id           INTEGER DEFAULT nextval('seq_stg_entry') PRIMARY KEY,
    phase        TEXT,
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

/// 10. fact_entry : table de faits — écritures consolidées aux 3 niveaux.
/// `consolidation_id` (FK dim_consolidation) isole les résultats d'un run (une
/// consolidation). `phase` est la dimension propagée issue de la remontée.
pub const DDL_FACT_ENTRY: &str = "\
CREATE TABLE fact_entry (
    id               INTEGER DEFAULT nextval('seq_entry'),
    consolidation_id INTEGER,
    phase            TEXT,
    entity           TEXT,
    entry_period     TEXT,
    period           TEXT,
    account          TEXT,
    flow             TEXT,
    currency         TEXT,
    nature           TEXT NOT NULL,
    partner          TEXT,
    share            TEXT,
    analysis         TEXT,
    analysis2        TEXT,
    level            TEXT CHECK (level IN ('corporate', 'converted', 'consolidated')),
    amount           DECIMAL(18,2),
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
///   `dim_consolidation`.
pub const ALL_DDL: &[&str] = &[
    DDL_SEQ_ENTRY,
    DDL_SEQ_STG_ENTRY,
    DDL_SEQ_CONSOLIDATION,
    DDL_APP_CONFIG,
    DDL_DIM_SCENARIO_CATEGORY,
    DDL_DIM_RATE_SET,
    DDL_DIM_PERIMETER_SET,
    DDL_DIM_VARIANT,
    DDL_DIM_CONSOLIDATION,
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
    DDL_DIM_VALUE_LIST,
    DDL_DIM_COEFFICIENT,
    DDL_DIM_AGGREGATE,
    DDL_DIM_INDICATOR,
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
    "DROP TABLE IF EXISTS dim_consolidation;",
    "DROP TABLE IF EXISTS dim_variant;",
    "DROP TABLE IF EXISTS dim_perimeter_set;",
    "DROP TABLE IF EXISTS dim_rate_set;",
    "DROP TABLE IF EXISTS dim_scenario_category;",
    "DROP TABLE IF EXISTS app_config;",
    "DROP SEQUENCE IF EXISTS seq_consolidation;",
    "DROP SEQUENCE IF EXISTS seq_stg_entry;",
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

    // 7. Peupler les FK natives du DDL statique dans `dim_custom_reference`
    //    (account.sous_classe, entity.entite_parent, scenario.category, …).
    //    Marquées `native=TRUE` et verrouillées contre édition via l'API.
    //    Idempotent : INSERT OR IGNORE préserve les customs utilisateur.
    crate::custom_references::seed_native(con)?;

    // 8. (Re)seeder les coefficients natifs (moteur de formules, volet 1).
    //    Idempotent (INSERT OR IGNORE) ; les coefficients utilisateur survivent.
    crate::coefficients::seed_builtins(con)?;

    // 9. Doter chaque dimension d'un `id` technique (chantier B1, étape 1).
    //    Idempotent ; non-breaking (les `id` ne sont pas encore consommés).
    crate::surrogate::ensure_ids(con)?;

    Ok(())
}
