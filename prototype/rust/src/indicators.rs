//! Indicateurs / KPI — **volet 2** du moteur de formules. Spec : `docs/FORMULES.md` §4.
//!
//! Deux objets, stockés en base (survivent au reset comme `dim_coefficient`) :
//!
//! - **Poste** (`dim_aggregate`) : une sélection nommée sur `fact_entry` (un
//!   `level` + des conditions dimensionnelles, traversées `via`/`ref`/`attr`
//!   comprises), agrégée en un montant signé. C'est la brique de base.
//! - **Indicateur** (`dim_indicator`) : une **formule** (langage `formula.rs`)
//!   combinant des postes (et d'autres indicateurs) — ex.
//!   `SAFE_DIV([resultat]; [ca])` — calculée à un **grain** de restitution.
//!
//! # Compilation
//!
//! Un indicateur se compile en **une** requête ensembliste : chaque poste devient
//! un agrégat conditionnel `SUM(e.amount) FILTER (WHERE level=… AND <sélection>)`,
//! la formule devient de l'arithmétique dans le `SELECT`, le tout groupé par le
//! grain. Les traversées (`via`/`ref`/`attr`) ajoutent des **LEFT JOIN partagés**
//! (un poste ne doit pas filtrer les lignes des autres postes — d'où LEFT, le
//! `FILTER` de chaque poste fait le tri).
//!
//! # Sécurité SQL
//!
//! Mêmes règles que `rules.rs` : identifiants (dimensions, `via`/`ref`/`attr`,
//! `level`, grain) validés contre des whitelists dérivées du registre ; valeurs
//! via `?` paramétrés. Rien d'interpolé brut depuis le JSON utilisateur.
//!
//! # Non-additivité (cf. `docs/FORMULES.md` §4.3)
//!
//! Un ratio ne s'additionne pas : un indicateur est calculé **au grain demandé**,
//! jamais sommé. Et il n'est **jamais** réinjecté dans `fact_entry` (couche
//! dérivée de présentation).

