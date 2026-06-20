//! Serveur HTTP/JSON exposant le moteur de consolidation via une API REST.
//!
//! Binaire `conso-server` du crate `conso-engine`. Démarre un serveur Axum
//! sur le port 3000 (par défaut) et expose les endpoints suivants :
//!
//! | Méthode | Route        | Description                       |
//! |---------|--------------|-----------------------------------|
//! | GET     | /api/health  | Health check                      |
//! | GET     | /api/levels  | Comptes par niveau                |
//! | GET     | /api/bilan   | Bilan par flux (consolidated)     |
//! | GET     | /api/entries | Écritures filtrées par niveau     |
//! | POST    | /api/run     | Déclenche le pipeline             |
//! | POST    | /api/reset   | Reset DB + reimport CSV           |
//!
//! # Configuration (variables d'environnement)
//!
//! - `CONSO_PORT`          : port d'écoute (défaut : 3000).
//! - `CONSO_DB_PATH`       : chemin du fichier DuckDB (défaut : `conso.duckdb`).
//! - `CONSO_CSV_DIR`       : répertoire contenant les CSV (défaut : `data`).
//! - `CONSO_WEB_DIR`       : répertoire du frontend buildé à servir en statique (défaut : `../../web/dist` depuis `prototype/rust`). Si absent, seule l'API est exposée.
//! - `CONSO_FORCE_RESEED`  : `1` pour forcer le rechargement CSV au démarrage (DROP schéma + import + pipeline), même si la base existe déjà. Utile après une évolution du schéma. À chaud, préférer `POST /api/reset`.
//!
//! # Persistance
//!
//! Au démarrage, les CSV ne sont réimportés que si la base est vierge (schéma
//! absent). Sinon, la base DuckDB existante est conservée telle quelle : les
//! éditions de master data faites via l'UI (périmètre, taux, entités…)
//! survivent aux redémarrages. Pour repartir des CSV : `POST /api/reset` ou
//! `CONSO_FORCE_RESEED=1`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use duckdb::params_from_iter;
use duckdb::types::Value as DbValue;
use duckdb::Connection;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use conso_engine::{
    characteristics, create_schema, dimensions, export, import, load_all, masterdata, money::Money,
    run_pipeline, run_pipeline_with_hook, seed_demo_rules, ConvertParams,
};
use conso_engine::rules::{run_ruleset_at_level, validate_definition, RuleResult, RulesetReport};
use conso_engine::state::{db_err, lock_con, AppError, AppState};

// ─────────────────────────────────────────────────────────────────────────────
//  État partagé et erreurs
// ─────────────────────────────────────────────────────────────────────────────
//
// `AppState`, `AppError`, `db_err` et `lock_con` sont définis dans
// `conso_engine::state` et partagés avec les modules `masterdata` et `import`.

// ─────────────────────────────────────────────────────────────────────────────
//  DTO sérialisés en JSON
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `/api/levels` : nombre d'écritures à un niveau de stockage.
#[derive(Serialize)]
struct LevelCount {
    level: String,
    count: i64,
}

/// Ligne `/api/bilan` : montant agrégé par (compte, flux, nature) au niveau demandé.
///
/// `amount` est sérialisé en **nombre** JSON (feature `serde-float` de
/// `rust_decimal`) — le frontend TS attend `amount: 9774.0`, pas une chaîne.
#[derive(Serialize)]
struct BilanRow {
    account: String,
    flow: String,
    nature: String,
    amount: Decimal,
}

/// Réponse `/api/run` : nombre de lignes produites à chaque étape du pipeline.
#[derive(Serialize)]
struct PipelineResult {
    corporate: usize,
    converted: usize,
    consolidated: usize,
    /// Code du scénario utilisé pour le run (explicite dans le body ou
    /// premier scénario `'ouvert'` trouvé).
    scenario: String,
    /// Ruleset exécuté après le pipeline (NULL si le scénario n'en référence pas).
    ruleset: Option<String>,
    /// Rapport du ruleset, présent si `ruleset` est `Some`.
    ruleset_report: Option<RulesetReport>,
}

/// Réponse `/api/reset` : statut + nombre d'écritures brutes rechargées.
#[derive(Serialize)]
struct ResetResult {
    status: &'static str,
    entries: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Paramètres de requête (query string)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BilanQuery {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default)]
    scenario: Option<String>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
}

#[derive(Deserialize)]
struct EntriesQuery {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    scenario: Option<String>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
}

fn default_level() -> String {
    "consolidated".to_string()
}

fn default_limit() -> i64 {
    100
}

