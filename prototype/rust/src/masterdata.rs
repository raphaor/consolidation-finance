//! CRUD générique sur les 10 tables master data (8 dimensions + 2 satellites).
//!
//! Expose `router()` qui monte les routes `/api/md/{table}` (GET/POST/PUT/DELETE)
//! sur le serveur Axum. La table est validée contre une whitelist statique
//! (`TABLES`) : aucun nom de table ni de colonne n'est interpolé depuis
//! l'utilisateur — seules les valeurs passent par des `?` paramétrés.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use duckdb::{params_from_iter, types::Value as DbValue, types::ValueRef, Row};
use serde_json::{Map, Value as JsonValue};
use std::sync::Arc;

use crate::state::{db_err, lock_con, AppError, AppState};

struct TableDef {
    api_name: &'static str,
    sql_name: &'static str,
    columns: &'static [&'static str],
    pk: &'static [&'static str],
}

const TABLES: &[TableDef] = &[
    TableDef {
        api_name: "scenarios",
        sql_name: "dim_scenario",
        columns: &["code", "libelle", "type", "statut"],
        pk: &["code"],
    },
    TableDef {
        api_name: "entities",
        sql_name: "dim_entity",
        columns: &["code", "libelle", "devise_fonctionnelle", "entite_parent", "statut"],
        pk: &["code"],
    },
    TableDef {
        api_name: "periods",
        sql_name: "dim_period",
        columns: &["code", "libelle", "type", "date_debut", "date_fin", "statut"],
        pk: &["code"],
    },
    TableDef {
        api_name: "accounts",
        sql_name: "dim_account",
        columns: &["code", "libelle", "classe", "sous_classe", "technical_grouping", "compte_parent"],
        pk: &["code"],
    },
    TableDef {
        api_name: "sous_classes",
        sql_name: "dim_sous_classe",
        columns: &["code", "libelle", "classe"],
        pk: &["code"],
    },
    TableDef {
        api_name: "flows",
        sql_name: "dim_flow",
        columns: &["code", "libelle", "taux_conversion", "flux_ecart", "flux_de_report"],
        pk: &["code"],
    },
    TableDef {
        api_name: "currencies",
        sql_name: "dim_currency",
        columns: &["code_iso", "libelle", "decimales"],
        pk: &["code_iso"],
    },
    TableDef {
        api_name: "natures",
        sql_name: "dim_nature",
        columns: &["code", "libelle", "rules"],
        pk: &["code"],
    },
    TableDef {
        api_name: "perimeter",
        sql_name: "sat_perimeter",
        columns: &[
            "entity",
            "scenario",
            "period",
            "methode",
            "pct_interet",
            "pct_integration",
            "entree",
            "sortie",
        ],
        pk: &["entity", "scenario", "period"],
    },
    TableDef {
        api_name: "rates",
        sql_name: "sat_exchange_rate",
        columns: &["currency_source", "period", "taux_close", "taux_moyen"],
        pk: &["currency_source", "period"],
    },
];

fn find_table(api: &str) -> Option<&'static TableDef> {
    TABLES.iter().find(|t| t.api_name == api)
}

fn quote_ident(col: &str) -> String {
    format!("\"{col}\"")
}

fn value_ref_to_json(v: ValueRef) -> JsonValue {
    match v {
        ValueRef::Null => JsonValue::Null,
        ValueRef::Boolean(b) => JsonValue::Bool(b),
        ValueRef::TinyInt(i) => JsonValue::Number(i.into()),
        ValueRef::SmallInt(i) => JsonValue::Number(i.into()),
        ValueRef::Int(i) => JsonValue::Number(i.into()),
        ValueRef::BigInt(i) => JsonValue::Number(i.into()),
        ValueRef::HugeInt(i) => JsonValue::String(i.to_string()),
        ValueRef::UTinyInt(i) => JsonValue::Number(i.into()),
        ValueRef::USmallInt(i) => JsonValue::Number(i.into()),
        ValueRef::UInt(i) => serde_json::json!(i),
        ValueRef::UBigInt(i) => serde_json::json!(i),
        ValueRef::Float(f) => serde_json::json!(f),
        ValueRef::Double(f) => serde_json::json!(f),
        ValueRef::Decimal(d) => match d.to_string().parse::<f64>() {
            Ok(f) => serde_json::json!(f),
            Err(_) => JsonValue::Null,
        },
        ValueRef::Text(t) => match std::str::from_utf8(t) {
            Ok(s) => JsonValue::String(s.to_string()),
            Err(_) => JsonValue::Null,
        },
        ValueRef::Date32(days) => JsonValue::String(date32_to_iso(days)),
        ValueRef::Blob(_) => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

fn date32_to_iso(days: i32) -> String {
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

fn json_to_db_value(v: &JsonValue) -> DbValue {
    match v {
        JsonValue::Null => DbValue::Null,
        JsonValue::Bool(b) => DbValue::Boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                DbValue::BigInt(i)
            } else if let Some(u) = n.as_u64() {
                DbValue::UBigInt(u)
            } else if let Some(f) = n.as_f64() {
                DbValue::Double(f)
            } else {
                DbValue::Null
            }
        }
        JsonValue::String(s) => DbValue::Text(s.clone()),
        _ => DbValue::Null,
    }
}

fn row_to_json(row: &Row, names: &[String]) -> Result<JsonValue, duckdb::Error> {
    let mut obj = Map::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let v = row.get_ref(i)?;
        obj.insert(name.clone(), value_ref_to_json(v));
    }
    Ok(JsonValue::Object(obj))
}

