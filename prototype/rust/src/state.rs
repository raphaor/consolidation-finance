//! État applicatif partagé et helpers d'erreur pour le serveur Axum.
//!
//! Centralise `AppState` (connexion DuckDB + répertoire CSV) afin que les
//! modules de routes (`masterdata`, `import`) et le binaire `conso-server`
//! partagent le même type d'état. Définit également `AppError` qui porte un
//! code HTTP — les handlers peuvent ainsi renvoyer 400/404/409 en plus du 500
//! par défaut (erreurs DuckDB).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use duckdb::Connection;
use std::sync::Mutex;

/// État applicatif partagé entre les handlers via `Arc`.
///
/// La connexion DuckDB est `Send` mais pas `Sync` : on la protège par un
/// `std::sync::Mutex` standard. Les requêtes SQL sont synchrones et courtes,
/// et l'on ne tient jamais le lock à travers un `.await`.
pub struct AppState {
    pub con: Mutex<Connection>,
    pub csv_dir: String,
    /// Chemin optionnel d'un paquet JSON servant de seed au boot sur base vierge
    /// et au `POST /api/reset` (T3 — `CONSO_SEED_JSON`). Quand `None`, le reset
    /// laisse la base vide (schéma seul) ; l'utilisateur enchaîne avec
    /// `POST /api/import/all`.
    pub seed_json: Option<String>,
}

/// Erreur applicative sérialisée en JSON `{ "error": "<message>" }` avec un
/// code HTTP explicite (`(StatusCode, String)`).
#[derive(Debug)]
pub struct AppError(pub StatusCode, pub String);

impl AppError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, msg.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

/// Convertit une erreur `Display` en `AppError` HTTP 500.
pub fn db_err<E: std::fmt::Display>(e: E) -> AppError {
    AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Verrouille le mutex en mappant l'erreur Poison vers `AppError` (HTTP 500).
pub fn lock_con(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
    state
        .con
        .lock()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