/// Construit le fragment SQL et les paramètres pour les filtres optionnels
/// `scenario`, `entity`, `entry_period` (exercice clôturé), `period`
/// (période impactée par l'écriture) et `nature`. Renvoie une chaîne
/// préfixée par " AND ..." prête à concaténer après un WHERE existant.
fn build_filters(
    scenario: &Option<String>,
    entity: &Option<String>,
    entry_period: &Option<String>,
    period: &Option<String>,
    nature: &Option<String>,
) -> (String, Vec<DbValue>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    if let Some(s) = scenario {
        sql.push_str(" AND scenario = ?");
        params.push(DbValue::Text(s.clone()));
    }
    if let Some(e) = entity {
        sql.push_str(" AND entity = ?");
        params.push(DbValue::Text(e.clone()));
    }
    if let Some(ep) = entry_period {
        sql.push_str(" AND entry_period = ?");
        params.push(DbValue::Text(ep.clone()));
    }
    if let Some(p) = period {
        sql.push_str(" AND period = ?");
        params.push(DbValue::Text(p.clone()));
    }
    if let Some(n) = nature {
        sql.push_str(" AND nature = ?");
        params.push(DbValue::Text(n.clone()));
    }
    (sql, params)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/health — health check simple, toujours 200.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// GET /api/levels — nombre de lignes stockées à chaque niveau de `fact_entry`.
///
/// Même SQL que `report::print_level_counts`, mais renvoyé en JSON.
async fn get_levels(State(state): State<Arc<AppState>>) -> Result<Json<Vec<LevelCount>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                "SELECT level, COUNT(*) AS n
                 FROM fact_entry
                 GROUP BY level
                 ORDER BY CASE level
                     WHEN 'corporate'    THEN 1
                     WHEN 'converted'    THEN 2
                     WHEN 'consolidated' THEN 3
                 END",
            )
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(LevelCount {
                    level: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
        // stmt et MutexGuard libérés ici
    };
    Ok(Json(rows))
}

/// GET /api/bilan?level=consolidated — bilan par flux.
///
/// Le « bilan » au sens large (actif + passif + capitaux propres) regroupe les
/// comptes de classe `bilan`. Les comptes de `resultat` (P&L : classes 6/7) sont
/// exclus — ils sont exposés via `/api/compte-resultat`. On join `dim_account`
/// pour filtrer sur la classe.
async fn get_bilan(
    Query(q): Query<BilanQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BilanRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Totaux = lignes principales uniquement : on exclut les « dont »
        // (dimensions analytiques renseignées), qui sont des détails de la ligne
        // de même grain sans la dimension. Cf. dimensions::analytical_cols.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let of_which: String = dimensions::analytical_cols(&dims)
            .iter()
            .map(|c| format!(" AND e.{c} IS NULL"))
            .collect();
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period, &q.nature);
        let sql = format!(
            "SELECT e.account, e.flow, e.nature, SUM(e.amount) AS amount
             FROM fact_entry e
             JOIN dim_account a ON a.code = e.account
             WHERE e.level = ? AND a.classe = 'bilan' {fsql}{of_which}
             GROUP BY e.account, e.flow, e.nature
             ORDER BY e.account, e.flow, e.nature"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(3)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
                    nature: row.get(2)?,
                    amount: m.into_decimal(),
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/compte-resultat?level=consolidated — compte de résultat par flux.
///
/// Restreint aux comptes de classe « resultat » (P&L : produits et charges).
async fn get_compte_resultat(
    Query(q): Query<BilanQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BilanRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Totaux = lignes principales uniquement (exclut les « dont »). Idem bilan.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let of_which: String = dimensions::analytical_cols(&dims)
            .iter()
            .map(|c| format!(" AND e.{c} IS NULL"))
            .collect();
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period, &q.nature);
        let sql = format!(
            "SELECT e.account, e.flow, e.nature, SUM(e.amount) AS amount
             FROM fact_entry e
             JOIN dim_account a ON a.code = e.account
             WHERE e.level = ? AND a.classe = 'resultat' {fsql}{of_which}
             GROUP BY e.account, e.flow, e.nature
             ORDER BY e.account, e.flow, e.nature"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(3)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
                    nature: row.get(2)?,
                    amount: m.into_decimal(),
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/entries?level=consolidated&limit=100&offset=0 — écritures paginées.
///
/// Colonnes **dynamiques** : toutes les dimensions propagées (built-in +
/// **custom**) + `id`, `level`, `amount`. La sérialisation générique
/// ([`masterdata::run_query`]) renvoie un objet JSON par ligne, donc les
/// dimensions custom apparaissent automatiquement (vue Écritures pilotée par
/// `/api/meta/dimensions`).
///
/// Niveau spécial `raw` : lit la saisie brute (`stg_entry`) avant pipeline,
/// avec un id synthétique (ROW_NUMBER) pour la cohérence de pagination côté UI.
async fn get_entries(
    Query(q): Query<EntriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Colonnes propagées (built-in + custom) depuis le registre.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let col_list = dimensions::propagated_cols(&dims).join(", ");
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period, &q.nature);
        let (sql, params): (String, Vec<DbValue>) = if q.level == "raw" {
            let where_stg = if fsql.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", fsql.trim_start_matches(" AND "))
            };
            let sql = format!(
                "SELECT * FROM (
                    SELECT ROW_NUMBER() OVER (ORDER BY entity, scenario, period, account, flow) AS id,
                           {col_list}, 'raw' AS level, amount
                    FROM stg_entry {where_stg}
                ) ORDER BY id
                LIMIT ? OFFSET ?"
            );
            let mut params = fparams;
            params.push(DbValue::BigInt(q.limit));
            params.push(DbValue::BigInt(q.offset));
            (sql, params)
        } else {
            let sql = format!(
                "SELECT id, {col_list}, level, amount
                 FROM fact_entry
                 WHERE level = ? {fsql}
                 ORDER BY id
                 LIMIT ? OFFSET ?"
            );
            let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
            params.extend(fparams);
            params.push(DbValue::BigInt(q.limit));
            params.push(DbValue::BigInt(q.offset));
            (sql, params)
        };
        masterdata::run_query(&con, &sql, params)?
    };
    Ok(Json(rows))
}

