//! CRUD générique sur les tables master data (dimensions + satellites +
//! catalogues scénario v2 + tables dynamiques des caractéristiques/listes).
//!
//! Expose `router()` qui monte les routes :
//! - `GET /api/md` — liste des tables navigables (natives + `car_<code>` + `lst_<code>`) ;
//! - `GET /api/md/{table}/schema` — schéma complet d'une table (colonnes + métadonnées FK) ;
//! - `GET/POST/PUT/DELETE /api/md/{table}` — CRUD sur les lignes.
//!
//! La résolution d'une table est **dynamique** ([`resolve_table`]) :
//! - tables natives de la whitelist [`TABLES`] ;
//! - tables de valeurs `car_<code>` (caractéristiques) et `lst_<code>` (listes) ;
//! - colonnes ajoutées dynamiquement aux tables natives par les caractéristiques
//!   (rattachement N1) et les références directes (patron B) — découvertes via le
//!   graphe [`crate::references::all_references`].
//!
//! Sécurité : aucun nom de table ni de colonne n'est interpolé depuis
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
    label: &'static str,
    sql_name: &'static str,
    columns: &'static [&'static str],
    pk: &'static [&'static str],
}

const TABLES: &[TableDef] = &[
    // --- Catalogues v2 (dépendances amont de dim_scenario) ---
    TableDef {
        api_name: "scenario_categories",
        label: "Phases",
        sql_name: "dim_scenario_category",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "variants",
        label: "Variantes",
        sql_name: "dim_variant",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "rate_sets",
        label: "Jeux de taux",
        sql_name: "dim_rate_set",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "perimeter_sets",
        label: "Jeux de périmètre",
        sql_name: "dim_perimeter_set",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    // --- Scénario v2 : category/entry_period/presentation_currency/variant/
    //     ruleset_code(nullable)/rate_set/statut (cf. SPEC_SCENARIO_V2_TECH §1.2) ---
    TableDef {
        api_name: "scenarios",
        label: "Définitions de consolidation",
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
            "perimeter_set",
            "statut",
            "a_nouveau_scenario",
        ],
        pk: &["code"],
    },
    TableDef {
        api_name: "entities",
        label: "Entités",
        sql_name: "dim_entity",
        columns: &[
            "code",
            "libelle",
            "devise_fonctionnelle",
            "entite_parent",
            "statut",
        ],
        pk: &["code"],
    },
    TableDef {
        api_name: "periods",
        label: "Périodes",
        sql_name: "dim_period",
        columns: &[
            "code",
            "libelle",
            "type",
            "date_debut",
            "date_fin",
            "statut",
        ],
        pk: &["code"],
    },
    TableDef {
        api_name: "accounts",
        label: "Comptes",
        sql_name: "dim_account",
        // `compte_parent` (réf. directe) et le regroupement par nature
        // (caractéristique) ne sont pas des colonnes en dur : ils sont gérés via
        // la page « Attributs de dimension » (characteristics / custom_references)
        // et **découverts dynamiquement** par [`resolve_table`].
        // `flow_scheme` (nullable) sélectionne le schéma de flux du compte (Q32).
        columns: &["code", "libelle", "classe", "sous_classe", "flow_scheme"],
        pk: &["code"],
    },
    TableDef {
        api_name: "sous_classes",
        label: "Sous-classes",
        sql_name: "dim_sous_classe",
        columns: &["code", "libelle", "classe"],
        pk: &["code"],
    },
    TableDef {
        api_name: "flows",
        label: "Flux",
        sql_name: "dim_flow",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "flow_schemes",
        label: "Schémas de flux",
        sql_name: "dim_flow_scheme",
        columns: &["code", "libelle"],
        pk: &["code"],
    },
    TableDef {
        api_name: "flow_scheme_items",
        label: "Articulation des flux (par schéma)",
        sql_name: "sat_flow_scheme_item",
        columns: &[
            "scheme",
            "flow",
            "taux_conversion",
            "flux_ecart",
            "flux_de_report",
            "flux_a_nouveau",
        ],
        pk: &["scheme", "flow"],
    },
    TableDef {
        api_name: "currencies",
        label: "Devises",
        sql_name: "dim_currency",
        columns: &["code_iso", "libelle", "decimales"],
        pk: &["code_iso"],
    },
    TableDef {
        api_name: "natures",
        label: "Natures",
        sql_name: "dim_nature",
        columns: &["code", "libelle", "rules"],
        pk: &["code"],
    },
    TableDef {
        api_name: "methods",
        label: "Méthodes de consolidation",
        sql_name: "dim_method",
        columns: &["code", "libelle", "consolidated"],
        pk: &["code"],
    },
    TableDef {
        api_name: "perimeter",
        label: "Périmètre",
        sql_name: "sat_perimeter",
        columns: &[
            "perimeter_set",
            "entity",
            "period",
            "methode",
            "pct_interet",
            "pct_integration",
            "entree",
            "sortie",
        ],
        pk: &["perimeter_set", "entity", "period"],
    },
    // PK étendue v2 : (rate_set, currency_source, period). `rate_set` en 1ère
    // position pour cohérence avec la PK (cf. SPEC_SCENARIO_V2_TECH §1.3).
    // `taux_ouverture` = clôture N-1 portée par N (résout `close_n1`).
    TableDef {
        api_name: "rates",
        label: "Taux de change",
        sql_name: "sat_exchange_rate",
        columns: &[
            "rate_set",
            "currency_source",
            "period",
            "taux_close",
            "taux_moyen",
            "taux_ouverture",
        ],
        pk: &["rate_set", "currency_source", "period"],
    },
];

