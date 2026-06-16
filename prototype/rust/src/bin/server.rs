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
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use duckdb::params_from_iter;
use duckdb::types::Value as DbValue;
use duckdb::Connection;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use conso_engine::{
    create_schema, import, load_all, masterdata, money::Money, run_pipeline, ConvertParams,
};
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

/// Ligne `/api/bilan` : montant agrégé par (compte, flux) au niveau demandé.
///
/// `amount` est sérialisé en **nombre** JSON (feature `serde-float` de
/// `rust_decimal`) — le frontend TS attend `amount: 9774.0`, pas une chaîne.
#[derive(Serialize)]
struct BilanRow {
    account: String,
    flow: String,
    amount: Decimal,
}

/// Ligne `/api/entries` : écriture individuelle de la table `fact_entry`.
///
/// Les colonnes `partner`, `share`, `analysis` sont optionnelles (NULL en base)
/// — sérialisées en `null` JSON quand absentes.
#[derive(Serialize)]
struct EntryRow {
    id: i64,
    scenario: String,
    entity: String,
    entry_period: String,
    period: String,
    account: String,
    flow: String,
    currency: String,
    partner: Option<String>,
    share: Option<String>,
    analysis: Option<String>,
    audit_id: Option<String>,
    level: String,
    amount: Decimal,
}

/// Réponse `/api/run` : nombre de lignes produites à chaque étape du pipeline.
#[derive(Serialize)]
struct PipelineResult {
    corporate: usize,
    reclassified: usize,
    converted: usize,
    consolidated: usize,
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
}

fn default_level() -> String {
    "consolidated".to_string()
}

fn default_limit() -> i64 {
    100
}

/// Construit le fragment SQL et les paramètres pour les filtres optionnels
/// `scenario`, `entity`, `entry_period` (exercice clôturé) et `period`
/// (période impactée par l'écriture). Renvoie une chaîne préfixée par " AND ..."
/// prête à concaténer après un WHERE existant.
fn build_filters(
    scenario: &Option<String>,
    entity: &Option<String>,
    entry_period: &Option<String>,
    period: &Option<String>,
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
                     WHEN 'reclassified' THEN 2
                     WHEN 'converted'    THEN 3
                     WHEN 'consolidated' THEN 4
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
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period);
        let sql = format!(
            "SELECT e.account, e.flow, SUM(e.amount) AS amount
             FROM fact_entry e
             JOIN dim_account a ON a.code = e.account
             WHERE e.level = ? AND a.classe = 'bilan' {fsql}
             GROUP BY e.account, e.flow
             ORDER BY e.account, e.flow"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(2)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
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
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period);
        let sql = format!(
            "SELECT e.account, e.flow, SUM(e.amount) AS amount
             FROM fact_entry e
             JOIN dim_account a ON a.code = e.account
             WHERE e.level = ? AND a.classe = 'resultat' {fsql}
             GROUP BY e.account, e.flow
             ORDER BY e.account, e.flow"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(2)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
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

fn map_entry_row(row: &duckdb::Row) -> duckdb::Result<EntryRow> {
    let m: Money = row.get(13)?;
    Ok(EntryRow {
        id: row.get(0)?,
        scenario: row.get(1)?,
        entity: row.get(2)?,
        entry_period: row.get(3)?,
        period: row.get(4)?,
        account: row.get(5)?,
        flow: row.get(6)?,
        currency: row.get(7)?,
        partner: row.get(8)?,
        share: row.get(9)?,
        analysis: row.get(10)?,
        audit_id: row.get(11)?,
        level: row.get(12)?,
        amount: m.into_decimal(),
    })
}

/// GET /api/entries?level=consolidated&limit=100&offset=0 — écritures paginées.
///
/// Niveau spécial `raw` : lit la saisie brute (`stg_entry`) avant pipeline,
/// avec un id synthétique (ROW_NUMBER) pour la cohérence de pagination côté UI.
async fn get_entries(
    Query(q): Query<EntriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<EntryRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let (fsql, fparams) = build_filters(&q.scenario, &q.entity, &q.entry_period, &q.period);
        let (sql, params): (String, Vec<DbValue>) = if q.level == "raw" {
            let where_stg = if fsql.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", fsql.trim_start_matches(" AND "))
            };
            let sql = format!(
                "SELECT * FROM (
                    SELECT ROW_NUMBER() OVER (ORDER BY entity, scenario, period, account, flow, audit_id) AS id,
                           scenario, entity, entry_period, period, account, flow,
                           currency, partner, share, analysis, audit_id,
                           'raw' AS level, amount
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
                "SELECT id, scenario, entity, entry_period, period, account, flow,
                        currency, partner, share, analysis, audit_id, level, amount
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
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), map_entry_row)
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// POST /api/run — déclenche le pipeline 4 étapes et renvoie les comptes.
async fn run_pipeline_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PipelineResult>, AppError> {
    let result = {
        let con = lock_con(&state)?;
        // Vider les résultats du pipeline avant de relancer (sinon accumulation).
        con.execute("DELETE FROM fact_entry", []).map_err(db_err)?;
        let params = ConvertParams::default();
        let counts = run_pipeline(&con, &params).map_err(db_err)?;
        PipelineResult {
            corporate: counts[0],
            reclassified: counts[1],
            converted: counts[2],
            consolidated: counts[3],
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

        // Pipeline initial pour exposer des données exploitables dès le démarrage.
        // En cas d'échec, on continue : l'utilisateur peut POST /api/run.
        match run_pipeline(&con, &ConvertParams::default()) {
            Ok(counts) => {
                println!(
                    "   Pipeline initial : corporate={}, reclassified={}, converted={}, consolidated={}",
                    counts[0], counts[1], counts[2], counts[3]
                );
            }
            Err(e) => {
                eprintln!("⚠ Pipeline initial échoué (le serveur démarre quand même) : {e}");
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
        .merge(masterdata::router())
        .merge(import::router())
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
