//! Import CSV via upload multipart (champ `file`).
//!
//! Deux endpoints :
//! - `POST /api/import/entries` : ajoute (append) dans `stg_entry` au format
//!   EDB (`Phase, Entity, Entry_period, Period, Account, Flow, Currency,
//!   Nature, Audit_id, Partner*, Share*, Analysis*, Amount`).
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

use crate::dimensions;
use crate::references;
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

/// Valide les références du CSV **avant** insertion : pour chaque colonne du
/// fichier qui est une référence (cf. [`crate::references`]), vérifie qu'aucune
/// valeur non-vide n'est absente de la table cible. Anti-jointure sur le fichier
/// temporaire (on ne contrôle que les données entrantes, pas l'existant).
fn validate_csv_references(
    con: &duckdb::Connection,
    target_table: &str,
    header: &[String],
    csv_path: &str,
) -> Result<(), AppError> {
    let mut bad = Vec::new();
    for r in references::references_for(target_table) {
        if !header.iter().any(|h| h == r.column) {
            continue;
        }
        // Colonne de **contrat** : pour une FK migrée en clé technique (ri()),
        // le CSV porte des codes → on valide contre la colonne code de la cible
        // (target_display_column), pas contre l'id de stockage.
        let tcol = r.target_display_column.unwrap_or(r.target_column);
        let sql = format!(
            "SELECT DISTINCT CAST(\"{col}\" AS VARCHAR) \
             FROM read_csv_auto('{path}', header=true, null_padding=true) \
             WHERE \"{col}\" IS NOT NULL AND CAST(\"{col}\" AS VARCHAR) <> '' \
               AND CAST(\"{col}\" AS VARCHAR) NOT IN (SELECT CAST(\"{tcol}\" AS VARCHAR) FROM {ttab}) \
             LIMIT 6",
            col = r.column,
            path = csv_path,
            ttab = r.target_table,
        );
        let err = |e: duckdb::Error| {
            AppError::bad_request(format!("validation référence {} : {e}", r.column))
        };
        let mut stmt = con.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(err)?;
        let mut vals = Vec::new();
        for x in rows {
            vals.push(x.map_err(err)?);
        }
        if !vals.is_empty() {
            bad.push(format!(
                "{} : valeur(s) absente(s) de {} → {}",
                r.column,
                r.target_table,
                vals.join(", ")
            ));
        }
    }
    if !bad.is_empty() {
        return Err(AppError::bad_request(format!(
            "références invalides dans le CSV : {}",
            bad.join(" ; ")
        )));
    }
    Ok(())
}