fn find_table(api: &str) -> Option<&'static TableDef> {
    TABLES.iter().find(|t| t.api_name == api)
}

/// Nom d'API (master data) correspondant à une table SQL, s'il en existe un.
/// Sert à traduire les cibles du graphe de références (`references.rs` raisonne
/// en noms SQL) vers les identifiants que le front consomme (`/api/md/{table}`).
/// Pour les tables dynamiques (`car_<code>`, `lst_<code>`), le nom SQL est aussi
/// le nom d'API — on le renvoie tel quel.
fn sql_to_api(sql: &str) -> String {
    api_name_for_sql(sql).unwrap_or(sql).to_string()
}

/// Variante stricte (tables natives uniquement) — utilisée par les endpoints
/// historiques (`get_references`, `get_data_health`) qui raisonnent sur la
/// whitelist statique pour produire les noms d'API.
fn api_name_for_sql(sql: &str) -> Option<&'static str> {
    TABLES
        .iter()
        .find(|t| t.sql_name == sql)
        .map(|t| t.api_name)
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

// ─────────────────────── Résolution dynamique des tables ───────────────────────

/// Définition de table « possédée » : noms et colonnes alloués à l'exécution
/// pour pouvoir exprimer :
/// - les tables natives étendues de leurs colonnes dynamiques (caractéristiques,
///   références directes ajoutées par ALTER TABLE) ;
/// - les tables de valeurs `car_<code>` et `lst_<code>` qui ne sont pas dans la
///   whitelist statique.
struct OwnedTableDef {
    api_name: String,
    label: String,
    sql_name: String,
    columns: Vec<String>,
    pk: Vec<String>,
}