use crate::formula::{self, CoeffJoins, OperandResolver, Resolved};
use crate::json_migration::{
    denormalize_aggregate_definition, normalize_aggregate_definition,
    normalize_indicator_expression,
};
use crate::state::{db_err, lock_con, AppError, AppState};
use crate::{characteristics, custom_references, dimensions, references};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use duckdb::{params, params_from_iter, types::Value as DbValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

const ALLOWED_LEVELS: &[&str] = &["corporate", "converted", "consolidated"];
const ALLOWED_OPS: &[&str] = &[
    "=", "!=", ">", "<", ">=", "<=", "IN", "IS NULL", "IS NOT NULL",
];

/// Migration idempotente au **démarrage** : crée `dim_aggregate` / `dim_indicator`
/// sur une base existante (volet 2), sans reset. Même esprit que
/// `coefficients::ensure_schema`.
pub fn ensure_schema(con: &Connection) -> duckdb::Result<()> {
    con.execute(crate::schema::DDL_DIM_AGGREGATE, [])?;
    con.execute(crate::schema::DDL_DIM_INDICATOR, [])?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Modèle d'un poste
// ─────────────────────────────────────────────────────────────────────────────

/// Une condition de sélection (même forme que `rules::SelectionCond`).
#[derive(Debug, Clone)]
struct SelCond {
    dim: String,
    op: String,
    val: Option<JsonValue>,
    via: Option<String>,      // caractéristique N1
    ref_field: Option<String>, // référence directe (patron B)
    attr: Option<String>,     // enum natif (classe, sous_classe…)
}

/// Un poste : niveau + sélection.
#[derive(Debug, Clone)]
struct Aggregate {
    level: String,
    selection: Vec<SelCond>,
}

fn parse_aggregate(definition: &str) -> Result<Aggregate, String> {
    let v: JsonValue = serde_json::from_str(definition)
        .map_err(|e| format!("définition de poste JSON invalide : {e}"))?;
    let obj = v.as_object().ok_or("la définition doit être un objet JSON")?;
    let level = obj
        .get("level")
        .and_then(|x| x.as_str())
        .ok_or("poste.level manquant")?
        .to_string();
    if !ALLOWED_LEVELS.contains(&level.as_str()) {
        return Err(format!("poste.level invalide : {level}"));
    }
    let selection = match obj.get("selection") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(parse_sel_cond)
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("poste.selection doit être un tableau".into()),
    };
    Ok(Aggregate { level, selection })
}

fn parse_sel_cond(v: &JsonValue) -> Result<SelCond, String> {
    let obj = v.as_object().ok_or("chaque condition doit être un objet")?;
    let dim = obj
        .get("dim")
        .and_then(|x| x.as_str())
        .ok_or("selection.dim manquant")?
        .to_string();
    let op = obj
        .get("op")
        .and_then(|x| x.as_str())
        .ok_or("selection.op manquant")?
        .to_string();
    if !ALLOWED_OPS.contains(&op.as_str()) {
        return Err(format!("selection.op invalide : {op}"));
    }
    let via = obj.get("via").and_then(|x| x.as_str()).map(String::from);
    let ref_field = obj.get("ref").and_then(|x| x.as_str()).map(String::from);
    let attr = obj.get("attr").and_then(|x| x.as_str()).map(String::from);
    if [via.is_some(), ref_field.is_some(), attr.is_some()]
        .iter()
        .filter(|&&b| b)
        .count()
        > 1
    {
        return Err(format!(
            "selection.{dim} : via / ref / attr sont mutuellement exclusifs"
        ));
    }
    Ok(SelCond {
        dim,
        op,
        val: obj.get("val").cloned(),
        via,
        ref_field,
        attr,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Construction du SQL d'un poste (prédicat + JOINs + paramètres)
// ─────────────────────────────────────────────────────────────────────────────

/// Parts mutables accumulées pendant la compilation d'un indicateur.
#[derive(Default)]
struct QueryParts {
    join_keys: BTreeSet<String>, // dédoublonnage des JOINs
    joins_sql: String,
    params: Vec<DbValue>,
    stack: Vec<String>, // détection de cycle entre indicateurs
}

/// Pousse une condition `operand op val` et renvoie le fragment SQL (avec `?`).
fn push_condition(
    operand: &str,
    op: &str,
    val: &Option<JsonValue>,
    params: &mut Vec<DbValue>,
) -> Result<String, String> {
    match op {
        "IS NULL" => Ok(format!("{operand} IS NULL")),
        "IS NOT NULL" => Ok(format!("{operand} IS NOT NULL")),
        "IN" => {
            let items: Vec<String> = match val {
                Some(JsonValue::Array(a)) => a.iter().filter_map(json_to_text).collect(),
                Some(JsonValue::String(s)) => {
                    s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
                }
                _ => Vec::new(),
            };
            if items.is_empty() {
                return Ok("1 = 0".to_string()); // IN vide → jamais vrai
            }
            let placeholders = vec!["?"; items.len()].join(", ");
            for it in items {
                params.push(DbValue::Text(it));
            }
            Ok(format!("{operand} IN ({placeholders})"))
        }
        "=" | "!=" | ">" | "<" | ">=" | "<=" => {
            let text = val
                .as_ref()
                .and_then(json_to_text)
                .ok_or_else(|| format!("valeur manquante pour op='{op}'"))?;
            params.push(DbValue::Text(text));
            Ok(format!("{operand} {op} ?"))
        }
        other => Err(format!("opérateur non supporté : {other}")),
    }
}

fn json_to_text(v: &JsonValue) -> Option<String> {
    match v {
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Number(n) => Some(n.to_string()),
        JsonValue::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Ajoute (dédoublonné) un JOIN au builder.
fn add_join(b: &mut QueryParts, key: &str, sql: String) {
    if b.join_keys.insert(key.to_string()) {
        b.joins_sql.push_str(&sql);
    }
}

/// Construit l'expression d'agrégat d'un poste :
/// `SUM(e.amount) FILTER (WHERE e.level = '<level>' AND <conditions>)`,
/// en ajoutant les LEFT JOINs de traversée nécessaires et en poussant les
/// paramètres dans l'ordre.
fn build_aggregate_sql(
    con: &Connection,
    b: &mut QueryParts,
    code: &str,
    agg: &Aggregate,
) -> Result<String, String> {
    let all_dims = dimensions::load_all(con).map_err(|e| e.to_string())?;
    let dims: Vec<String> = all_dims.iter().map(|d| d.name.clone()).collect();
    let mut clauses: Vec<String> = vec![format!("e.level = '{}'", agg.level)]; // level whitelisté
    for s in &agg.selection {
        // Validation de la dimension cible.
        if !dims.contains(&s.dim) {
            return Err(format!("poste '{code}' : dimension inconnue '{}'", s.dim));
        }
        // Opérande selon la traversée + JOIN associé (LEFT, partagé).
        let operand = if let Some(via) = &s.via {
            if !dimensions::is_valid_custom_name(via) {
                return Err(format!("via invalide : {via:?}"));
            }
            let base = characteristics::base_dimension_of(con, via)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("caractéristique inconnue : {via}"))?;
            if base != s.dim {
                return Err(format!(
                    "via '{via}' a pour base '{base}', pas '{}'",
                    s.dim
                ));
            }
            let (bt, _) = references::dimension_master_id_join(&base)
                .ok_or_else(|| format!("dimension sans master data : {base}"))?;
            let char_id = characteristics::id_of(con, via)
                .ok_or_else(|| format!("caractéristique '{via}' sans id technique"))?;
            let car_table = characteristics::value_table(char_id);
            add_join(
                b,
                &format!("via_{via}"),
                format!(
                    "\nLEFT JOIN {bt} imd_{via} ON imd_{via}.id = e.{base}\
                     \nLEFT JOIN {car_table} icg_{via} ON icg_{via}.code = imd_{via}.\"{via}\""
                ),
            );
            format!("icg_{via}.code")
        } else if let Some(rf) = &s.ref_field {
            if !dimensions::is_valid_custom_name(rf) {
                return Err(format!("ref invalide : {rf:?}"));
            }
            custom_references::target_of(con, &s.dim, rf)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("référence inconnue : {}.{}", s.dim, rf))?;
            let (ht, _) = references::dimension_master_id_join(&s.dim)
                .ok_or_else(|| format!("dimension sans master data : {}", s.dim))?;
            // B1 étape 11 : col physique r{id} pour refs custom, nom API pour natives.
            let rf_phys = custom_references::col_of_ref(con, &s.dim, rf)
                .unwrap_or_else(|_| rf.clone());
            add_join(
                b,
                &format!("ref_{}_{rf}", s.dim),
                format!("\nLEFT JOIN {ht} imdr_{rf} ON imdr_{rf}.id = e.{}", s.dim),
            );
            format!("imdr_{rf}.\"{rf_phys}\"")
        } else if let Some(attr) = &s.attr {
            if !dimensions::is_valid_custom_name(attr) {
                return Err(format!("attr invalide : {attr:?}"));
            }
            if references::native_enum_lookup(&s.dim, attr).is_none() {
                return Err(format!("enum natif inconnu : {}.{}", s.dim, attr));
            }
            let (ht, _) = references::dimension_master_id_join(&s.dim)
                .ok_or_else(|| format!("dimension sans master data : {}", s.dim))?;
            add_join(
                b,
                &format!("attr_{}_{attr}", s.dim),
                format!(
                    "\nLEFT JOIN {ht} imda_{}_{attr} ON imda_{}_{attr}.id = e.{}",
                    s.dim, s.dim, s.dim
                ),
            );
            format!("imda_{}_{attr}.{attr}", s.dim)
        } else if let Some((mt, _)) = references::dimension_master_id_join(&s.dim) {
            // Dimension à master data : sous B1 `e.{dim}` est un INTEGER id, mais
            // la sélection cite le **code** (contrat externe). Joindre la master
            // sur l'id et filtrer sur sa colonne de code.
            let (_, code_col) = references::dimension_master(&s.dim)
                .ok_or_else(|| format!("dimension sans master data : {}", s.dim))?;
            add_join(
                b,
                &format!("md_{}", s.dim),
                format!(
                    "\nLEFT JOIN {mt} imdd_{} ON imdd_{}.id = e.{}",
                    s.dim, s.dim, s.dim
                ),
            );
            format!("imdd_{}.{code_col}", s.dim)
        } else {
            // Dimension libre (analysis, analysis2, custom) : reste en TEXT.
            // B1 étape 10 : colonnes custom sont x{id}, pas le nom API.
            let col = dimensions::col_of(&all_dims, &s.dim);
            format!("e.{col}")
        };
        let cond = push_condition(&operand, &s.op, &s.val, &mut b.params)?;
        clauses.push(cond);
    }
    Ok(format!(
        "SUM(e.amount) FILTER (WHERE {})",
        clauses.join(" AND ")
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Résolveur de formule (contexte indicateur : opérandes = postes / indicateurs)
// ─────────────────────────────────────────────────────────────────────────────

struct IndicatorResolver<'a> {
    con: &'a Connection,
    builder: RefCell<QueryParts>,
}

impl<'a> OperandResolver for IndicatorResolver<'a> {
    fn resolve(&self, name: &str) -> Result<Resolved, String> {
        // 1. Poste (dim_aggregate) — par code ou par id entier (post-étape 6 B1).
        let agg_def = if let Ok(id) = name.parse::<i64>() {
            load_aggregate_def_by_id(self.con, id).map_err(|e| e.to_string())?
        } else {
            load_aggregate_def(self.con, name).map_err(|e| e.to_string())?
        };
        if let Some(def) = agg_def {
            // Dénormaliser avant parsing : la DB stocke `via` en ids entiers (étape 6b).
            let def_denorm = denormalize_aggregate_definition(self.con, &def)
                .map_err(|e| e.to_string())?;
            let agg = parse_aggregate(&def_denorm)?;
            let mut b = self.builder.borrow_mut();
            let sql = build_aggregate_sql(self.con, &mut b, name, &agg)?;
            return Ok(Resolved {
                sql,
                joins: CoeffJoins::default(),
            });
        }
        // 2. Indicateur (dim_indicator) ? → inline récursif. Par code ou par id.
        let ind_expr = if let Ok(id) = name.parse::<i64>() {
            load_indicator_expr_by_id(self.con, id).map_err(|e| e.to_string())?
        } else {
            load_indicator_expr(self.con, name).map_err(|e| e.to_string())?
        };
        if let Some(expr) = ind_expr {
            {
                let mut b = self.builder.borrow_mut();
                if b.stack.iter().any(|s| s == name) {
                    return Err(format!("cycle d'indicateurs détecté sur '{name}'"));
                }
                b.stack.push(name.to_string());
            }
            let res = formula::compile(&expr, self);
            self.builder.borrow_mut().stack.pop();
            let (sql, _) = res?;
            return Ok(Resolved {
                sql: format!("({sql})"),
                joins: CoeffJoins::default(),
            });
        }
        Err(format!(
            "référence inconnue : '{name}' (ni poste ni indicateur)"
        ))
    }
}

fn load_aggregate_def(con: &Connection, code: &str) -> duckdb::Result<Option<String>> {
    con.query_row(
        "SELECT definition FROM dim_aggregate WHERE code = ?",
        params![code],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        duckdb::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

fn load_aggregate_def_by_id(con: &Connection, id: i64) -> duckdb::Result<Option<String>> {
    con.query_row(
        "SELECT definition FROM dim_aggregate WHERE id = ?",
        params![id],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        duckdb::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

fn load_indicator_expr(con: &Connection, code: &str) -> duckdb::Result<Option<String>> {
    con.query_row(
        "SELECT expression FROM dim_indicator WHERE code = ?",
        params![code],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        duckdb::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

fn load_indicator_expr_by_id(con: &Connection, id: i64) -> duckdb::Result<Option<String>> {
    con.query_row(
        "SELECT expression FROM dim_indicator WHERE id = ?",
        params![id],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        duckdb::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Compilation + exécution d'un indicateur à un grain
// ─────────────────────────────────────────────────────────────────────────────

/// Compile une formule d'indicateur en SQL complet (SELECT … GROUP BY grain) +
/// paramètres, pour une consolidation donnée. `grain` = colonnes de restitution.
fn compile_indicator(
    con: &Connection,
    expression: &str,
    grain: &[String],
    consolidation_id: i64,
) -> Result<(String, Vec<DbValue>, Vec<String>), String> {
    // Valider le grain contre les dimensions propagées.
    let all_dims = dimensions::load_all(con).map_err(|e| e.to_string())?;
    let api_names: Vec<String> = all_dims.iter().map(|d| d.name.clone()).collect();
    for g in grain {
        if !api_names.contains(g) {
            return Err(format!("grain : dimension inconnue '{g}'"));
        }
    }
    let resolver = IndicatorResolver {
        con,
        builder: RefCell::new(QueryParts::default()),
    };
    let (value_sql, _) = formula::compile(expression, &resolver)?;
    let QueryParts {
        joins_sql, mut params, ..
    } = resolver.builder.into_inner();

    // B1 étape 10 : e.{col} pour les cols physiques (x{id} pour custom),
    // alias en nom API pour la sérialisation (IndicatorRow.grain).
    let select_grain: String = grain
        .iter()
        .map(|g| {
            let col = dimensions::col_of(&all_dims, g);
            format!("e.{col} AS {g}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let group_cols: String = grain
        .iter()
        .map(|g| dimensions::col_of(&all_dims, g).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let select = if grain.is_empty() {
        format!("{value_sql} AS value")
    } else {
        format!("{select_grain}, {value_sql} AS value")
    };
    let group = if grain.is_empty() {
        String::new()
    } else {
        format!("\nGROUP BY {group_cols}")
    };

    // `consolidation_id` poussé en dernier (apparaît dans le WHERE, après les
    // FILTER du SELECT — ordre textuel = ordre des `?`).
    params.push(DbValue::BigInt(consolidation_id));

    let sql = format!(
        "SELECT {select}\nFROM fact_entry e{joins_sql}\nWHERE e.consolidation_id = ?{group}"
    );
    Ok((sql, params, grain.to_vec()))
}

/// Une ligne de résultat : valeurs de grain (texte) + la valeur de l'indicateur.
#[derive(Serialize)]
pub struct IndicatorRow {
    pub grain: std::collections::BTreeMap<String, Option<String>>,
    pub value: Option<f64>,
}

/// Exécute un indicateur et renvoie ses lignes. Public pour le serveur MCP
/// (Q54) — calcule une formule à un grain sur une consolidation, sans passer
/// par l'API REST.
pub fn run_indicator(
    con: &Connection,
    expression: &str,
    grain: &[String],
    consolidation_id: i64,
) -> Result<Vec<IndicatorRow>, String> {
    let (sql, prm, grain_cols) = compile_indicator(con, expression, grain, consolidation_id)?;
    let mut stmt = con.prepare(&sql).map_err(|e| e.to_string())?;
    let n_grain = grain_cols.len();
    let rows = stmt
        .query_map(params_from_iter(prm.into_iter()), |r| {
            let mut grain = std::collections::BTreeMap::new();
            for (i, col) in grain_cols.iter().enumerate() {
                grain.insert(col.clone(), r.get::<_, Option<String>>(i)?);
            }
            // La valeur est la dernière colonne ; décimale → f64 pour l'affichage.
            let value: Option<f64> = r.get::<_, Option<f64>>(n_grain)?;
            Ok(IndicatorRow { grain, value })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<duckdb::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
}

/// Valide une définition de poste (parsing + construction SQL : vérifie dims,
/// traversées, opérateurs). N'exécute rien.
fn validate_aggregate(con: &Connection, definition: &str) -> Result<(), String> {
    let agg = parse_aggregate(definition)?;
    let mut b = QueryParts::default();
    build_aggregate_sql(con, &mut b, "(validation)", &agg).map(|_| ())
}

/// Valide une formule d'indicateur (parsing + résolution des postes/indicateurs).
fn validate_indicator(con: &Connection, expression: &str, grain: &[String]) -> Result<(), String> {
    compile_indicator(con, expression, grain, 0).map(|_| ())
}

// ─────────────────────────────────────────────────────────────────────────────
//  API REST
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AggregateOut {
    code: String,
    libelle: Option<String>,
    level: String,
    definition: JsonValue,
}

#[derive(Deserialize)]
struct AggregateBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    level: String,
    definition: JsonValue,
}

#[derive(Serialize)]
struct IndicatorOut {
    code: String,
    libelle: Option<String>,
    expression: String,
    grain: Vec<String>,
    format: Option<String>,
}

#[derive(Deserialize)]
struct IndicatorBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    expression: String,
    #[serde(default)]
    grain: Vec<String>,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Deserialize)]
struct PreviewBody {
    expression: String,
    #[serde(default)]
    grain: Vec<String>,
    consolidation_id: i64,
}

#[derive(Serialize)]
struct PreviewOut {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sql: Option<String>,
    rows: Vec<IndicatorRow>,
}

#[derive(Serialize)]
struct OperandOut {
    token: String,
    label: String,
    kind: String, // "poste" | "indicateur"
}

fn definition_to_text(v: &JsonValue) -> Result<String, AppError> {
    match v {
        JsonValue::String(s) => Ok(s.clone()),
        other => serde_json::to_string(other)
            .map_err(|e| AppError::bad_request(format!("definition illisible : {e}"))),
    }
}

fn text_to_json(s: &str) -> JsonValue {
    serde_json::from_str(s).unwrap_or(JsonValue::Null)
}

// --- Postes (aggregates) ---

async fn list_aggregates(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<AggregateOut>>, AppError> {
    let con = lock_con(&state)?;
    // Charger les lignes brutes (avec ids en JSON).
    let mut stmt = con
        .prepare("SELECT code, libelle, level, definition FROM dim_aggregate ORDER BY code")
        .map_err(db_err)?;
    let raw_rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(db_err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(db_err)?;
    // Dénormaliser (ids → codes) pour l'exposition API.
    let rows = raw_rows
        .into_iter()
        .map(|(code, libelle, level, def_text)| {
            let denorm = denormalize_aggregate_definition(&con, &def_text)
            .unwrap_or(def_text);
            AggregateOut {
                code,
                libelle,
                level,
                definition: text_to_json(&denorm),
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(rows))
}

async fn create_aggregate(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AggregateBody>,
) -> Result<(StatusCode, Json<AggregateOut>), AppError> {
    let def_text = definition_to_text(&body.definition)?;
    // Le niveau vit dans la définition pour la validation : on l'y injecte.
    let def_with_level = inject_level(&def_text, &body.level)?;
    let con = lock_con(&state)?;
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_aggregate WHERE code = ?",
            params![body.code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!("poste {} existe déjà", body.code)));
    }
    validate_aggregate(&con, &def_with_level).map_err(AppError::bad_request)?;
    let def_with_level =
        normalize_aggregate_definition(&con, &def_with_level).map_err(db_err)?;
    con.execute(
        "INSERT INTO dim_aggregate (code, libelle, level, definition) VALUES (?, ?, ?, ?)",
        params![body.code, body.libelle, body.level, def_with_level],
    )
    .map_err(db_err)?;
    Ok((
        StatusCode::CREATED,
        Json(AggregateOut {
            code: body.code,
            libelle: body.libelle,
            level: body.level,
            definition: text_to_json(&def_with_level),
        }),
    ))
}

async fn update_aggregate(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<AggregateBody>,
) -> Result<Json<AggregateOut>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request("le `code` du body ne correspond pas à l'URL"));
    }
    let def_text = definition_to_text(&body.definition)?;
    let def_with_level = inject_level(&def_text, &body.level)?;
    let con = lock_con(&state)?;
    validate_aggregate(&con, &def_with_level).map_err(AppError::bad_request)?;
    let def_with_level =
        normalize_aggregate_definition(&con, &def_with_level).map_err(db_err)?;
    let n = con
        .execute(
            "UPDATE dim_aggregate SET libelle = ?, level = ?, definition = ? WHERE code = ?",
            params![body.libelle, body.level, def_with_level, code],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!("poste {code} introuvable")));
    }
    Ok(Json(AggregateOut {
        code,
        libelle: body.libelle,
        level: body.level,
        definition: text_to_json(&def_with_level),
    }))
}

async fn delete_aggregate(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    con.execute("DELETE FROM dim_aggregate WHERE code = ?", params![code])
        .map_err(db_err)?;
    Ok(Json(serde_json::json!({ "status": "ok", "deleted": code })))
}

/// Force la clé `level` dans le JSON de définition (source de vérité = colonne).
fn inject_level(def_text: &str, level: &str) -> Result<String, AppError> {
    let mut v: JsonValue = serde_json::from_str(def_text)
        .map_err(|e| AppError::bad_request(format!("definition JSON invalide : {e}")))?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("level".to_string(), JsonValue::String(level.to_string()));
    }
    serde_json::to_string(&v).map_err(|e| AppError::bad_request(e.to_string()))
}

// --- Indicateurs ---

async fn list_indicators(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<IndicatorOut>>, AppError> {
    let con = lock_con(&state)?;
    let mut stmt = con
        .prepare("SELECT code, libelle, expression, grain, format FROM dim_indicator ORDER BY code")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            let grain_json: Option<String> = r.get(3)?;
            let grain: Vec<String> = grain_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            Ok(IndicatorOut {
                code: r.get(0)?,
                libelle: r.get(1)?,
                expression: r.get(2)?,
                grain,
                format: r.get(4)?,
            })
        })
        .map_err(db_err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(db_err)?;
    Ok(Json(rows))
}

async fn create_indicator(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IndicatorBody>,
) -> Result<(StatusCode, Json<IndicatorOut>), AppError> {
    let con = lock_con(&state)?;
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_indicator WHERE code = ?",
            params![body.code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!(
            "indicateur {} existe déjà",
            body.code
        )));
    }
    validate_indicator(&con, &body.expression, &body.grain).map_err(AppError::bad_request)?;
    let expression =
        normalize_indicator_expression(&con, &body.expression).map_err(db_err)?;
    let grain_json = serde_json::to_string(&body.grain).unwrap_or_else(|_| "[]".into());
    con.execute(
        "INSERT INTO dim_indicator (code, libelle, expression, grain, format) VALUES (?,?,?,?,?)",
        params![body.code, body.libelle, expression, grain_json, body.format],
    )
    .map_err(db_err)?;
    Ok((
        StatusCode::CREATED,
        Json(IndicatorOut {
            code: body.code,
            libelle: body.libelle,
            expression: body.expression,
            grain: body.grain,
            format: body.format,
        }),
    ))
}

async fn update_indicator(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<IndicatorBody>,
) -> Result<Json<IndicatorOut>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request("le `code` du body ne correspond pas à l'URL"));
    }
    let con = lock_con(&state)?;
    validate_indicator(&con, &body.expression, &body.grain).map_err(AppError::bad_request)?;
    let expression =
        normalize_indicator_expression(&con, &body.expression).map_err(db_err)?;
    let grain_json = serde_json::to_string(&body.grain).unwrap_or_else(|_| "[]".into());
    let n = con
        .execute(
            "UPDATE dim_indicator SET libelle=?, expression=?, grain=?, format=? WHERE code=?",
            params![body.libelle, expression, grain_json, body.format, code],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!("indicateur {code} introuvable")));
    }
    Ok(Json(IndicatorOut {
        code,
        libelle: body.libelle,
        expression: body.expression,
        grain: body.grain,
        format: body.format,
    }))
}

async fn delete_indicator(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    con.execute("DELETE FROM dim_indicator WHERE code = ?", params![code])
        .map_err(db_err)?;
    Ok(Json(serde_json::json!({ "status": "ok", "deleted": code })))
}

/// POST /api/indicators/preview — compile + exécute une formule non sauvegardée.
async fn preview(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PreviewBody>,
) -> Result<Json<PreviewOut>, AppError> {
    let con = lock_con(&state)?;
    // Compilation d'abord (donne le SQL même en cas d'erreur d'exécution).
    let compiled = compile_indicator(&con, &body.expression, &body.grain, body.consolidation_id);
    match compiled {
        Err(e) => Ok(Json(PreviewOut {
            ok: false,
            error: Some(e),
            sql: None,
            rows: Vec::new(),
        })),
        Ok((sql, _, _)) => match run_indicator(&con, &body.expression, &body.grain, body.consolidation_id) {
            Ok(rows) => Ok(Json(PreviewOut {
                ok: true,
                error: None,
                sql: Some(sql),
                rows,
            })),
            Err(e) => Ok(Json(PreviewOut {
                ok: false,
                error: Some(e),
                sql: Some(sql),
                rows: Vec::new(),
            })),
        },
    }
}

/// GET /api/indicators/operands — postes + indicateurs référençables dans une formule.
async fn operands_catalog(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<OperandOut>>, AppError> {
    let con = lock_con(&state)?;
    let mut out = Vec::new();
    {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_aggregate ORDER BY code")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OperandOut {
                    token: r.get(0)?,
                    label: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    kind: "poste".to_string(),
                })
            })
            .map_err(db_err)?;
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
    }
    {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_indicator ORDER BY code")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(OperandOut {
                    token: r.get(0)?,
                    label: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    kind: "indicateur".to_string(),
                })
            })
            .map_err(db_err)?;
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
    }
    Ok(Json(out))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/aggregates", get(list_aggregates).post(create_aggregate))
        .route(
            "/api/aggregates/{code}",
            axum::routing::put(update_aggregate).delete(delete_aggregate),
        )
        .route("/api/indicators", get(list_indicators).post(create_indicator))
        .route("/api/indicators/operands", get(operands_catalog))
        .route("/api/indicators/preview", post(preview))
        .route(
            "/api/indicators/{code}",
            axum::routing::put(update_indicator).delete(delete_indicator),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let con = Connection::open_in_memory().unwrap();
        crate::schema::create_schema(&con).unwrap();
        crate::seed::seed_all(&con).unwrap();
        con
    }

    fn mk_aggregate(con: &Connection, code: &str, level: &str, selection: &str) {
        let def = format!(r#"{{"level":"{level}","selection":{selection}}}"#);
        con.execute(
            "INSERT INTO dim_aggregate (code, libelle, level, definition) VALUES (?,?,?,?)",
            params![code, code, level, def],
        )
        .unwrap();
    }

    #[test]
    fn poste_direct_compile_et_filter() {
        let con = db();
        mk_aggregate(
            &con,
            "ca",
            "consolidated",
            r#"[{"dim":"account","op":"=","val":"700"}]"#,
        );
        let (sql, _, _) = compile_indicator(&con, "[ca]", &[], 1).unwrap();
        assert!(sql.contains("FILTER (WHERE e.level = 'consolidated'"));
        // Sous B1 `e.account` est un id : la sélection directe joint la master
        // et filtre sur le code.
        assert!(sql.contains("imdd_account.code = ?"));
        assert!(sql.contains("LEFT JOIN dim_account imdd_account ON imdd_account.id = e.account"));
    }

    #[test]
    fn poste_attr_classe_ajoute_un_join() {
        let con = db();
        mk_aggregate(
            &con,
            "resultat",
            "consolidated",
            r#"[{"dim":"account","op":"=","val":"resultat","attr":"classe"}]"#,
        );
        let (sql, _, _) = compile_indicator(&con, "[resultat]", &[], 1).unwrap();
        assert!(sql.contains("LEFT JOIN"));
        assert!(sql.contains("imda_account_classe.classe"));
    }

    #[test]
    fn indicateur_ratio_combine_deux_postes() {
        let con = db();
        mk_aggregate(&con, "num", "consolidated", r#"[{"dim":"account","op":"=","val":"700"}]"#);
        mk_aggregate(&con, "den", "consolidated", r#"[{"dim":"account","op":"=","val":"100"}]"#);
        let (sql, prm, _) =
            compile_indicator(&con, "SAFE_DIV([num]; [den])", &["entity".into()], 1).unwrap();
        assert!(sql.contains("GROUP BY entity"));
        // 2 valeurs de FILTER (700, 100) + consolidation_id = 3 params.
        assert_eq!(prm.len(), 3);
    }

    #[test]
    fn execute_indicateur_sur_seed() {
        let con = db();
        // Lance le pipeline pour peupler fact_entry (consolidation 1 du seed).
        let params = crate::ConvertParams::load_params(&con, 1).unwrap();
        crate::run_pipeline(&con, &params).unwrap();
        mk_aggregate(
            &con,
            "ca",
            "consolidated",
            r#"[{"dim":"account","op":"=","val":"700","attr":null}]"#,
        );
        // Indicateur trivial : la somme du CA, grain global.
        let rows = run_indicator(&con, "[ca]", &[], 1).unwrap();
        assert_eq!(rows.len(), 1);
        // Une valeur numérique est calculée (peu importe le montant exact ici).
        assert!(rows[0].value.is_some());
    }

    #[test]
    fn poste_inconnu_rejete() {
        let con = db();
        assert!(compile_indicator(&con, "[nexiste_pas]", &[], 1).is_err());
    }

    #[test]
    fn cycle_indicateur_detecte() {
        let con = db();
        con.execute(
            "INSERT INTO dim_indicator (code, libelle, expression, grain, format) VALUES ('a','a','[b]','[]',NULL)",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_indicator (code, libelle, expression, grain, format) VALUES ('b','b','[a]','[]',NULL)",
            [],
        )
        .unwrap();
        let err = compile_indicator(&con, "[a]", &[], 1).unwrap_err();
        assert!(err.contains("cycle"));
    }

    #[test]
    fn grain_invalide_rejete() {
        let con = db();
        mk_aggregate(&con, "ca", "consolidated", r#"[{"dim":"account","op":"=","val":"700"}]"#);
        assert!(compile_indicator(&con, "[ca]", &["pas_une_dim".into()], 1).is_err());
    }
}