fn split_first_line(bytes: &[u8]) -> Result<(String, usize), AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| AppError::bad_request(format!("fichier non UTF-8 : {e}")))?;
    let end = text.find('\n').unwrap_or(text.len());
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
    // Colonnes required minimales (Fixed + Active + amount). Les dimensions
    // Analytical (partner, share, analysis, analysis2 et toutes les customs)
    // sont optionnelles : absentes du header, elles seront insérées à NULL.
    require_columns(
        &header,
        &[
            "phase",
            "entity",
            "entry_period",
            "period",
            "account",
            "flow",
            "currency",
            "nature",
            "amount",
        ],
    )?;

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());

    let imported = {
        let con = lock_con(&state)?;

        // Cohérence référentielle des dimensions avant insertion.
        if let Err(e) = validate_csv_references(&con, "stg_entry", &header, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }

        // Colonnes connues de stg_entry = dimensions propagées (built-in +
        // customs) + amount. Pas de `level` dans stg_entry.
        let dims = dimensions::load_all(&con).map_err(|e| {
            AppError::bad_request(format!("registre des dimensions illisible : {e}"))
        })?;
        let mut cols = dimensions::propagated_cols(&dims);
        cols.push("amount");

        // SELECT adaptatif : reprend la colonne si elle est dans le header du
        // CSV, sinon émet `NULL AS <col>`. Généralise le pattern historique
        // (partner/share/analysis) à toutes les dimensions optionnelles et
        // custom — un CSV décrivant une dimension custom est ainsi propagé
        // automatiquement plutôt que silencieusement ignoré.
        let select_list = cols
            .iter()
            .map(|c| {
                if header.iter().any(|h| h == *c) {
                    (*c).to_string()
                } else {
                    format!("NULL AS {c}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let col_list = cols.join(", ");

        let sql = format!(
            "INSERT INTO stg_entry\n\
              ({col_list})\n\
              SELECT {select_list}\n\
              FROM read_csv_auto('{path}', header=true, null_padding=true)"
        );

        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!(
                    "lecture CSV impossible : {e}"
                )));
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
        &[
            "rate_set",
            "currency_source",
            "period",
            "taux_close",
            "taux_moyen",
        ],
    )?;

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());
    // `taux_ouverture` est optionnel dans le CSV importé (rétro-compat) : on
    // l'inclut dans l'INSERT/SELECT seulement s'il est présent, sinon NULL.
    let has_ouverture = header.iter().any(|c| c == "taux_ouverture");
    // `rate_set` est stocké en id (B1) : le CSV porte des codes, résolus par
    // sous-requête corrélée. Le reader est aliasé `src` (cf. loader::build_insert_sql).
    let (cols, select_cols, conflict_set) = if has_ouverture {
        (
            "rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture",
            "(SELECT id FROM dim_rate_set WHERE code = src.rate_set), \
             src.currency_source, src.period, src.taux_close, src.taux_moyen, src.taux_ouverture",
            "taux_close = excluded.taux_close, \
             taux_moyen = excluded.taux_moyen, \
             taux_ouverture = excluded.taux_ouverture",
        )
    } else {
        (
            "rate_set, currency_source, period, taux_close, taux_moyen",
            "(SELECT id FROM dim_rate_set WHERE code = src.rate_set), \
             src.currency_source, src.period, src.taux_close, src.taux_moyen, NULL",
            "taux_close = excluded.taux_close, taux_moyen = excluded.taux_moyen",
        )
    };
    let sql = format!(
        "INSERT INTO sat_exchange_rate ({cols}) \
         SELECT {select_cols} \
         FROM read_csv_auto('{path}', header=true) AS src \
         ON CONFLICT(rate_set, currency_source, period) DO UPDATE SET {conflict_set}"
    );

    let imported = {
        let con = lock_con(&state)?;
        if let Err(e) = validate_csv_references(&con, "sat_exchange_rate", &header, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!(
                    "lecture CSV impossible : {e}"
                )));
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
            "perimeter_set",
            "entity",
            "period",
            "methode",
            "pct_interet",
            "pct_integration",
            "entree",
            "sortie",
        ],
    )?;

    let tmp = unique_tmp_path("csv");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| AppError::bad_request(format!("écriture temp : {e}")))?;
    let path = escape_csv_path(&tmp.display().to_string());
    // `perimeter_set` et `methode` sont stockés en id (B1) : le CSV porte des codes,
    // résolus par sous-requête corrélée.
    let sql = format!(
        "INSERT INTO sat_perimeter \
         (perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie) \
         SELECT (SELECT id FROM dim_perimeter_set WHERE code = src.perimeter_set), \
                src.entity, src.period, \
                (SELECT id FROM dim_method WHERE code = src.methode), \
                src.pct_interet, src.pct_integration, \
                CAST(src.entree AS BOOLEAN), CAST(src.sortie AS BOOLEAN) \
         FROM read_csv_auto('{path}', header=true) AS src \
         ON CONFLICT(perimeter_set, entity, period) DO UPDATE SET \
            methode = excluded.methode, \
            pct_interet = excluded.pct_interet, \
            pct_integration = excluded.pct_integration, \
            entree = excluded.entree, \
            sortie = excluded.sortie"
    );

    let imported = {
        let con = lock_con(&state)?;
        if let Err(e) = validate_csv_references(&con, "sat_perimeter", &header, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        match con.execute(&sql, []) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(AppError::bad_request(format!(
                    "lecture CSV impossible : {e}"
                )));
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