/// Résout une table demandée par son nom d'API (`/api/md/{table}`) en une
/// [`OwnedTableDef`] complète, en découvrant dynamiquement :
/// - pour une table native (`dim_<base>`) : les colonnes ajoutées par les
///   caractéristiques et les références directes (via le graphe
///   [`references::all_references`]) ;
/// - pour `car_<code>` : les attributs N2 de la caractéristique ;
/// - pour `lst_<code>` : juste `code`, `libelle`.
///
/// Renvoie `Ok(None)` si le nom ne correspond à aucune table connue (native ou
/// enregistrée). La résolution d'une table native ne touche pas la base ; celle
/// d'une `car_` / `lst_` vérifie l'existence du code dans les registres.
fn resolve_table(con: &duckdb::Connection, api: &str) -> Result<Option<OwnedTableDef>, AppError> {
    // 1. Table native (whitelist statique) étendue des colonnes dynamiques.
    if let Some(def) = find_table(api) {
        let native: Vec<String> = def.columns.iter().map(|s| s.to_string()).collect();
        // On complète avec toute colonne dynamique référencée sur cette table et
        // absente de la whitelist native (caractéristiques N1 + références
        // directes patron B). On s'appuie sur le graphe de références, déjà
        // construit par `references::dynamic_references` à partir des registres.
        let mut cols = native.clone();
        for r in references::all_references(con) {
            if r.table == def.sql_name && !cols.contains(&r.column) {
                cols.push(r.column);
            }
        }
        return Ok(Some(OwnedTableDef {
            api_name: def.api_name.to_string(),
            label: def.label.to_string(),
            sql_name: def.sql_name.to_string(),
            columns: cols,
            pk: def.pk.iter().map(|s| s.to_string()).collect(),
        }));
    }

    // 2. Table de valeurs d'une caractéristique : car_<code>.
    if let Some(code) = api.strip_prefix("car_") {
        // `load_all` filtre déjà l'absence éventuelle des registres (premier
        // démarrage). On recherche la caractéristique demandée.
        let chars = crate::characteristics::load_all(con).map_err(db_err)?;
        if let Some(c) = chars.into_iter().find(|c| c.code == code) {
            let mut cols = vec!["code".to_string(), "libelle".to_string()];
            cols.extend(c.attributes.iter().map(|a| a.name.clone()));
            return Ok(Some(OwnedTableDef {
                api_name: format!("car_{code}"),
                label: c.libelle,
                sql_name: format!("car_{code}"),
                columns: cols,
                pk: vec!["code".to_string()],
            }));
        }
        return Ok(None);
    }

    // 3. Table de valeurs d'une liste : lst_<code>.
    if let Some(code) = api.strip_prefix("lst_") {
        let lists = crate::value_lists::load_all(con).map_err(db_err)?;
        if let Some(l) = lists.into_iter().find(|l| l.code == code) {
            return Ok(Some(OwnedTableDef {
                api_name: format!("lst_{code}"),
                label: l.libelle,
                sql_name: format!("lst_{code}"),
                columns: vec!["code".to_string(), "libelle".to_string()],
                pk: vec!["code".to_string()],
            }));
        }
        return Ok(None);
    }

    Ok(None)
}

