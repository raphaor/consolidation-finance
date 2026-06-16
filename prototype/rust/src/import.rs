//! Import CSV via upload multipart (champ `file`).
//!
//! Deux endpoints :
//! - `POST /api/import/entries` : ajoute (append) dans `stg_entry` au format
//!   EDB (`Scenario, Entity, Entry_period, Period, Account, Flow, Currency,
//!   Audit_id, Partner*, Share*, Analysis*, Amount`).
//! - `POST /api/import/rates` : upsert dans `sat_exchange_rate`
//!   (`currency_source, period, taux_close, taux_moyen`).
//!
//! Le fichier est écrit dans un temporaire puis chargé via `read_csv_auto`
//! (même pattern que `loader.rs`). Le header est validé avant chargement.

use axum::{
    extract::{Multipart, State},
    routing::post,
    Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::state::{lock_con, AppError, AppState};

fn escape_csv_path(p: &str) -> String {
    p.replace('\'', "''")
}

fn unique_tmp_path(suffix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("conso_import_{pid}_{nanos}.{suffix}"))
}

async fn extract_file_bytes(mut multipart: Multipart) -> Result<Vec<u8>, AppError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(format!("multipart illisible : {e}")))?
    {
        if field.name() == Some("file") {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| AppError::bad_request(format!("corps du fichier illisible : {e}")))?;
            return Ok(bytes.to_vec());
        }
    }
    Err(AppError::bad_request("champ 'file' manquant"))
}

fn parse_header_line(line: &str) -> Vec<String> {
    line.trim_matches(['\u{feff}', '\r', '\n', ' '])
        .split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .collect()
}

fn require_columns(header: &[String], required: &[&str]) -> Result<(), AppError> {
    for col in required {
        if !header.iter().any(|h| h == col) {
            return Err(AppError::bad_request(format!(
                "colonne absente du header : {col}"
            )));
        }
    }
    Ok(())
}

fn split_first_line(bytes: &[u8]) -> Result<(String, usize), AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| AppError::bad_request(format!("fichier non UTF-8 : {e}")))?;
    let end = text
        .find('\n')
        .unwrap_or(text.len());
    let header = text[..end].trim_end_matches('\r').to_string();
    Ok((header, end))
}

async fn import_entries(
    State(state): State<Arc<AppState>>,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let bytes = extract_file_bytes(multipart).await?;
    if bytes.is_empty() {
        return Err(AppError::bad_request("fichier vide"));
    }
    let (header_line, _header_end) = split_first_line(&bytes)?;
    let header = parse_header_line(&header_line);
    let required = &[
        "scenario",
        "entity",
        "entry_period",
        "period",
        "account",
        "flow",
        "currency",
        "audit_id",
        "amount",
    ];
    require_columns(&header, required)?;
    let has = |name: &str| header.iter().any(|h| h == name);
    let partner = if has("partner") { "partner" } else { "NULL" };
    let share = if has("share") { "share" } else { "NULL" };
    let analysis = if has("analysis") { "analysis" } else { "NULL" };

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes).map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());
    let sql = format!(
        "INSERT INTO stg_entry \
         (scenario, entity, entry_period, period, account, flow, currency, \
          partner, share, analysis, audit_id, amount) \
         SELECT scenario, entity, entry_period, period, account, flow, currency, \
                {partner}, {share}, {analysis}, audit_id, amount \
         FROM read_csv_auto('{path}', header=true)"
    );

    let imported = {
        let con = lock_con(&state)?;
        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!("lecture CSV impossible : {e}")));
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    Ok(Json(json!({ "imported": imported })))
}

async fn import_rates(
    State(state): State<Arc<AppState>>,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let bytes = extract_file_bytes(multipart).await?;
    if bytes.is_empty() {
        return Err(AppError::bad_request("fichier vide"));
    }
    let (header_line, _) = split_first_line(&bytes)?;
    let header = parse_header_line(&header_line);
    require_columns(
        &header,
        &["currency_source", "period", "taux_close", "taux_moyen"],
    )?;

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes).map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());
    let sql = format!(
        "INSERT INTO sat_exchange_rate \
         (currency_source, period, taux_close, taux_moyen) \
         SELECT currency_source, period, taux_close, taux_moyen \
         FROM read_csv_auto('{path}', header=true) \
         ON CONFLICT(currency_source, period) DO UPDATE SET \
            taux_close = excluded.taux_close, \
            taux_moyen = excluded.taux_moyen"
    );

    let imported = {
        let con = lock_con(&state)?;
        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!("lecture CSV impossible : {e}")));
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    Ok(Json(json!({ "imported": imported })))
}

async fn import_perimeter(
    State(state): State<Arc<AppState>>,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    let bytes = extract_file_bytes(multipart).await?;
    if bytes.is_empty() {
        return Err(AppError::bad_request("fichier vide"));
    }
    let (header_line, _) = split_first_line(&bytes)?;
    let header = parse_header_line(&header_line);
    require_columns(
        &header,
        &[
            "entity",
            "scenario",
            "period",
            "methode",
            "pct_interet",
            "pct_integration",
            "entree",
            "sortie",
        ],
    )?;

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes).map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());
    let sql = format!(
        "INSERT INTO sat_perimeter \
         (entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie) \
         SELECT entity, scenario, period, methode, pct_interet, pct_integration, \
                CAST(entree AS BOOLEAN), CAST(sortie AS BOOLEAN) \
         FROM read_csv_auto('{path}', header=true) \
         ON CONFLICT(entity, scenario, period) DO UPDATE SET \
            methode = excluded.methode, \
            pct_interet = excluded.pct_interet, \
            pct_integration = excluded.pct_integration, \
            entree = excluded.entree, \
            sortie = excluded.sortie"
    );

    let imported = {
        let con = lock_con(&state)?;
        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!("lecture CSV impossible : {e}")));
            }
        }
    };
    let _ = std::fs::remove_file(&tmp);
    Ok(Json(json!({ "imported": imported })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/import/entries", post(import_entries))
        .route("/api/import/rates", post(import_rates))
        .route("/api/import/perimeter", post(import_perimeter))
}