fn run_query(
    con: &duckdb::Connection,
    sql: &str,
    params: Vec<DbValue>,
) -> Result<Vec<JsonValue>, AppError> {
    let mut stmt = con.prepare(sql).map_err(db_err)?;
    let mut rows = stmt.query(params_from_iter(params)).map_err(db_err)?;
    let names = {
        let s = rows
            .as_ref()
            .ok_or_else(|| AppError(StatusCode::INTERNAL_SERVER_ERROR, "stmt évanoui".into()))?;
        s.column_names()
    };
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(db_err)? {
        out.push(row_to_json(row, &names).map_err(db_err)?);
    }
    Ok(out)
}

fn select_all(def: &TableDef, con: &duckdb::Connection) -> Result<Vec<JsonValue>, AppError> {
    let cols = def
        .columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT {cols} FROM {}", def.sql_name);
    run_query(con, &sql, Vec::new())
}

fn fetch_one(
    def: &TableDef,
    pk_vals: &[(String, JsonValue)],
    con: &duckdb::Connection,
) -> Result<Option<JsonValue>, AppError> {
    let cols = def
        .columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let where_clause = def
        .pk
        .iter()
        .map(|c| format!("{} = ?", quote_ident(c)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT {cols} FROM {} WHERE {where_clause}", def.sql_name);
    let params: Vec<DbValue> = pk_vals.iter().map(|(_, v)| json_to_db_value(v)).collect();
    let rows = run_query(con, &sql, params)?;
    Ok(rows.into_iter().next())
}

/// Rejette les champs JSON qui ne correspondent à aucune colonne connue.
/// Évite la perte silencieuse de données (ex: `label` au lieu de `libelle`).
fn reject_unknown_fields(def: &TableDef, obj: &Map<String, JsonValue>) -> Result<(), AppError> {
    let known: std::collections::HashSet<&str> = def.columns.iter().copied().collect();
    let unknown: Vec<&str> = obj
        .keys()
        .filter(|k| !known.contains(k.as_str()))
        .map(|s| s.as_str())
        .collect();
    if !unknown.is_empty() {
        return Err(AppError::bad_request(format!(
            "champs inconnus : {}. Colonnes valides : {}",
            unknown.join(", "),
            def.columns.join(", ")
        )));
    }
    Ok(())
}

fn pk_from_body(def: &TableDef, body: &JsonValue) -> Result<Vec<(String, JsonValue)>, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?;
    let mut out = Vec::with_capacity(def.pk.len());
    for col in def.pk {
        let val = obj.get(*col).cloned().ok_or_else(|| {
            AppError::bad_request(format!("colonne PK manquante : {col}"))
        })?;
        if val.is_null() {
            return Err(AppError::bad_request(format!("colonne PK nulle : {col}")));
        }
        out.push((col.to_string(), val));
    }
    Ok(out)
}

async fn list(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let def = find_table(&table)
        .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
    let rows = {
        let con = lock_con(&state)?;
        select_all(def, &con)?
    };
    Ok(Json(rows))
}

async fn create(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<JsonValue>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let def = find_table(&table)
        .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?
        .clone();

    reject_unknown_fields(def, &obj)?;

    let result = {
        let con = lock_con(&state)?;
        let pk_vals = pk_from_body(def, &JsonValue::Object(obj.clone()))?;
        if fetch_one(def, &pk_vals, &con)?.is_some() {
            return Err(AppError::conflict("déjà existant"));
        }
        let mut cols = Vec::new();
        let mut vals: Vec<DbValue> = Vec::new();
        for col in def.columns {
            if let Some(v) = obj.get(*col) {
                cols.push(quote_ident(col));
                vals.push(json_to_db_value(v));
            }
        }
        if cols.is_empty() {
            return Err(AppError::bad_request("aucune colonne fournie"));
        }
        let placeholders = cols.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            def.sql_name,
            cols.join(", "),
            placeholders
        );
        con.execute(&sql, params_from_iter(vals)).map_err(db_err)?;
        fetch_one(def, &pk_vals, &con)?
    };
    match result {
        Some(row) => Ok((StatusCode::CREATED, Json(row))),
        None => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ligne non retrouvée après insertion".into(),
        )),
    }
}

