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
//! - `CONSO_PORT`     : port d'écoute (défaut : 3000).
//! - `CONSO_DB_PATH`  : chemin du fichier DuckDB (défaut : `conso.duckdb`).
//! - `CONSO_CSV_DIR`  : répertoire contenant les CSV (défaut : `data`).

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use duckdb::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

use conso_engine::{create_schema, load_all, run_pipeline, ConvertParams};

// ─────────────────────────────────────────────────────────────────────────────
//  État partagé
// ─────────────────────────────────────────────────────────────────────────────

/// État applicatif partagé entre les handlers via `Arc`.
///
/// La connexion DuckDB est `Send` mais pas `Sync` : on la protège par un
/// `std::sync::Mutex` standard. Les requêtes SQL sont synchrones et courtes,
/// et l'on ne tient jamais le lock à travers un `.await` — pas besoin d'un
/// `tokio::sync::Mutex` pour ce prototype mono-utilisateur.
struct AppState {
    /// Connexion DuckDB partagée.
    con: Mutex<Connection>,
    /// Répertoire contenant les CSV (pour `/api/reset`).
    csv_dir: String,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Erreurs
// ─────────────────────────────────────────────────────────────────────────────

/// Erreur applicative sérialisée en JSON `{ "error": "<message>" }` (HTTP 500).
struct AppError(String);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": self.0 })),
        )
            .into_response()
    }
}

/// Convertit n'importe quelle erreur `Display` en `AppError`.
fn db_err<E: std::fmt::Display>(e: E) -> AppError {
    AppError(e.to_string())
}

/// Verrouille le mutex en mappant l'erreur Poison vers `AppError`.
fn lock_con(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
    state.con.lock().map_err(|e| AppError(e.to_string()))
}

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
#[derive(Serialize)]
struct BilanRow {
    account: String,
    flow: String,
    amount: f64,
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
    amount: f64,
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
}

#[derive(Deserialize)]
struct EntriesQuery {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_level() -> String {
    "consolidated".to_string()
}

fn default_limit() -> i64 {
    100
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
/// Exécute la même requête SQL que `report::bilan_par_flux` (format long) mais
/// renvoie du JSON au lieu d'imprimer un tableau.
async fn get_bilan(
    Query(q): Query<BilanQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BilanRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                "SELECT account, flow, SUM(amount) AS amount
                 FROM fact_entry
                 WHERE level = ?
                 GROUP BY account, flow
                 ORDER BY account, flow",
            )
            .map_err(db_err)?;
        let iter = stmt
            .query_map([&q.level], |row| {
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
                    amount: row.get(2)?,
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
async fn get_entries(
    Query(q): Query<EntriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<EntryRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                "SELECT id, scenario, entity, entry_period, period, account, flow,
                       currency, partner, share, analysis, audit_id, level, amount
                 FROM fact_entry
                 WHERE level = ?
                 ORDER BY id
                 LIMIT ? OFFSET ?",
            )
            .map_err(db_err)?;
        // Cast explicite en BIGINT : DuckDB est parfois tatillon sur les
        // paramètres bindés dans LIMIT/OFFSET.
        let iter = stmt
            .query_map(
                duckdb::params![q.level.as_str(), q.limit as i64, q.offset as i64],
                |row| {
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
                        amount: row.get(13)?,
                    })
                },
            )
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

    println!("▶ Ouverture de DuckDB ({db_path})…");
    let con = Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("✗ Impossible d'ouvrir DuckDB ({db_path}) : {e}"));

    // Schéma + chargement initial des CSV.
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

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/levels", get(get_levels))
        .route("/api/bilan", get(get_bilan))
        .route("/api/entries", get(get_entries))
        .route("/api/run", post(run_pipeline_handler))
        .route("/api/reset", post(reset_handler))
        .layer(cors)
        .with_state(state);

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| panic!("✗ bind 0.0.0.0:{port} : {e}"));
    println!("▶ conso-server en écoute sur http://localhost:{port}");
    axum::serve(listener, app).await.unwrap();
}