/// Corps accepté par `POST /api/run`.
///
/// `scenario` est optionnel : s'il est absent (body `{}` ou `{}`), le handler
/// sélectionne le premier scénario de statut `'ouvert'`. C'est l'amorti de
/// rétro-compatibilité pendant le développement.
#[derive(Deserialize, Default)]
struct RunBody {
    #[serde(default)]
    scenario: Option<String>,
}

/// GET /api/scenarios — liste des scénarios avec leurs paramètres dépliés.
///
/// Sert le dropdown UI de `PipelinePage`. Une entrée par scénario ; les FK
/// (category, variant, rate_set, ruleset_code) sont ramenées telles quelles
/// (leurs libellés sont récupérés côté UI via les tables master data).
#[derive(Serialize)]
struct ScenarioSummary {
    code: String,
    libelle: Option<String>,
    category: Option<String>,
    entry_period: Option<String>,
    presentation_currency: Option<String>,
    variant: Option<String>,
    ruleset_code: Option<String>,
    rate_set: Option<String>,
    statut: Option<String>,
}

/// GET /api/scenarios — liste de tous les scénarios avec leurs paramètres.
async fn list_scenarios(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ScenarioSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                "SELECT code, libelle, category, entry_period, presentation_currency,
                        variant, ruleset_code, rate_set, statut
                 FROM dim_scenario
                 ORDER BY code",
            )
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(ScenarioSummary {
                    code: row.get(0)?,
                    libelle: row.get(1)?,
                    category: row.get(2)?,
                    entry_period: row.get(3)?,
                    presentation_currency: row.get(4)?,
                    variant: row.get(5)?,
                    ruleset_code: row.get(6)?,
                    rate_set: row.get(7)?,
                    statut: row.get(8)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// POST /api/run — déclenche le pipeline 4 étapes (sur le scénario du body,
/// sinon le 1er scénario `'ouvert'`) et, si le scénario référence un ruleset,
/// exécute celui-ci après le pipeline.
///
/// Workflow (cf. SPEC_SCENARIO_V2.md §7) :
/// 1. Résolution du code scénario (body ou défaut `'ouvert'`).
/// 2. `ConvertParams::load_params(con, scenario_code)`.
/// 3. `DELETE FROM fact_entry WHERE scenario = ?` (reset du scénario courant ;
///    les autres scénarios — snapshot d'à-nouveau figé — sont préservés).
/// 4. `run_pipeline(con, &params)`.
/// 5. Si `scenario.ruleset_code` non NULL : `run_ruleset(con, ruleset_code)`.
/// 6. Retourne `{ counts, scenario, ruleset?, ruleset_report? }`.
async fn run_pipeline_handler(
    State(state): State<Arc<AppState>>,
    body: Option<Json<RunBody>>,
) -> Result<Json<PipelineResult>, AppError> {
    let result = {
        let con = lock_con(&state)?;
        // 1. Résolution du scénario : explicite dans le body, sinon 1er 'ouvert'.
        let scenario_code: String = match body.and_then(|b| b.0.scenario) {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => con
                .query_row(
                    "SELECT code FROM dim_scenario \
                     WHERE statut = 'ouvert' OR statut IS NULL \
                     ORDER BY code LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| {
                    AppError::bad_request(format!(
                        "aucun scénario 'ouvert' trouvé (précisez {{\"scenario\":\"...\"}}) : {e}"
                    ))
                })?,
        };

        // 2. Chargement des params depuis dim_scenario + app_config.
        let params = ConvertParams::load_params(&con, &scenario_code)
            .map_err(db_err)?;

        // 3. Lecture du ruleset_code (NULL si le scénario n'en référence pas).
        let ruleset_code: Option<String> = con
            .query_row(
                "SELECT ruleset_code FROM dim_scenario WHERE code = ?",
                [&scenario_code],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(db_err)?;

        // 4. Vider les résultats du pipeline du SCÉNARIO COURANT avant de relancer
        //    (isolation par scénario : les autres scénarios — ex. snapshot
        //    d'à-nouveau figé — sont préservés). Cf. docs/A_NOUVEAU.md §2.3 / §3.
        con.execute("DELETE FROM fact_entry WHERE scenario = ?", [&scenario_code])
            .map_err(db_err)?;

        // 5. Pipeline. Si le scénario référence un ruleset, on **intercale** ses
        //    règles au niveau qu'elles ciblent (hook post-étape) : une règle
        //    `converted` est injectée juste après l'étape C, puis consolidée par
        //    l'étape D — propagation identique à une écriture manuelle. Sinon,
        //    pipeline natif seul.
        let mut rule_results: Vec<RuleResult> = Vec::new();
        let counts = match &ruleset_code {
            Some(code) => {
                let mut hook = |c: &Connection, level: &str| -> duckdb::Result<()> {
                    rule_results.extend(run_ruleset_at_level(c, code, level)?);
                    Ok(())
                };
                run_pipeline_with_hook(&con, &params, &mut hook)
                    .map_err(db_err)?
                    .counts()
            }
            None => run_pipeline(&con, &params).map_err(db_err)?.counts(),
        };

        // 6. Rapport du ruleset (agrégé depuis les niveaux intercalés).
        let ruleset_report = ruleset_code.as_ref().map(|code| RulesetReport {
            ruleset: code.clone(),
            total_generated: rule_results.iter().map(|r| r.generated).sum(),
            rules: rule_results,
        });

        PipelineResult {
            corporate: counts[0],
            converted: counts[1],
            consolidated: counts[2],
            scenario: scenario_code,
            ruleset: ruleset_code,
            ruleset_report,
        }
    };
    Ok(Json(result))
}

/// POST /api/reset — reset complet : DROP + CREATE schéma + rechargement CSV.
async fn reset_handler(State(state): State<Arc<AppState>>) -> Result<Json<ResetResult>, AppError> {
    let entries = {
        let con = lock_con(&state)?;
        create_schema(&con).map_err(db_err)?; // DROP + CREATE (idempotent)
        load_all(&con, std::path::Path::new(&state.csv_dir)).map_err(db_err)?;
        seed_demo_rules(&con).map_err(db_err)?; // règle + jeu interco (hors CSV)
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM stg_entry", [], |row| row.get(0))
            .map_err(db_err)?;
        n
    };
    Ok(Json(ResetResult {
        status: "ok",
        entries,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Dimensions — registre central (built-in + custom)
//
//  Trois endpoints :
//  - GET    /api/meta/dimensions        : liste toutes les dimensions
//  - POST   /api/meta/dimensions        : crée une dimension custom
//  - DELETE /api/meta/dimensions/{name} : supprime une dimension custom
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `GET /api/meta/dimensions` : une dimension du registre.
#[derive(Serialize)]
struct DimensionInfo {
    name: String,
    category: String,
    custom: bool,
    label: String,
    pilotable: bool,
}

/// Corps accepté par `POST /api/meta/dimensions`.
#[derive(Deserialize)]
struct DimensionBody {
    name: String,
    label: String,
}

impl DimensionInfo {
    fn from_def(d: &dimensions::DimDef) -> Self {
        Self {
            name: d.name.clone(),
            category: format!("{:?}", d.category),
            custom: d.custom,
            label: d.label.clone(),
            pilotable: d.pilotable(),
        }
    }
}

/// GET /api/meta/dimensions — liste toutes les dimensions (built-in + custom).
async fn list_dimensions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DimensionInfo>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        dims.iter().map(DimensionInfo::from_def).collect()
    };
    Ok(Json(rows))
}

/// POST /api/meta/dimensions — crée une dimension custom.
///
/// Valide le nom via `dimensions::is_valid_custom_name` et refuse les
/// doublons (built-in ou déjà présente dans le registre). Répond 201 + la
/// dimension créée.
async fn create_dimension(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DimensionBody>,
) -> Result<(StatusCode, Json<DimensionInfo>), AppError> {
    let info = {
        let con = lock_con(&state)?;
        dimensions::create_custom(&con, &body.name, &body.label).map_err(|e| {
            // Les erreurs de validation sont des `InvalidParameterName` → 400.
            AppError::bad_request(e.to_string())
        })?;
        DimensionInfo {
            name: body.name.clone(),
            category: "Analytical".to_string(),
            custom: true,
            label: body.label.clone(),
            pilotable: true,
        }
    };
    Ok((StatusCode::CREATED, Json(info)))
}

/// DELETE /api/meta/dimensions/{name} — supprime une dimension custom.
async fn delete_dimension(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        match dimensions::delete_custom(&con, &name) {
            Ok(()) => 1,
            Err(e) => {
                // Inexistante → 404 ; autre erreur DuckDB → 500.
                if matches!(e, duckdb::Error::InvalidParameterName(_)) {
                    return Err(AppError::not_found(e.to_string()));
                }
                return Err(db_err(e));
            }
        }
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Règles de consolidation — CRUD + exécution
//
//  Tables : `dim_rule` (bibliothèque), `dim_ruleset` + `dim_ruleset_item` (jeux
//  ordonnés). L'exécution d'un ruleset délègue à `conso_engine::rules::run_ruleset`.
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `GET /api/rules` : résumé d'une règle (sans la définition, qui peut
/// être volumineuse).
#[derive(Serialize)]
struct RuleSummary {
    code: String,
    libelle: Option<String>,
}

/// Réponse `GET /api/rules/{code}` et `POST /api/rules` : règle complète avec
/// définition parsée en JSON.
#[derive(Serialize)]
struct RuleDetail {
    code: String,
    libelle: Option<String>,
    definition: JsonValue,
}

/// Corps accepté par `POST /api/rules` et `PUT /api/rules/{code}` :
/// `definition` peut être un objet JSON (re-sérialisé en TEXT) ou une chaîne
/// (utilisée telle quelle si déjà du JSON valide).
#[derive(Deserialize)]
struct RuleBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    definition: JsonValue,
}

/// Item ordonné d'un ruleset, joint à `dim_rule.libelle` quand la règle existe.
#[derive(Serialize)]
struct RulesetItemOut {
    ordre: i64,
    rule_code: String,
    #[serde(default)]
    libelle: Option<String>,
}

/// Réponse `GET /api/rulesets/{code}` : jeu + items ordonnés.
#[derive(Serialize)]
struct RulesetDetail {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    items: Vec<RulesetItemOut>,
}

/// Résumé d'un ruleset (sans items).
#[derive(Serialize)]
struct RulesetSummary {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
}

/// Corps accepté par `POST /api/rulesets` et `PUT /api/rulesets/{code}`.
#[derive(Deserialize)]
struct RulesetBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    #[serde(default)]
    items: Vec<RulesetItemIn>,
}

#[derive(Deserialize)]
struct RulesetItemIn {
    ordre: i64,
    rule_code: String,
}

/// Sérialise une `JsonValue` en chaîne compacte pour stockage TEXT.
fn definition_to_text(def: &JsonValue) -> Result<String, AppError> {
    serde_json::to_string(def)
        .map_err(|e| AppError::bad_request(format!("définition non sérialisable : {e}")))
}

/// Parse une chaîne TEXT en `JsonValue` (fallback sur la chaîne brute si elle
/// n'est pas du JSON valide — mais on attend du JSON).
fn text_to_definition(s: &str) -> JsonValue {
    serde_json::from_str(s).unwrap_or(JsonValue::String(s.to_string()))
}

/// GET /api/rules — liste toutes les règles (code, libelle).
async fn list_rules(State(state): State<Arc<AppState>>) -> Result<Json<Vec<RuleSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_rule ORDER BY code")
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(RuleSummary {
                    code: row.get(0)?,
                    libelle: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/rules/{code} — détail d'une règle (définition parsée en JSON).
async fn get_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RuleDetail>, AppError> {
    let row = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle, definition FROM dim_rule WHERE code = ?")
            .map_err(db_err)?;
        let mut iter = stmt
            .query_map([&code], |row| {
                let def: Option<String> = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    def,
                ))
            })
            .map_err(db_err)?;
        iter.next()
            .transpose()
            .map_err(db_err)?
            .ok_or_else(|| AppError::not_found(format!("règle {code} introuvable")))?
    };
    let definition = row.2
        .as_deref()
        .map(text_to_definition)
        .unwrap_or(JsonValue::Null);
    Ok(Json(RuleDetail {
        code: row.0,
        libelle: row.1,
        definition,
    }))
}

/// POST /api/rules — crée une règle.
async fn create_rule(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RuleBody>,
) -> Result<(StatusCode, Json<RuleDetail>), AppError> {
    let definition_text = definition_to_text(&body.definition)?;
    let detail = {
        let con = lock_con(&state)?;
        let exists: bool = con
            .query_row(
                "SELECT COUNT(*) > 0 FROM dim_rule WHERE code = ?",
                [&body.code],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if exists {
            return Err(AppError::conflict(format!(
                "règle {} existe déjà",
                body.code
            )));
        }
        validate_definition(&con, &definition_text).map_err(AppError::bad_request)?;
        con.execute(
            "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
            params_from_iter(vec![
                DbValue::Text(body.code.clone()),
                body.libelle.clone().map(DbValue::Text).unwrap_or(DbValue::Null),
                DbValue::Text(definition_text),
            ]),
        )
        .map_err(db_err)?;
        RuleDetail {
            code: body.code.clone(),
            libelle: body.libelle.clone(),
            definition: body.definition.clone(),
        }
    };
    Ok((StatusCode::CREATED, Json(detail)))
}

/// PUT /api/rules/{code} — modifie libelle et/ou definition d'une règle.
async fn update_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RuleBody>,
) -> Result<Json<RuleDetail>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request(
            "le `code` du body ne correspond pas à l'URL",
        ));
    }
    let definition_text = definition_to_text(&body.definition)?;
    let detail = {
        let con = lock_con(&state)?;
        validate_definition(&con, &definition_text).map_err(AppError::bad_request)?;
        let n = con
            .execute(
                "UPDATE dim_rule SET libelle = ?, definition = ? WHERE code = ?",
                params_from_iter(vec![
                    body.libelle.clone().map(DbValue::Text).unwrap_or(DbValue::Null),
                    DbValue::Text(definition_text),
                    DbValue::Text(code.clone()),
                ]),
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(AppError::not_found(format!("règle {code} introuvable")));
        }
        RuleDetail {
            code: body.code.clone(),
            libelle: body.libelle.clone(),
            definition: body.definition.clone(),
        }
    };
    Ok(Json(detail))
}