async fn update(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let def = find_table(&table)
        .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?
        .clone();

    reject_unknown_fields(def, &obj)?;

    let result = {
        let con = lock_con(&state)?;
        let pk_vals = pk_from_body(def, &JsonValue::Object(obj.clone()))?;
        if fetch_one(def, &pk_vals, &con)?.is_none() {
            return Err(AppError::not_found("introuvable"));
        }
        let mut sets = Vec::new();
        let mut vals: Vec<DbValue> = Vec::new();
        for col in def.columns {
            if def.pk.iter().any(|p| *p == *col) {
                continue;
            }
            if let Some(v) = obj.get(*col) {
                sets.push(format!("{} = ?", quote_ident(col)));
                vals.push(json_to_db_value(v));
            }
        }
        if !sets.is_empty() {
            let where_clause = def
                .pk
                .iter()
                .map(|c| format!("{} = ?", quote_ident(c)))
                .collect::<Vec<_>>()
                .join(" AND ");
            let sql = format!(
                "UPDATE {} SET {} WHERE {}",
                def.sql_name,
                sets.join(", "),
                where_clause
            );
            for (_, v) in &pk_vals {
                vals.push(json_to_db_value(v));
            }
            con.execute(&sql, params_from_iter(vals)).map_err(db_err)?;
        }
        fetch_one(def, &pk_vals, &con)?
    };
    match result {
        Some(row) => Ok(Json(row)),
        None => Err(AppError::not_found("introuvable")),
    }
}

async fn remove(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
    Query(query): Query<Vec<(String, String)>>,
    body_bytes: axum::body::Bytes,
) -> Result<Json<JsonValue>, AppError> {
    let def = find_table(&table)
        .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;

    // PK depuis query string ou body JSON (query string prioritaire)
    let pk_vals: Vec<(String, JsonValue)> = {
        let from_query: Vec<(String, JsonValue)> = def
            .pk
            .iter()
            .filter_map(|col| {
                query
                    .iter()
                    .find(|(k, _)| k == *col)
                    .map(|(_, v)| (col.to_string(), JsonValue::String(v.clone())))
            })
            .collect();

        if from_query.len() == def.pk.len() {
            from_query
        } else if !body_bytes.is_empty() {
            let body: JsonValue = serde_json::from_slice(&body_bytes)
                .map_err(|_| AppError::bad_request("body JSON invalide"))?;
            pk_from_body(def, &body)?
        } else {
            let pk_cols = def.pk.join(", ");
            return Err(AppError::bad_request(format!(
                "PK manquante. Passez-la en query string (?{pk_cols}=valeur) ou dans le body JSON.\n\
                 Exemple : DELETE /api/md/{table}?{pk_cols}=valeur"
            )));
        }
    };

    let deleted = {
        let con = lock_con(&state)?;
        if fetch_one(def, &pk_vals, &con)?.is_none() {
            return Err(AppError::not_found("introuvable"));
        }
        let where_clause = def
            .pk
            .iter()
            .map(|c| format!("{} = ?", quote_ident(c)))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!("DELETE FROM {} WHERE {}", def.sql_name, where_clause);
        let params: Vec<DbValue> = pk_vals.iter().map(|(_, v)| json_to_db_value(v)).collect();
        con.execute(&sql, params_from_iter(params))
            .map_err(db_err)?
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/md/{table}",
        get(list).post(create).put(update).delete(remove),
    )
}