fn select_all(def: &OwnedTableDef, con: &duckdb::Connection) -> Result<Vec<JsonValue>, AppError> {
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
    def: &OwnedTableDef,
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
fn reject_unknown_fields(
    def: &OwnedTableDef,
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    let known: std::collections::HashSet<&str> = def.columns.iter().map(|s| s.as_str()).collect();
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
    def: &OwnedTableDef,
    obj: &Map<String, JsonValue>,
    con: &duckdb::Connection,
) -> Result<(), AppError> {
    let mut bad = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    // `all_references` = graphe statique + références dynamiques (caractéristiques
    // N1/N2 + références directes patron B). Filtré sur la table éditée.
    let all = references::all_references(con);
    for r in all.iter().filter(|r| r.table == def.sql_name) {
        // Une valeur est « vide » si la colonne est absente, JSON null, ou chaîne
        // vide. Sur une référence obligatoire (non-nullable), c'est rejeté ;
        // sinon (nullable) c'est toléré (= NULL).
        let s = match obj.get(r.column.as_str()) {
            Some(JsonValue::String(s)) if !s.is_empty() => s.as_str(),
            None | Some(JsonValue::Null) | Some(JsonValue::String(_)) => {
                if r.required {
                    missing.push(r.column.clone());
                }
                continue;
            }
            // Valeur non-textuelle (nombre, booléen…) : pas un code référentiel,
            // on ne vérifie pas son existence.
            Some(_) => continue,
        };
        // Auto-référence : la ligne se référence elle-même par sa PK.
        if r.target_table == def.sql_name {
            if let Some(own) = obj.get(r.target_column.as_str()).and_then(|x| x.as_str()) {
                if own == s {
                    continue;
                }
            }
        }
        if !references::value_exists(con, &r.target_table, &r.target_column, s).map_err(db_err)? {
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

fn pk_from_body(
    def: &OwnedTableDef,
    body: &JsonValue,
) -> Result<Vec<(String, JsonValue)>, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?;
    let mut out = Vec::with_capacity(def.pk.len());
    for col in &def.pk {
        let val = obj
            .get(col.as_str())
            .cloned()
            .ok_or_else(|| AppError::bad_request(format!("colonne PK manquante : {col}")))?;
        if val.is_null() {
            return Err(AppError::bad_request(format!("colonne PK nulle : {col}")));
        }
        out.push((col.clone(), val));
    }
    Ok(out)
}

// ───────────────────────────── HTTP — CRUD lignes ──────────────────────────────

async fn list(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let def = resolve_table(&con, &table)?
            .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
        select_all(&def, &con)?
    };
    Ok(Json(rows))
}

async fn create(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<JsonValue>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?
        .clone();

    let result = {
        let con = lock_con(&state)?;
        let def = resolve_table(&con, &table)?
            .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
        reject_unknown_fields(&def, &obj)?;
        let pk_vals = pk_from_body(&def, &JsonValue::Object(obj.clone()))?;
        if fetch_one(&def, &pk_vals, &con)?.is_some() {
            return Err(AppError::conflict("déjà existant"));
        }
        validate_references(&def, &obj, &con)?;
        let mut cols = Vec::new();
        let mut vals: Vec<DbValue> = Vec::new();
        for col in &def.columns {
            if let Some(v) = obj.get(col.as_str()) {
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
        fetch_one(&def, &pk_vals, &con)?
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
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?
        .clone();

    let result = {
        let con = lock_con(&state)?;
        let def = resolve_table(&con, &table)?
            .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
        reject_unknown_fields(&def, &obj)?;
        let pk_vals = pk_from_body(&def, &JsonValue::Object(obj.clone()))?;
        if fetch_one(&def, &pk_vals, &con)?.is_none() {
            return Err(AppError::not_found("introuvable"));
        }
        validate_references(&def, &obj, &con)?;
        let mut sets = Vec::new();
        let mut vals: Vec<DbValue> = Vec::new();
        for col in &def.columns {
            if def.pk.iter().any(|p| p == col) {
                continue;
            }
            if let Some(v) = obj.get(col.as_str()) {
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
        fetch_one(&def, &pk_vals, &con)?
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
    // On a besoin de la définition (notamment la PK) avant de pouvoir interpréter
    // les paramètres de suppression : on locke la con et on résout la table.
    let (def, deleted) = {
        let con = lock_con(&state)?;
        let def = resolve_table(&con, &table)?
            .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;

        // PK depuis query string ou body JSON (query string prioritaire)
        let pk_vals: Vec<(String, JsonValue)> = {
            let from_query: Vec<(String, JsonValue)> = def
                .pk
                .iter()
                .filter_map(|col| {
                    query
                        .iter()
                        .find(|(k, _)| k == col)
                        .map(|(_, v)| (col.clone(), JsonValue::String(v.clone())))
                })
                .collect();

            if from_query.len() == def.pk.len() {
                from_query
            } else if !body_bytes.is_empty() {
                let body: JsonValue = serde_json::from_slice(&body_bytes)
                    .map_err(|_| AppError::bad_request("body JSON invalide"))?;
                pk_from_body(&def, &body)?
            } else {
                let pk_cols = def.pk.join(", ");
                return Err(AppError::bad_request(format!(
                    "PK manquante. Passez-la en query string (?{pk_cols}=valeur) ou dans le body JSON.\n\
                     Exemple : DELETE /api/md/{table}?{pk_cols}=valeur"
                )));
            }
        };

        if fetch_one(&def, &pk_vals, &con)?.is_none() {
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
        let n = con
            .execute(&sql, params_from_iter(params))
            .map_err(db_err)?;
        (def, n)
    };
    let _ = def; // définition déjà consommée pour la PK
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

// ─────────────────────── HTTP — Schéma et liste des tables ─────────────────────

/// Résumé d'une table navigable dans master data.
#[derive(serde::Serialize)]
struct TableSummary {
    /// Nom d'API (`/api/md/{table}`).
    table: String,
    /// Libellé affichable.
    label: String,
    /// `native` (dimension/satellite builtin), `characteristic` (`car_<code>`),
    /// ou `value_list` (`lst_<code>`).
    kind: &'static str,
}

/// GET /api/md — liste toutes les tables master data navigables : natives +
/// caractéristiques (`car_<code>`) + listes de valeurs (`lst_<code>`).
async fn list_tables(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<TableSummary>>, AppError> {
    let con = lock_con(&state)?;
    let mut out: Vec<TableSummary> = TABLES
        .iter()
        .map(|t| TableSummary {
            table: t.api_name.to_string(),
            label: t.label.to_string(),
            kind: "native",
        })
        .collect();
    for c in crate::characteristics::load_all(&con).map_err(db_err)? {
        out.push(TableSummary {
            table: format!("car_{}", c.code),
            label: c.libelle,
            kind: "characteristic",
        });
    }
    for l in crate::value_lists::load_all(&con).map_err(db_err)? {
        out.push(TableSummary {
            table: format!("lst_{}", l.code),
            label: l.libelle,
            kind: "value_list",
        });
    }
    Ok(Json(out))
}

/// Cible d'une FK : `(api_table, colonne)` + nullabilité.
#[derive(serde::Serialize)]
struct FkTarget {
    /// Nom d'API de la table cible (`accounts`, `car_comportement`, `lst_incoterm`…).
    table: String,
    /// Colonne clé de la table cible (`code`, `code_iso`…).
    column: String,
    /// `true` si non-nullable (rejette une valeur vide à l'écriture).
    required: bool,
}

/// Métadonnées d'une colonne pour le schéma exposé.
#[derive(serde::Serialize)]
struct ColumnSchema {
    name: String,
    /// `true` si la colonne fait partie de la PK.
    pk: bool,
    /// Référence (FK) portée par cette colonne, le cas échéant — sert au front
    /// à configurer un dropdown d'options depuis `GET /api/md/{fk.table}`.
    fk: Option<FkTarget>,
}

/// Schéma complet d'une table, tel qu'exposé par `GET /api/md/{table}/schema`.
#[derive(serde::Serialize)]
struct TableSchema {
    /// Nom d'API.
    table: String,
    /// Libellé affichable.
    label: String,
    /// Nom SQL sous-jacent (utile pour debug).
    sql_name: String,
    /// Colonnes natives + dynamiques, dans l'ordre canonique.
    columns: Vec<ColumnSchema>,
    /// Liste des colonnes composant la PK.
    pk: Vec<String>,
}

/// GET /api/md/{table}/schema — schéma complet d'une table : colonnes (natives +
/// dynamiques) avec métadonnées FK, pour permettre au front de construire
/// dynamiquement la grille et les dropdowns contextuels.
async fn table_schema(
    Path(table): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<TableSchema>, AppError> {
    let con = lock_con(&state)?;
    let def = resolve_table(&con, &table)?
        .ok_or_else(|| AppError::bad_request(format!("table inconnue : {table}")))?;
    // Graphe de références : on ne garde que celles portées par cette table.
    let all_refs = references::all_references(&con);
    let refs: Vec<&references::OwnedReference> = all_refs
        .iter()
        .filter(|r| r.table == def.sql_name)
        .collect();
    let columns = def
        .columns
        .iter()
        .map(|name| {
            let pk = def.pk.iter().any(|p| p == name);
            let fk = refs.iter().find(|r| r.column == *name).map(|r| FkTarget {
                table: sql_to_api(&r.target_table),
                column: r.target_column.clone(),
                required: r.required,
            });
            ColumnSchema {
                name: name.clone(),
                pk,
                fk,
            }
        })
        .collect();
    Ok(Json(TableSchema {
        table: def.api_name,
        label: def.label,
        sql_name: def.sql_name,
        columns,
        pk: def.pk,
    }))
}

// ───────────────────────── HTTP — Graphe de références ─────────────────────────

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
    column: String,
    target_table: String,
    target_column: String,
    required: bool,
}

/// GET /api/meta/references — graphe des références (source de vérité unique pour
/// les dropdowns contextuels du front, en remplacement des miroirs codés en dur).
/// Inclut les références **dynamiques** des caractéristiques N1/N2 (cf.
/// [`references::all_references`]).
async fn get_references(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ReferenceDto>>, AppError> {
    let con = lock_con(&state)?;
    let out = references::all_references(&con)
        .into_iter()
        .map(|r| ReferenceDto {
            table: sql_to_api(&r.table),
            column: r.column,
            target_table: sql_to_api(&r.target_table),
            target_column: r.target_column,
            required: r.required,
        })
        .collect();
    Ok(Json(out))
}

/// Ligne renvoyée par `GET /api/meta/native-enums` : un enum natif `CHECK` du
/// DDL (ex. `account.classe`), exposé comme attribut traversable en sélection
/// via le mode `attr` de `SelectionCond` (cf. `rules.rs`).
#[derive(serde::Serialize)]
struct NativeEnumDto {
    host_dimension: &'static str,
    column: &'static str,
    values: &'static [&'static str],
}

/// GET /api/meta/native-enums — catalogue des enums natifs (`CHECK` du DDL).
/// Contrairement à `references-custom` (FK natives auto-peuplées), ces enums
/// n'ont pas de table cible : ils sont résolus par filtre direct sur la colonne
/// de la master data hôte (cf. mode `attr` dans `rules.rs`).
async fn get_native_enums() -> Json<Vec<NativeEnumDto>> {
    let out = references::NATIVE_ENUMS
        .iter()
        .map(|e| NativeEnumDto {
            host_dimension: e.host_dimension,
            column: e.column,
            values: e.values,
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
    column: String,
    target_table: String,
    target_column: String,
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
    // Graphe statique + références dynamiques (caractéristiques N1/N2) : ainsi le
    // rapport couvre aussi les orphelins sur les colonnes de rattachement et les
    // attributs des caractéristiques.
    for r in references::all_references(&con) {
        let col = r.column.as_str();
        let tt = r.target_table.as_str();
        let tc = r.target_column.as_str();
        let tbl = r.table.as_str();
        let where_orphan = format!(
            "e.\"{col}\" IS NOT NULL AND e.\"{col}\" <> '' \
             AND NOT EXISTS (SELECT 1 FROM {tt} t WHERE t.\"{tc}\" = e.\"{col}\")"
        );
        let count_sql =
            format!("SELECT COUNT(DISTINCT e.\"{col}\") FROM {tbl} e WHERE {where_orphan}");
        // Table source potentiellement absente selon le schéma → on tolère.
        let count: i64 = match con.query_row(&count_sql, [], |row| row.get(0)) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if count == 0 {
            continue;
        }
        let sample_sql = format!(
            "SELECT DISTINCT e.\"{col}\" AS v FROM {tbl} e WHERE {where_orphan} ORDER BY v LIMIT 20"
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
            table: sql_to_api(tbl),
            column: col.to_string(),
            target_table: sql_to_api(tt),
            target_column: tc.to_string(),
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
        .route("/api/md", get(list_tables))
        .route(
            "/api/md/{table}",
            get(list).post(create).put(update).delete(remove),
        )
        .route("/api/md/{table}/schema", get(table_schema))
        .route("/api/meta/references", get(get_references))
        .route("/api/meta/native-enums", get(get_native_enums))
        .route("/api/meta/health", get(get_data_health))
}

// ───────────────────────────────── Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    fn setup() -> Connection {
        let con = Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        con
    }

    /// `true` si `api` apparaît dans la liste des tables navigables.
    fn table_listed(con: &Connection, api: &str) -> bool {
        // Reproduit le calcul de `list_tables` (synchrone, sans router) en
        // parcourant les natives + caractéristiques + listes.
        if TABLES.iter().any(|t| t.api_name == api) {
            return true;
        }
        if api.starts_with("car_") {
            let code = &api[4..];
            return crate::characteristics::load_all(con)
                .unwrap_or_default()
                .iter()
                .any(|c| c.code == code);
        }
        if api.starts_with("lst_") {
            let code = &api[4..];
            return crate::value_lists::load_all(con)
                .unwrap_or_default()
                .iter()
                .any(|l| l.code == code);
        }
        false
    }

    #[test]
    fn resolve_table_native_basique() {
        let con = setup();
        let def = resolve_table(&con, "flows")
            .unwrap()
            .expect("flows résolvable");
        assert_eq!(def.sql_name, "dim_flow");
        assert_eq!(def.api_name, "flows");
        assert_eq!(def.pk, vec!["code".to_string()]);
        // Sans caractéristique/ref directe, colonnes = whitelist native.
        assert_eq!(def.columns, vec!["code".to_string(), "libelle".to_string()]);
    }

    #[test]
    fn resolve_table_etend_avec_caracteristique_et_ref_directe() {
        let con = setup();
        // Caractéristique N1 sur account : dim_account.comportement (TEXT).
        crate::characteristics::create_characteristic(
            &con,
            "comportement",
            "Comportement",
            "account",
        )
        .unwrap();
        // Référence directe (patron B) sur account : compte_parent.
        crate::custom_references::create(&con, "account", "compte_parent", "account").unwrap();

        let def = resolve_table(&con, "accounts")
            .unwrap()
            .expect("accounts résolvable");
        assert_eq!(def.sql_name, "dim_account");
        // Colonnes natives + comportement + compte_parent.
        assert!(def.columns.contains(&"comportement".to_string()));
        assert!(def.columns.contains(&"compte_parent".to_string()));
        // Les natives sont toujours là.
        assert!(def.columns.contains(&"code".to_string()));
        assert!(def.columns.contains(&"sous_classe".to_string()));
    }

    #[test]
    fn resolve_table_car_code() {
        let con = setup();
        crate::characteristics::create_characteristic(
            &con,
            "comportement",
            "Comportement",
            "account",
        )
        .unwrap();
        crate::characteristics::add_attribute(
            &con,
            "comportement",
            "compte_destination",
            "C",
            "account",
        )
        .unwrap();

        let def = resolve_table(&con, "car_comportement")
            .unwrap()
            .expect("car_comportement résolvable");
        assert_eq!(def.sql_name, "car_comportement");
        assert_eq!(def.label, "Comportement");
        assert_eq!(def.pk, vec!["code".to_string()]);
        // code, libelle + l'attribut N2.
        assert_eq!(
            def.columns,
            vec![
                "code".to_string(),
                "libelle".to_string(),
                "compte_destination".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_table_lst_code() {
        let con = setup();
        crate::value_lists::create_list(&con, "incoterm", "Incoterms").unwrap();
        let def = resolve_table(&con, "lst_incoterm")
            .unwrap()
            .expect("lst_incoterm résolvable");
        assert_eq!(def.sql_name, "lst_incoterm");
        assert_eq!(def.label, "Incoterms");
        assert_eq!(def.columns, vec!["code".to_string(), "libelle".to_string()]);
        assert_eq!(def.pk, vec!["code".to_string()]);
    }

    #[test]
    fn resolve_table_inconnue_renvoie_none() {
        let con = setup();
        assert!(resolve_table(&con, "inconnue").unwrap().is_none());
        // car_<code> inexistant → None (pas de panic sur une table absente).
        assert!(resolve_table(&con, "car_merveille").unwrap().is_none());
        assert!(resolve_table(&con, "lst_fantome").unwrap().is_none());
    }

    #[test]
    fn list_tables_inclut_car_et_lst() {
        let con = setup();
        // Aucune caractéristique/liste : on a juste les natives.
        assert!(table_listed(&con, "flows"));
        assert!(table_listed(&con, "accounts"));
        assert!(!table_listed(&con, "car_x"));

        crate::characteristics::create_characteristic(&con, "comportement", "C", "account")
            .unwrap();
        crate::value_lists::create_list(&con, "incoterm", "Incoterms").unwrap();

        assert!(table_listed(&con, "car_comportement"));
        assert!(table_listed(&con, "lst_incoterm"));
    }

    /// Vérifie que le CRUD via `resolve_table` fonctionne sur une `car_<code>`,
    /// en exposant les colonnes `code`, `libelle` et l'attribut N2.
    #[test]
    fn crud_sur_car_via_master_data() {
        let con = setup();
        // Quelques comptes pour la FK N2.
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('471L', 'Liaison', 'bilan')",
            [],
        )
        .unwrap();

        crate::characteristics::create_characteristic(&con, "comportement", "C", "account")
            .unwrap();
        crate::characteristics::add_attribute(
            &con,
            "comportement",
            "compte_destination",
            "C",
            "account",
        )
        .unwrap();

        let def = resolve_table(&con, "car_comportement")
            .unwrap()
            .expect("car_comportement résolvable");

        // Insert via select_all/fetch_one (les helpers utilisés par les handlers).
        let mut obj = Map::new();
        obj.insert("code".into(), JsonValue::String("VENTES_IC".into()));
        obj.insert("libelle".into(), JsonValue::String("Ventes interco".into()));
        obj.insert(
            "compte_destination".into(),
            JsonValue::String("471L".into()),
        );
        reject_unknown_fields(&def, &obj).unwrap();
        validate_references(&def, &obj, &con).unwrap();

        let mut cols = Vec::new();
        let mut vals: Vec<DbValue> = Vec::new();
        for col in &def.columns {
            if let Some(v) = obj.get(col.as_str()) {
                cols.push(quote_ident(col));
                vals.push(json_to_db_value(v));
            }
        }
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            def.sql_name,
            cols.join(", "),
            cols.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
        );
        con.execute(&sql, params_from_iter(vals)).unwrap();

        // Lecture via select_all.
        let rows = select_all(&def, &con).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["code"], JsonValue::String("VENTES_IC".into()));
        assert_eq!(
            rows[0]["compte_destination"],
            JsonValue::String("471L".into())
        );

        // Validation N2 : valeur inexistante rejetée.
        let mut bad = Map::new();
        bad.insert("code".into(), JsonValue::String("X".into()));
        bad.insert(
            "compte_destination".into(),
            JsonValue::String("INEXISTANT".into()),
        );
        assert!(
            validate_references(&def, &bad, &con).is_err(),
            "FK N2 vérifiée via le graphe de références"
        );
    }

    /// L'écriture sur une colonne caractéristique (rattachement N1) via la table
    /// native étendue fonctionne et est validée contre `car_<code>`.
    #[test]
    fn update_colonne_caracteristique_via_master_data() {
        let con = setup();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('700', 'Ventes', 'resultat')",
            [],
        )
        .unwrap();
        crate::characteristics::create_characteristic(&con, "comportement", "C", "account")
            .unwrap();
        // Une valeur N1 à affecter.
        let mut v = Map::new();
        v.insert("code".into(), JsonValue::String("VENTES_IC".into()));
        crate::characteristics::create_value(&con, "comportement", &v).unwrap();

        let def = resolve_table(&con, "accounts").unwrap().expect("accounts");
        // La colonne caractéristique est exposée.
        assert!(def.columns.contains(&"comportement".to_string()));

        // Update via master data : on affecte la valeur N1 au compte 700.
        let mut upd = Map::new();
        upd.insert("code".into(), JsonValue::String("700".into()));
        upd.insert("comportement".into(), JsonValue::String("VENTES_IC".into()));
        validate_references(&def, &upd, &con).unwrap();

        // Valeur N1 inexistante → rejetée (FK dynamique).
        let mut bad = Map::new();
        bad.insert("code".into(), JsonValue::String("700".into()));
        bad.insert("comportement".into(), JsonValue::String("NOPE".into()));
        assert!(validate_references(&def, &bad, &con).is_err());
    }
}