/// DELETE /api/rules/{code} — supprime une règle.
///
/// Si la règle est référencée par un `dim_ruleset_item`, on renvoie 409
/// (Conflict) avec un message listant les rulesets concernés ; l'utilisateur
/// doit d'abord retirer la règle des jeux qui la référencent.
async fn delete_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        // Vérifier les références avant suppression.
        let referees: Vec<String> = {
            let mut stmt = con
                .prepare(
                    "SELECT DISTINCT ruleset_code FROM dim_ruleset_item WHERE rule_code = ? \
                     ORDER BY ruleset_code",
                )
                .map_err(db_err)?;
            let iter = stmt
                .query_map([&code], |row| row.get::<_, String>(0))
                .map_err(db_err)?;
            let mut v = Vec::new();
            for r in iter {
                v.push(r.map_err(db_err)?);
            }
            v
        };
        if !referees.is_empty() {
            return Err(AppError::conflict(format!(
                "la règle {code} est référencée par les rulesets : {}",
                referees.join(", ")
            )));
        }
        con.execute("DELETE FROM dim_rule WHERE code = ?", [&code])
            .map_err(db_err)?
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

/// GET /api/rulesets — liste tous les rulesets (sans items).
async fn list_rulesets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RulesetSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_ruleset ORDER BY code")
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(RulesetSummary {
                    code: row.get(0)?,
                    libelle: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/rulesets/{code} — détail + items ordonnés (avec libellés des règles).
async fn get_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RulesetDetail>, AppError> {
    let detail = {
        let con = lock_con(&state)?;
        build_ruleset_detail(&con, &code)?
    };
    Ok(Json(detail))
}

/// POST /api/rulesets — crée un ruleset avec ses items.
async fn create_ruleset(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RulesetBody>,
) -> Result<(StatusCode, Json<RulesetDetail>), AppError> {
    let detail = {
        let con = lock_con(&state)?;
        let exists: bool = con
            .query_row(
                "SELECT COUNT(*) > 0 FROM dim_ruleset WHERE code = ?",
                [&body.code],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if exists {
            return Err(AppError::conflict(format!(
                "ruleset {} existe déjà",
                body.code
            )));
        }
        con.execute(
            "INSERT INTO dim_ruleset (code, libelle) VALUES (?, ?)",
            params_from_iter(vec![
                DbValue::Text(body.code.clone()),
                body.libelle.clone().map(DbValue::Text).unwrap_or(DbValue::Null),
            ]),
        )
        .map_err(db_err)?;
        insert_ruleset_items(&con, &body.code, &body.items)?;
        build_ruleset_detail(&con, &body.code)?
    };
    Ok((StatusCode::CREATED, Json(detail)))
}

/// PUT /api/rulesets/{code} — modifie un ruleset (libellé + réordonnancement
/// complet des items).
async fn update_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RulesetBody>,
) -> Result<Json<RulesetDetail>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request(
            "le `code` du body ne correspond pas à l'URL",
        ));
    }
    let detail = {
        let con = lock_con(&state)?;
        let n = con
            .execute(
                "UPDATE dim_ruleset SET libelle = ? WHERE code = ?",
                params_from_iter(vec![
                    body.libelle.clone().map(DbValue::Text).unwrap_or(DbValue::Null),
                    DbValue::Text(code.clone()),
                ]),
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(AppError::not_found(format!("ruleset {code} introuvable")));
        }
        // Réordonnancement complet : on supprime tous les items puis on
        // ré-insère ceux du body.
        con.execute(
            "DELETE FROM dim_ruleset_item WHERE ruleset_code = ?",
            [&code],
        )
        .map_err(db_err)?;
        insert_ruleset_items(&con, &code, &body.items)?;
        build_ruleset_detail(&con, &code)?
    };
    Ok(Json(detail))
}

