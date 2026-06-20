//! CRUD générique sur les tables master data (dimensions + satellites +
//! catalogues scénario v2).
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

use crate::references;
use crate::state::{db_err, lock_con, AppError, AppState};

struct TableDef {
    api_name: &'static str,
    sql_name: &'static str,
    columns: &'static [&'static str],
    pk: &'static [&'static str],
}

const TABLES: &[TableDef] = &[
    // --- Catalogues v2 (dépendances amont de dim_scenario) ---
    TableDef {
        api_name: "scenario_categories",
        sql_name: "dim_scenario_category",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "variants",
        sql_name: "dim_variant",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "rate_sets",
        sql_name: "dim_rate_set",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    // --- Scénario v2 : category/entry_period/presentation_currency/variant/
    //     ruleset_code(nullable)/rate_set/statut (cf. SPEC_SCENARIO_V2_TECH §1.2) ---
    TableDef {
        api_name: "scenarios",
        sql_name: "dim_scenario",
        columns: &[
            "code",
            "libelle",
            "category",
            "entry_period",
            "presentation_currency",
            "variant",
            "ruleset_code",
            "rate_set",
            "statut",
        ],
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
        api_name: "methods",
        sql_name: "dim_method",
        columns: &["code", "libelle", "consolidated"],
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
    // PK étendue v2 : (rate_set, currency_source, period). `rate_set` en 1ère
    // position pour cohérence avec la PK (cf. SPEC_SCENARIO_V2_TECH §1.3).
    TableDef {
        api_name: "rates",
        sql_name: "sat_exchange_rate",
        columns: &["rate_set", "currency_source", "period", "taux_close", "taux_moyen"],
        pk: &["rate_set", "currency_source", "period"],
    },
];

fn find_table(api: &str) -> Option<&'static TableDef> {
    TABLES.iter().find(|t| t.api_name == api)
}

/// Nom d'API (master data) correspondant à une table SQL, s'il en existe un.
/// Sert à traduire les cibles du graphe de références (`references.rs` raisonne
/// en noms SQL) vers les identifiants que le front consomme (`/api/md/{table}`).
fn api_name_for_sql(sql: &str) -> Option<&'static str> {
    TABLES.iter().find(|t| t.sql_name == sql).map(|t| t.api_name)
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

pub(crate) fn json_to_db_value(v: &JsonValue) -> DbValue {
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

pub fn run_query(
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

/// Vérifie que chaque valeur référentielle de la ligne existe dans sa table
/// cible (cf. [`crate::references`]). Tolère l'auto-référence (valeur = PK de la
/// ligne elle-même, ex. `dim_flow.flux_de_report = 'F99'` sur la ligne F99) et
/// ignore les colonnes non-textuelles ou vides.
fn validate_references(
    def: &TableDef,
    obj: &Map<String, JsonValue>,
    con: &duckdb::Connection,
) -> Result<(), AppError> {
    let mut bad = Vec::new();
    let mut missing = Vec::new();
    for r in references::references_for(def.sql_name) {
        // Une valeur est « vide » si la colonne est absente, JSON null, ou chaîne
        // vide. Sur une référence obligatoire (non-nullable), c'est rejeté ;
        // sinon (nullable) c'est toléré (= NULL).
        let s = match obj.get(r.column) {
            Some(JsonValue::String(s)) if !s.is_empty() => s.as_str(),
            None | Some(JsonValue::Null) | Some(JsonValue::String(_)) => {
                if r.required {
                    missing.push(r.column);
                }
                continue;
            }
            // Valeur non-textuelle (nombre, booléen…) : pas un code référentiel,
            // on ne vérifie pas son existence.
            Some(_) => continue,
        };
        // Auto-référence : la ligne se référence elle-même par sa PK.
        if r.target_table == def.sql_name {
            if let Some(own) = obj.get(r.target_column).and_then(|x| x.as_str()) {
                if own == s {
                    continue;
                }
            }
        }
        if !references::value_exists(con, r.target_table, r.target_column, s).map_err(db_err)? {
            bad.push(format!(
                "{} = '{}' (absent de {}.{})",
                r.column, s, r.target_table, r.target_column
            ));
        }
    }
    if !missing.is_empty() {
        return Err(AppError::bad_request(format!(
            "champ(s) obligatoire(s) non renseigné(s) : {}",
            missing.join(", ")
        )));
    }
    if !bad.is_empty() {
        return Err(AppError::bad_request(format!(
            "référence(s) invalide(s) : {}",
            bad.join(" ; ")
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
        validate_references(def, &obj, &con)?;
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
        validate_references(def, &obj, &con)?;
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

/// Ligne renvoyée par `GET /api/meta/references` : une référence du graphe.
///
/// `table` (source) et `target_table` sont traduits en nom d'API master data
/// quand la table en a un (ex. `dim_scenario` → `scenarios`, `sat_perimeter` →
/// `perimeter`) ; sinon le nom SQL est conservé (ex. `stg_entry` qui n'est pas
/// une table master data CRUD, ou `dim_ruleset` / `dim_rule`). Le front filtre
/// donc sur `stg_entry` / `perimeter` pour dériver le mapping dimension → table.
#[derive(serde::Serialize)]
struct ReferenceDto {
    table: String,
    column: &'static str,
    target_table: String,
    target_column: &'static str,
    required: bool,
}

/// GET /api/meta/references — graphe des références (source de vérité unique pour
/// les dropdowns contextuels du front, en remplacement des miroirs codés en dur).
async fn get_references() -> Json<Vec<ReferenceDto>> {
    // Traduit un nom de table SQL en nom d'API master data, en conservant le nom
    // SQL pour les tables sans équivalent CRUD (stg_entry, dim_ruleset…).
    let to_api = |sql: &'static str| {
        api_name_for_sql(sql)
            .map(str::to_string)
            .unwrap_or_else(|| sql.to_string())
    };
    let out = references::REFERENCES
        .iter()
        .map(|r| ReferenceDto {
            table: to_api(r.table),
            column: r.column,
            target_table: to_api(r.target_table),
            target_column: r.target_column,
            required: r.required,
        })
        .collect();
    Json(out)
}

/// Une anomalie d'intégrité : des valeurs de `table.column` n'existent pas dans
/// `target_table.target_column`. `count` = nb de valeurs orphelines distinctes,
/// `sample` = échantillon (max 20).
#[derive(serde::Serialize)]
struct OrphanCheck {
    table: String,
    column: &'static str,
    target_table: String,
    target_column: &'static str,
    count: i64,
    sample: Vec<String>,
}

/// Rapport « santé des données » : agrège les orphelins sur tout le graphe.
#[derive(serde::Serialize)]
struct DataHealthReport {
    ok: bool,
    total: i64,
    checks: Vec<OrphanCheck>,
}

/// GET /api/meta/health — rapport d'orphelins sur l'ensemble du graphe de
/// références (généralise `validate::check_natures` à toutes les FK). Pour
/// chaque référence, liste les valeurs présentes dans la source mais absentes de
/// la cible (vide/NULL ignoré : c'est la nullabilité, pas un orphelin).
///
/// Sécurité : tables/colonnes proviennent du registre `const` `REFERENCES`
/// (jamais de l'utilisateur) → interpolation sûre.
async fn get_data_health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DataHealthReport>, AppError> {
    let con = lock_con(&state)?;
    let mut checks = Vec::new();
    let mut total = 0i64;
    for r in references::REFERENCES {
        let where_orphan = format!(
            "e.\"{col}\" IS NOT NULL AND e.\"{col}\" <> '' \
             AND NOT EXISTS (SELECT 1 FROM {tt} t WHERE t.\"{tc}\" = e.\"{col}\")",
            col = r.column,
            tt = r.target_table,
            tc = r.target_column,
        );
        let count_sql = format!(
            "SELECT COUNT(DISTINCT e.\"{col}\") FROM {tbl} e WHERE {where_orphan}",
            col = r.column,
            tbl = r.table,
        );
        // Table source potentiellement absente selon le schéma → on tolère.
        let count: i64 = match con.query_row(&count_sql, [], |row| row.get(0)) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if count == 0 {
            continue;
        }
        let sample_sql = format!(
            "SELECT DISTINCT e.\"{col}\" AS v FROM {tbl} e WHERE {where_orphan} ORDER BY v LIMIT 20",
            col = r.column,
            tbl = r.table,
        );
        let mut sample = Vec::new();
        if let Ok(mut stmt) = con.prepare(&sample_sql) {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for v in rows.flatten() {
                    sample.push(v);
                }
            }
        }
        total += count;
        checks.push(OrphanCheck {
            table: api_name_for_sql(r.table).unwrap_or(r.table).to_string(),
            column: r.column,
            target_table: api_name_for_sql(r.target_table)
                .unwrap_or(r.target_table)
                .to_string(),
            target_column: r.target_column,
            count,
            sample,
        });
    }
    Ok(Json(DataHealthReport {
        ok: checks.is_empty(),
        total,
        checks,
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/md/{table}",
            get(list).post(create).put(update).delete(remove),
        )
        .route("/api/meta/references", get(get_references))
        .route("/api/meta/health", get(get_data_health))
}