/// DELETE /api/rulesets/{code} — supprime le ruleset + ses items.
async fn delete_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        con.execute(
            "DELETE FROM dim_ruleset_item WHERE ruleset_code = ?",
            [&code],
        )
        .map_err(db_err)?;
        let n = con
            .execute("DELETE FROM dim_ruleset WHERE code = ?", [&code])
            .map_err(db_err)?;
        n
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers rulesets (locales au module binaire)
// ─────────────────────────────────────────────────────────────────────────────

/// Insère les items d'un ruleset dans l'ordre donné.
fn insert_ruleset_items(
    con: &Connection,
    ruleset_code: &str,
    items: &[RulesetItemIn],
) -> Result<(), AppError> {
    for item in items {
        con.execute(
            "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) VALUES (?, ?, ?)",
            params_from_iter(vec![
                DbValue::Text(ruleset_code.to_string()),
                DbValue::BigInt(item.ordre),
                DbValue::Text(item.rule_code.clone()),
            ]),
        )
        .map_err(db_err)?;
    }
    Ok(())
}

/// Reconstruit un `RulesetDetail` depuis la base (après insert/update).
///
/// Renvoie `AppError::not_found` si le ruleset n'existe pas.
fn build_ruleset_detail(con: &Connection, code: &str) -> Result<RulesetDetail, AppError> {
    let header = {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_ruleset WHERE code = ?")
            .map_err(db_err)?;
        let mut iter = stmt
            .query_map([code], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(db_err)?;
        iter.next()
            .transpose()
            .map_err(db_err)?
            .ok_or_else(|| AppError::not_found(format!("ruleset {code} introuvable")))?
    };
    let mut stmt = con
        .prepare(
            "SELECT i.ordre, i.rule_code, r.libelle \
             FROM dim_ruleset_item i \
             LEFT JOIN dim_rule r ON r.code = i.rule_code \
             WHERE i.ruleset_code = ? \
             ORDER BY i.ordre",
        )
        .map_err(db_err)?;
    let iter = stmt
        .query_map([code], |row| {
            Ok(RulesetItemOut {
                ordre: row.get(0)?,
                rule_code: row.get(1)?,
                libelle: row.get(2)?,
            })
        })
        .map_err(db_err)?;
    let mut items = Vec::new();
    for r in iter {
        items.push(r.map_err(db_err)?);
    }
    Ok(RulesetDetail {
        code: header.0,
        libelle: header.1,
        items,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Démarrage
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // --- Configuration via env (pas de clap pour un prototype) ---
    let port: u16 = std::env::var("CONSO_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    let db_path = std::env::var("CONSO_DB_PATH").unwrap_or_else(|_| "conso.duckdb".to_string());
    let csv_dir = std::env::var("CONSO_CSV_DIR").unwrap_or_else(|_| "data".to_string());
    let web_dir = std::env::var("CONSO_WEB_DIR").unwrap_or_else(|_| "../../web/dist".to_string());

    println!("▶ Ouverture de DuckDB ({db_path})…");
    let con = Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("✗ Impossible d'ouvrir DuckDB ({db_path}) : {e}"));

    // Schéma + chargement initial des CSV.
    //
    // IMPORTANT : on ne recharge les CSV que si la base n'est pas déjà
    // initialisée. Sinon, `create_schema` (DROP de toutes les tables) +
    // `load_all` effaceraient à chaque démarrage les éditions de master data
    // faites via l'UI (périmètre, taux, entités…). La base DuckDB est ainsi la
    // source de vérité entre redémarrages.
    //
    // Pour forcer un rechargement complet (ex. après évolution du schéma) :
    //   - POST /api/reset (à chaud), ou
    //   - CONSO_FORCE_RESEED=1 au démarrage.
    let force_reseed = std::env::var("CONSO_FORCE_RESEED").unwrap_or_default() == "1";
    let schema_exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = 'fact_entry'",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    if schema_exists && !force_reseed {
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM fact_entry", [], |r| r.get(0))
            .unwrap_or(0);
        println!(
            "   Base déjà initialisée ({n} lignes dans fact_entry) — CSV non réimportés, éditions UI préservées."
        );
        println!("   (Pour forcer le rechargement : POST /api/reset ou CONSO_FORCE_RESEED=1)");
    } else {
        if force_reseed {
            println!("   CONSO_FORCE_RESEED=1 — rechargement complet demandé.");
        }
        println!("   Initialisation : création du schéma + import CSV…");
        create_schema(&con).expect("✗ create_schema");
        load_all(&con, std::path::Path::new(&csv_dir)).expect("✗ load_all");
        seed_demo_rules(&con).expect("✗ seed_demo_rules"); // règle + jeu interco (hors CSV)

        // Pipeline initial pour exposer des données exploitables dès le démarrage.
        // En cas d'échec, on continue : l'utilisateur peut POST /api/run.
        // On sélectionne le 1er scénario 'ouvert' (post-SPEC_SCENARIO_V2 : plus
        // de ConvertParams::default(), les params viennent du scénario).
        let initial_scenario: Option<String> = con
            .query_row(
                "SELECT code FROM dim_scenario \
                 WHERE statut = 'ouvert' OR statut IS NULL \
                 ORDER BY code LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok();
        match initial_scenario {
            Some(code) => match ConvertParams::load_params(&con, &code) {
                Ok(params) => match run_pipeline(&con, &params) {
                    Ok(report) => {
                        let counts = report.counts();
                        println!(
                            "   Pipeline initial (scénario {code}) : corporate={}, converted={}, consolidated={}",
                            counts[0], counts[1], counts[2]
                        );
                    }
                    Err(e) => {
                        eprintln!("⚠ Pipeline initial échoué (le serveur démarre quand même) : {e}");
                    }
                },
                Err(e) => {
                    eprintln!(
                        "⚠ load_params initial échoué pour {code} (le serveur démarre quand même) : {e}"
                    );
                }
            },
            None => {
                eprintln!("⚠ Aucun scénario 'ouvert' trouvé — pipeline initial sauté.");
            }
        }
    }

    let state = Arc::new(AppState {
        con: Mutex::new(con),
        csv_dir,
    });

    // CORS permissif pour le prototype : autorise le frontend React (Vite,
    // localhost:5173) et tout autre origine. À restreindre en production.
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    // Servir le frontend buildé en statique (SPA : fallback sur index.html pour
    // toutes les routes non-API). Si le répertoire n'existe pas, seule l'API reste
    // exposée — utile en dev (Vite sert le frontend sur :5173 avec proxy /api).
    let serve_dir = ServeDir::new(&web_dir)
        .not_found_service(ServeFile::new(format!("{web_dir}/index.html")));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/levels", get(get_levels))
        .route("/api/bilan", get(get_bilan))
        .route("/api/compte-resultat", get(get_compte_resultat))
        .route("/api/entries", get(get_entries))
        .route("/api/run", post(run_pipeline_handler))
        .route("/api/reset", post(reset_handler))
        .route("/api/scenarios", get(list_scenarios))
        // Dimensions — registre central (built-in + custom)
        .route(
            "/api/meta/dimensions",
            get(list_dimensions).post(create_dimension),
        )
        .route(
            "/api/meta/dimensions/{name}",
            delete(delete_dimension),
        )
        // Règles de consolidation (CRUD). L'exécution des règles passe par le
        // pipeline (/api/run applique le ruleset du scénario), pas par une route
        // standalone.
        .route(
            "/api/rules",
            get(list_rules).post(create_rule),
        )
        .route(
            "/api/rules/{code}",
            get(get_rule).put(update_rule).delete(delete_rule),
        )
        .route(
            "/api/rulesets",
            get(list_rulesets).post(create_ruleset),
        )
        .route(
            "/api/rulesets/{code}",
            get(get_ruleset).put(update_ruleset).delete(delete_ruleset),
        )
        .merge(masterdata::router())
        .merge(characteristics::router())
        .merge(import::router())
        .merge(export::router())
        .fallback_service(serve_dir)
        .layer(cors)
        .with_state(state);

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| panic!("✗ bind 0.0.0.0:{port} : {e}"));
    println!(
        "▶ conso-server en écoute sur http://localhost:{port} (frontend servi depuis {web_dir})"
    );
    axum::serve(listener, app).await.unwrap();
}
