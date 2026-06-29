//! Contrôles de données — vérifications configurables exécutées à la demande.
//!
//! Un contrôle sélectionne des données à un ou plusieurs niveaux (raw, corporate,
//! converted, consolidated), les agrège par grain, et évalue des assertions
//! (seuils, non-nullité, existence). Optionnellement compare N vs N-1.
//!
//! Spec : `docs/CONTROLES_DONNEES.md`.

use crate::state::{db_err, lock_con, AppError, AppState};
use crate::{characteristics, custom_references, dimensions, references};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use duckdb::{params, params_from_iter, types::Value as DbValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
//  Whitelists
// ─────────────────────────────────────────────────────────────────────────────

const ALLOWED_LEVELS: &[&str] = &["raw", "corporate", "converted", "consolidated"];
#[allow(dead_code)]
const PIPELINE_LEVELS: &[&str] = &["corporate", "converted", "consolidated"];
const ALLOWED_OPS: &[&str] = &[
    "=", "!=", ">", "<", ">=", "<=", "IN", "IS NULL", "IS NOT NULL",
];
#[allow(dead_code)]
const ALLOWED_ASSERTION_TYPES: &[&str] = &["range", "nonzero", "existence", "equals"];
const ALLOWED_METRICS: &[&str] = &["variation_abs", "variation_pct", "variation"];

// ─────────────────────────────────────────────────────────────────────────────
//  Structures JSON (sérialisées en DB)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlDefinition {
    pub levels: Vec<String>,
    #[serde(default)]
    pub grain: Vec<String>,
    #[serde(default)]
    pub selection: Vec<SelectionCond>,
    pub expression: Option<String>,
    pub assertions: Vec<Assertion>,
    pub compare: Option<Compare>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionCond {
    pub dim: String,
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub val: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "ref")]
    pub ref_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Assertion {
    #[serde(rename = "range")]
    Range { warn: f64, error: f64 },
    #[serde(rename = "nonzero")]
    Nonzero,
    #[serde(rename = "existence")]
    Existence,
    #[serde(rename = "equals")]
    Equals { target: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compare {
    pub metric: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_consolidation_id: Option<i64>,
    pub warn: f64,
    pub error: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Rapport de résultats
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Error,
    NoData,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlRowResult {
    pub grain: BTreeMap<String, Option<String>>,
    pub value: Option<f64>,
    pub baseline: Option<f64>,
    pub variation: Option<f64>,
    pub status: Status,
    pub row_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlLevelResult {
    pub status: Status,
    pub rows: Vec<ControlRowResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlReport {
    pub control_code: String,
    pub control_libelle: Option<String>,
    pub levels: BTreeMap<String, ControlLevelResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlSetReport {
    pub set_code: String,
    pub executed_at: String,
    pub consolidation_id: Option<i64>,
    pub phase: Option<String>,
    pub entry_period: Option<String>,
    pub summary: Summary,
    pub details: Vec<ControlReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total: usize,
    pub by_level: BTreeMap<String, LevelSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LevelSummary {
    pub pass: usize,
    pub warn: usize,
    pub error: usize,
    pub no_data: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Validation
// ─────────────────────────────────────────────────────────────────────────────

/// Valide une définition de contrôle (appelée à la création/modification).
pub fn validate_definition(con: &Connection, def: &ControlDefinition) -> Result<(), String> {
    // levels
    if def.levels.is_empty() {
        return Err("levels doit contenir au moins 1 valeur".into());
    }
    for l in &def.levels {
        if !ALLOWED_LEVELS.contains(&l.as_str()) {
            return Err(format!(
                "level invalide : {l} (attendu parmi {ALLOWED_LEVELS:?})"
            ));
        }
    }

    // grain
    let all_dims = propagated_dims(con);
    for g in &def.grain {
        if !all_dims.contains(g) {
            return Err(format!("grain : dimension inconnue '{g}'"));
        }
    }

    // selection
    let sel_dims = selection_dims(con);
    for s in &def.selection {
        validate_sel_cond(s, &sel_dims, con)?;
    }

    // expression (optionnelle — on ne passe PAS par le formule compiler pour
    // les contrôles : l'expression est du SQL brut (SUM, CASE, IF, etc.) injecté
    // directement dans la requête. La validation se fait à l'exécution.
    // On vérifie juste que l'expression n'est pas vide.
    if let Some(expr) = &def.expression {
        if expr.trim().is_empty() {
            return Err("expression ne peut pas être vide".into());
        }
    }

    // assertions
    if def.assertions.is_empty() {
        return Err("assertions doit contenir au moins 1 élément".into());
    }

    // compare
    if let Some(cmp) = &def.compare {
        if !ALLOWED_METRICS.contains(&cmp.metric.as_str()) {
            return Err(format!(
                "compare.metric invalide : {} (attendu parmi {ALLOWED_METRICS:?})",
                cmp.metric
            ));
        }
        // Interdit si le seul niveau est raw
        if def.levels.iter().all(|l| l == "raw") {
            return Err("compare n'est pas applicable quand le seul niveau est 'raw'".into());
        }
    }

    Ok(())
}

fn validate_sel_cond(
    s: &SelectionCond,
    allowed_dims: &[String],
    con: &Connection,
) -> Result<(), String> {
    if !allowed_dims.contains(&s.dim) {
        return Err(format!("selection.dim invalide : {}", s.dim));
    }
    if !ALLOWED_OPS.contains(&s.op.as_str()) {
        return Err(format!("selection.op invalide : {}", s.op));
    }
    if s.val.is_none() && s.op != "IS NULL" && s.op != "IS NOT NULL" {
        return Err(format!("selection.val manquant pour op='{}'", s.op));
    }
    // Traversées mutuellement exclusives
    if [s.via.is_some(), s.ref_field.is_some(), s.attr.is_some()]
        .iter()
        .filter(|&&b| b)
        .count()
        > 1
    {
        return Err(format!(
            "selection.{} : via / ref / attr sont mutuellement exclusifs",
            s.dim
        ));
    }
    // Validation référentielle (comme rules::validate_definition)
    if let Some(via) = &s.via {
        match characteristics::base_dimension_of(con, via).map_err(|e| e.to_string())? {
            Some(base) if base == s.dim => {}
            Some(other) => {
                return Err(format!(
                    "selection.{} via : la caractéristique '{}' a pour base '{}', pas '{}'",
                    s.dim, via, other, s.dim
                ));
            }
            None => {
                return Err(format!(
                    "selection.{} via : caractéristique inconnue : {}",
                    s.dim, via
                ));
            }
        }
    }
    if let Some(rf) = &s.ref_field {
        match custom_references::target_of(con, &s.dim, rf).map_err(|e| e.to_string())? {
            Some(_) => {}
            None => {
                return Err(format!(
                    "selection.{} ref : référence inconnue : {}.{}",
                    s.dim, s.dim, rf
                ));
            }
        }
    }
    if let Some(attr) = &s.attr {
        if references::native_enum_lookup(&s.dim, attr).is_none() {
            return Err(format!(
                "selection.{} attr : enum natif inconnu : {}.{}",
                s.dim, s.dim, attr
            ));
        }
    }
    // Valeur référentielle
    if s.op != "IS NULL" && s.op != "IS NOT NULL" {
        if let Some(val) = &s.val {
            let target = if s.via.is_some() || s.ref_field.is_some() || s.attr.is_some() {
                None // traversée — déjà validée ci-dessus
            } else {
                references::entry_dimension_target(&s.dim)
                    .map(|r| (r.target_table.to_string(), r.target_column.to_string()))
            };
            let target_ref = target.as_ref().map(|(t, c)| (t.as_str(), c.as_str()));
            check_ref_value(con, target_ref, &s.op, val, &format!("selection.{}", s.dim))?;
        }
    }
    Ok(())
}

fn check_ref_value(
    con: &Connection,
    target: Option<(&str, &str)>,
    op: &str,
    val: &JsonValue,
    ctx: &str,
) -> Result<(), String> {
    let (table, col) = match target {
        Some(t) => t,
        None => return Ok(()), // pas de cible référentielle (dimension libre)
    };
    let vals: Vec<String> = match op {
        "IN" => match val {
            JsonValue::Array(a) => a
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            _ => return Err(format!("{ctx}: IN attend un tableau")),
        },
        "=" | "!=" | ">" | "<" | ">=" | "<=" => match val {
            JsonValue::String(s) => vec![s.clone()],
            JsonValue::Number(n) => vec![n.to_string()],
            _ => vec![],
        },
        _ => return Ok(()),
    };
    for v in &vals {
        if !references::value_exists(con, table, col, v).map_err(|e| e.to_string())? {
            return Err(format!("{ctx}: '{v}' inexistant dans {table}.{col}"));
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers dimensions
// ─────────────────────────────────────────────────────────────────────────────

fn propagated_dims(con: &Connection) -> Vec<String> {
    dimensions::load_all(con)
        .map(|dims| dims.iter().map(|d| d.name.clone()).collect())
        .unwrap_or_default()
}

fn selection_dims(con: &Connection) -> Vec<String> {
    let mut dims = propagated_dims(con);
    dims.push("level".to_string());
    dims
}

#[allow(dead_code)]
fn grain_columns(con: &Connection) -> Vec<String> {
    // Colonnes de fact_entry correspondant aux dimensions propagées (INTEGER ids).
    // Pour le grain on utilise les noms de colonnes de fact_entry (= noms de dimensions).
    propagated_dims(con)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Exécution d'un contrôle
// ─────────────────────────────────────────────────────────────────────────────

/// Paramètres d'exécution passés par l'appelant.
#[derive(Debug, Clone)]
pub struct RunParams {
    pub consolidation_id: Option<i64>,
    pub phase: Option<String>,
    pub entry_period: Option<String>,
}

/// Résultat brut d'une ligne de contrôle (avant évaluation des assertions).
#[derive(Debug)]
struct RawRow {
    grain: BTreeMap<String, Option<String>>,
    value: Option<f64>,
    baseline: Option<f64>,
    variation: Option<f64>,
    row_count: i64,
}

/// Exécute un contrôle pour un niveau donné.
fn run_control_at_level(
    con: &Connection,
    def: &ControlDefinition,
    level: &str,
    params: &RunParams,
) -> Result<Vec<RawRow>, String> {
    if level == "raw" {
        run_raw(con, def, params)
    } else {
        run_pipeline_level(con, def, level, params)
    }
}

/// Exécute sur stg_entry (niveau raw).
fn run_raw(con: &Connection, def: &ControlDefinition, params: &RunParams) -> Result<Vec<RawRow>, String> {
    let phase = params
        .phase
        .as_deref()
        .ok_or("phase requise pour le niveau raw")?;
    let entry_period = params
        .entry_period
        .as_deref()
        .ok_or("entry_period requise pour le niveau raw")?;

    let grain = &def.grain;
    let mut sql_params: Vec<DbValue> = Vec::new();

    // Colonnes de grain (TEXT dans stg_entry)
    let select_grain = if grain.is_empty() {
        "1 AS _dummy_grain".to_string()
    } else {
        grain
            .iter()
            .map(|g| format!("s.{g}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    // Filtres de sélection (sur colonnes TEXT de stg_entry)
    let mut where_clauses = vec!["s.phase = ?".to_string(), "s.entry_period = ?".to_string()];
    sql_params.push(DbValue::Text(phase.to_string()));
    sql_params.push(DbValue::Text(entry_period.to_string()));

    let mut joins = String::new();
    for s in &def.selection {
        let (operand, extra_join) = resolve_raw_sel_operand(con, s)?;
        if let Some(j) = extra_join {
            if !joins.contains(&j) {
                joins.push_str(&j);
            }
        }
        let cond = push_condition(&operand, &s.op, &s.val, &mut sql_params)?;
        where_clauses.push(cond);
    }

    // Réécriture de l'expression : si une sélection utilise `ref:` (ex:
    // account.sous_classe), l'expression peut référencer `sous_classe`
    // directement. On remplace par l'alias de join `r_sous_classe.code` pour
    // que le SQL fonctionne (la colonne brute est un INTEGER id, pas un code).
    let value_expr = rewrite_expr_refs(&def.expression, &def.selection, "s");
    let value_expr = value_expr
        .replace("e.amount", "s.amount")
        .replace("SUM(amount)", "SUM(s.amount)");

    let group = if grain.is_empty() {
        String::new()
    } else {
        format!("\nGROUP BY {}", grain.iter().map(|g| format!("s.{g}")).collect::<Vec<_>>().join(", "))
    };

    let sql = format!(
        "SELECT {select_grain}, {value_expr} AS value, COUNT(*) AS row_count, \
         ARRAY_AGG(s.id) AS sample_ids\nFROM stg_entry s{joins}\nWHERE {}{group}",
        where_clauses.join(" AND ")
    );

    execute_raw_query(con, &sql, &sql_params, grain)
}

/// Exécute sur fact_entry (niveaux corporate/converted/consolidated).
fn run_pipeline_level(
    con: &Connection,
    def: &ControlDefinition,
    level: &str,
    params: &RunParams,
) -> Result<Vec<RawRow>, String> {
    let consolidation_id = params
        .consolidation_id
        .ok_or("consolidation_id requis pour les niveaux pipeline")?;

    let grain = &def.grain;
    let mut sql_params: Vec<DbValue> = Vec::new();

    // Colonnes de grain (INTEGER ids dans fact_entry — on résout les codes via JOINs)
    let mut select_grain_parts = Vec::new();
    let mut grain_joins = String::new();
    if grain.is_empty() {
        select_grain_parts.push("1 AS _dummy_grain".to_string());
    } else {
        for g in grain {
            if let Some((table, code_col)) = references::dimension_master(g) {
                let alias = format!("gr_{g}");
                select_grain_parts.push(format!("{alias}.{code_col} AS {g}"));
                let join = format!("\nLEFT JOIN {table} {alias} ON {alias}.id = e.{g}");
                if !grain_joins.contains(&join) {
                    grain_joins.push_str(&join);
                }
            } else {
                // Dimension libre (analysis, custom) — reste TEXT
                select_grain_parts.push(format!("e.{g} AS {g}"));
            }
        }
    }
    let select_grain = select_grain_parts.join(", ");

    let mut where_clauses = vec![
        "e.consolidation_id = ?".to_string(),
        "e.level = ?".to_string(),
    ];
    sql_params.push(DbValue::BigInt(consolidation_id));
    sql_params.push(DbValue::Text(level.to_string()));

    // Filtres de sélection (avec résolution code→id comme indicators.rs)
    let mut joins = String::new();
    for s in &def.selection {
        let (operand, extra_join) =
            resolve_sel_operand(con, s).map_err(|e| format!("selection.{}: {e}", s.dim))?;
        if let Some(j) = extra_join {
            if !joins.contains(&j) {
                joins.push_str(&j);
            }
        }
        let cond = push_condition(&operand, &s.op, &s.val, &mut sql_params)?;
        where_clauses.push(cond);
    }

    // Expression (par défaut SUM(e.amount))
    // Même remplacement que raw pour que l'utilisateur puisse écrire `amount`
    // (sans préfixe) et que ça fonctionne à tous les niveaux.
    // Réécrit aussi les références `ref:` (ex: sous_classe → r_sous_classe.code).
    let value_expr = rewrite_expr_refs(&def.expression, &def.selection, "e");
    let value_expr = value_expr
        .replace("s.amount", "e.amount")
        .replace("SUM(amount)", "SUM(e.amount)");

    let group = if grain.is_empty() {
        String::new()
    } else {
        format!(
            "\nGROUP BY {}",
            grain
                .iter()
                .map(|g| {
                    if let Some((_table, code_col)) = references::dimension_master(g) {
                        let alias = format!("gr_{g}");
                        format!("{alias}.{code_col}")
                    } else {
                        format!("e.{g}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let sql = format!(
        "SELECT {select_grain}, {value_expr} AS value, COUNT(*) AS row_count, \
         ARRAY_AGG(e.id) AS sample_ids\nFROM fact_entry e{grain_joins}{joins}\nWHERE {}{group}",
        where_clauses.join(" AND ")
    );

    execute_raw_query(con, &sql, &sql_params, grain)
}

/// Résout l'opérande d'une sélection pour fact_entry (INTEGER ids).
/// Retourne (sql_operand, optional_join).
fn resolve_sel_operand(
    con: &Connection,
    s: &SelectionCond,
) -> Result<(String, Option<String>), String> {
    if let Some(via) = &s.via {
        // Caractéristique N1
        let char_id = characteristics::id_of(con, via)
            .ok_or_else(|| format!("caractéristique '{via}' sans id technique"))?;
        let val_table = characteristics::value_table(char_id);
        let base = characteristics::base_dimension_of(con, via)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("caractéristique '{via}' sans base"))?;
        let join = format!(
            "\nLEFT JOIN {val_table} c_{via} ON c_{via}.{base} = e.{base}",
        );
        Ok((format!("c_{via}.code"), Some(join)))
    } else if let Some(rf) = &s.ref_field {
        // Référence directe (ex: account.sous_classe, entity.entite_parent)
        // fact_entry.account = id INTEGER → JOIN dim_account → dim_sous_classe
        let dim = &s.dim;
        let target_dim = custom_references::target_of(con, dim, rf)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("référence {dim}.{rf} inconnue"))?;
        let (host_table, _) = references::dimension_master(dim)
            .ok_or_else(|| format!("dimension sans master data : {dim}"))?;
        let join = format!("\nLEFT JOIN {host_table} r_{dim} ON r_{dim}.id = e.{dim}");
        // Cible : résoudre table + colonne code
        let (target_table, target_code) = references::target_master(con, &target_dim)
            .ok_or_else(|| format!("dimension cible sans master data : {target_dim}"))?;
        let join2 = format!(
            "\nLEFT JOIN {target_table} r_{rf} ON r_{rf}.id = r_{dim}.{rf}"
        );
        Ok((format!("r_{rf}.{target_code}"), Some(format!("{join}{join2}"))))
    } else if let Some(attr) = &s.attr {
        // Enum natif
        let dim = &s.dim;
        let (mt, _) = references::dimension_master(dim)
            .ok_or_else(|| format!("dimension sans master data : {dim}"))?;
        let join = format!(
            "\nLEFT JOIN {mt} a_{dim}_{attr} ON a_{dim}_{attr}.id = e.{dim}"
        );
        Ok((format!("a_{dim}_{attr}.{attr}"), Some(join)))
    } else {
        // Dimension directe — sous B1 c'est un INTEGER id, on joint la master
        // pour filtrer sur le code.
        let dim = &s.dim;
        if let Some((mt, _)) = references::dimension_master_id_join(dim) {
            let (_, code_col) = references::dimension_master(dim)
                .ok_or_else(|| format!("dimension sans master data : {dim}"))?;
            let join = format!("\nLEFT JOIN {mt} md_{dim} ON md_{dim}.id = e.{dim}");
            Ok((format!("md_{dim}.{code_col}"), Some(join)))
        } else {
            // Dimension libre (analysis, custom) — reste TEXT
            Ok((format!("e.{dim}"), None))
        }
    }
}

/// Résout l'opérande d'une sélection pour stg_entry (TEXT codes).
/// Gère les sélections directes (colonne sur stg_entry) et les références
/// (traversée via `ref:` — JOIN sur la master data hôte puis filtre sur la
/// table cible). Les alias JOIN sont harmonisés avec `resolve_sel_operand`
/// (pipeline) pour que les expressions fonctionnent aux deux niveaux.
fn resolve_raw_sel_operand(
    con: &Connection,
    s: &SelectionCond,
) -> Result<(String, Option<String>), String> {
    if let Some(rf) = &s.ref_field {
        // Référence directe : account.sous_classe, entity.entite_parent, etc.
        // stg_entry.account = code TEXT → JOIN dim_account → dim_sous_classe
        let dim = &s.dim;
        let target_dim = custom_references::target_of(con, dim, rf)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("référence {dim}.{rf} inconnue"))?;
        let (host_table, _) = references::dimension_master(dim)
            .ok_or_else(|| format!("dimension sans master data : {dim}"))?;
        // stg_entry stocke des codes TEXT → JOIN sur la colonne code de la table hôte
        let join = format!("\nLEFT JOIN {host_table} r_{dim} ON r_{dim}.code = s.{dim}");
        // Cible : résoudre table + colonne code (secondaire ou primaire).
        // Alias = r_{dim} (même convention que pipeline) ; la colonne filtrée
        // est la colonne code de la table cible.
        let (target_table, target_code) = references::target_master(con, &target_dim)
            .ok_or_else(|| format!("dimension cible sans master data : {target_dim}"))?;
        let join2 = format!(
            "\nLEFT JOIN {target_table} r_{rf} ON r_{rf}.id = r_{dim}.{rf}"
        );
        Ok((format!("r_{rf}.{target_code}"), Some(format!("{join}{join2}"))))
    } else if let Some(attr) = &s.attr {
        // Enum natif (classe, etc.) — colonne directe sur la table hôte
        let dim = &s.dim;
        let (mt, _) = references::dimension_master(dim)
            .ok_or_else(|| format!("dimension sans master data : {dim}"))?;
        let join = format!("\nLEFT JOIN {mt} a_{dim} ON a_{dim}.code = s.{dim}");
        Ok((format!("a_{dim}.{attr}"), Some(join)))
    } else if let Some(via) = &s.via {
        // Caractéristique N1
        let _dim = &s.dim;
        let char_id = characteristics::id_of(con, via)
            .ok_or_else(|| format!("caractéristique '{via}' sans id technique"))?;
        let val_table = characteristics::value_table(char_id);
        let base = characteristics::base_dimension_of(con, via)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("caractéristique '{via}' sans base"))?;
        // stg_entry stocke des codes TEXT → JOIN sur code
        let join = format!("\nLEFT JOIN {val_table} c_{via} ON c_{via}.{base} = s.{base}");
        Ok((format!("c_{via}.code"), Some(join)))
    } else {
        // Sélection directe : colonne sur stg_entry
        Ok((format!("s.{}", s.dim), None))
    }
}

/// Réécrit les références de colonnes dans une expression de contrôle.
///
/// Quand une sélection utilise `ref:` (ex: `account.sous_classe`), la colonne
/// cible (`sous_classe`) est un INTEGER id dans la table de fait. Si
/// l'expression référence cette colonne directement (ex: `sous_classe = 'actif'`),
/// le SQL échoue (conversion string → INT). On remplace par l'alias de join
/// `r_{ref}.code` (ex: `r_sous_classe.code = 'actif'`).
///
/// `table_prefix` est le préfixe de la table de fait (`s` pour raw, `e` pour pipeline).
fn rewrite_expr_refs(
    expr: &Option<String>,
    selection: &[SelectionCond],
    table_prefix: &str,
) -> String {
    let Some(expr) = expr else {
        return format!("SUM({table_prefix}.amount)");
    };
    let mut out = expr.clone();
    for s in selection {
        if let Some(rf) = &s.ref_field {
            let dim = rf.as_str();
            // Remplacer le nom de colonne comme mot isolé par r_{dim}.code.
            // On NE remplace PAS les occurrences préfixées par `.` ou `_`.
            let patterns = [
                (format!(" {dim} "), format!(" r_{dim}.code ")),
                (format!("({dim} "), format!("(r_{dim}.code ")),
                (format!(" {dim})"), format!(" r_{dim}.code)")),
                (format!(" {dim}="), format!(" r_{dim}.code=")),
                (format!("({dim}="), format!("(r_{dim}.code=")),
                (format!(" {dim}!"), format!(" r_{dim}.code!")),
                (format!(" {dim}>"), format!(" r_{dim}.code>")),
                (format!(" {dim}<"), format!(" r_{dim}.code<")),
                (format!("({dim},"), format!("(r_{dim}.code,")),
                (format!(" {dim},"), format!(" r_{dim}.code,")),
            ];
            for (from, to) in &patterns {
                out = out.replace(from, to);
            }
            // Cas début de chaîne
            if out.starts_with(dim) {
                let rest = &out[dim.len()..];
                if rest.starts_with(|c: char| c == ' ' || c == '=' || c == '!' || c == '>' || c == '<' || c == ')' || c == ',') {
                    out = format!("r_{dim}.code{}", &out[dim.len()..]);
                }
            }
        }
    }
    out
}

fn push_condition(
    operand: &str,
    op: &str,
    val: &Option<JsonValue>,
    params: &mut Vec<DbValue>,
) -> Result<String, String> {
    match op {
        "IS NULL" => Ok(format!("{operand} IS NULL")),
        "IS NOT NULL" => Ok(format!("{operand} IS NOT NULL")),
        "=" | "!=" | ">" | "<" | ">=" | "<=" => {
            let v = val.as_ref().ok_or_else(|| format!("val manquant pour op='{op}'"))?;
            let db_val = json_to_db_value(v)?;
            params.push(db_val);
            Ok(format!("{operand} {op} ?"))
        }
        "IN" => {
            let v = val.as_ref().ok_or_else(|| "val manquant pour IN".to_string())?;
            let arr = v.as_array().ok_or("IN attend un tableau")?;
            if arr.is_empty() {
                return Ok("FALSE".to_string());
            }
            let placeholders: Vec<String> = arr.iter().map(|_| "?".to_string()).collect();
            for item in arr {
                params.push(json_to_db_value(item)?);
            }
            Ok(format!("{operand} IN ({})", placeholders.join(", ")))
        }
        _ => Err(format!("opérateur inconnu : {op}")),
    }
}

fn json_to_db_value(v: &JsonValue) -> Result<DbValue, String> {
    match v {
        JsonValue::String(s) => Ok(DbValue::Text(s.clone())),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(DbValue::BigInt(i))
            } else if let Some(f) = n.as_f64() {
                Ok(DbValue::Double(f))
            } else {
                Err(format!("nombre non supporté : {n}"))
            }
        }
        _ => Err(format!("valeur non supportée : {v}")),
    }
}

/// Exécute une requête SQL et mappe les résultats en RawRow.
fn execute_raw_query(
    con: &Connection,
    sql: &str,
    sql_params: &[DbValue],
    grain: &[String],
) -> Result<Vec<RawRow>, String> {
    let mut stmt = con.prepare(sql).map_err(|e| format!("SQL invalide : {e}"))?;
    let rows = stmt
        .query_map(params_from_iter(sql_params.iter()), |r| {
            let mut grain_vals = BTreeMap::new();
            for (i, g) in grain.iter().enumerate() {
                let v: Option<String> = r.get(i).ok();
                grain_vals.insert(g.clone(), v);
            }
            let value: Option<f64> = r.get(grain.len()).ok();
            let row_count: i64 = r.get(grain.len() + 1).ok().unwrap_or(0);
            Ok(RawRow {
                grain: grain_vals,
                value,
                baseline: None,
                variation: None,
                row_count,
            })
        })
        .map_err(|e| format!("erreur requête : {e}"))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("erreur ligne : {e}"))?);
    }
    Ok(result)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Comparaison inter-périodes
// ─────────────────────────────────────────────────────────────────────────────

/// Enrichit les résultats avec la baseline et la variation.
fn enrich_with_comparison(
    con: &Connection,
    def: &ControlDefinition,
    level: &str,
    params: &RunParams,
    rows: &mut [RawRow],
) -> Result<(), String> {
    let cmp = match &def.compare {
        Some(c) => c,
        None => return Ok(()),
    };
    // Pas de comparaison sur raw
    if level == "raw" {
        return Ok(());
    }

    let baseline_id = match cmp.baseline_consolidation_id {
        Some(id) => id,
        None => {
            // Déduire N-1 : même phase, exercice -1
            let cid = params
                .consolidation_id
                .ok_or("consolidation_id requis pour la comparaison")?;
            find_baseline_consolidation(con, cid)?
        }
    };

    let mut baseline_params = params.clone();
    baseline_params.consolidation_id = Some(baseline_id);
    let baseline_rows = run_pipeline_level(con, def, level, &baseline_params)?;

    // Indexer par grain
    let baseline_map: BTreeMap<String, Option<f64>> = baseline_rows
        .into_iter()
        .map(|r| (grain_key(&r.grain), r.value))
        .collect();

    for row in rows.iter_mut() {
        let key = grain_key(&row.grain);
        let base_val = baseline_map.get(&key).and_then(|v| *v);
        row.baseline = base_val;
        if let (Some(cur), Some(base)) = (row.value, base_val) {
            row.variation = Some(match cmp.metric.as_str() {
                "variation_abs" => (cur - base).abs(),
                "variation_pct" => {
                    if base == 0.0 {
                        0.0
                    } else {
                        ((cur - base) / base.abs()) * 100.0
                    }
                }
                "variation" => cur - base,
                _ => 0.0,
            });
        }
    }
    Ok(())
}

fn grain_key(grain: &BTreeMap<String, Option<String>>) -> String {
    grain
        .values()
        .map(|v| v.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("|")
}

/// Trouve la consolidation N-1 (même phase, exercice précédent).
fn find_baseline_consolidation(con: &Connection, consolidation_id: i64) -> Result<i64, String> {
    con.query_row(
        "SELECT b.id FROM dim_consolidation a \
         JOIN dim_consolidation b \
           ON a.phase = b.phase \
           AND a.perimeter_set = b.perimeter_set \
           AND a.variant = b.variant \
           AND a.presentation_currency = b.presentation_currency \
           AND CAST(a.exercice AS INTEGER) = CAST(b.exercice AS INTEGER) + 1 \
         WHERE a.id = ? \
         LIMIT 1",
        params![consolidation_id],
        |r| r.get(0),
    )
    .map_err(|e| format!("baseline N-1 introuvable : {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Évaluation des assertions
// ─────────────────────────────────────────────────────────────────────────────

fn evaluate_assertions(assertions: &[Assertion], row: &RawRow) -> Status {
    if row.value.is_none() {
        return Status::NoData;
    }
    let value = row.value.unwrap();
    let mut worst = Status::Pass;
    for a in assertions {
        let s = match a {
            Assertion::Range { warn, error } => {
                if value.abs() > *error {
                    Status::Error
                } else if value.abs() > *warn {
                    Status::Warn
                } else {
                    Status::Pass
                }
            }
            Assertion::Nonzero => {
                if value.abs() < 0.005 {
                    Status::Error
                } else {
                    Status::Pass
                }
            }
            Assertion::Existence => Status::Pass, // déjà géré par value.is_none()
            Assertion::Equals { target } => {
                if (value - target).abs() > 0.01 {
                    Status::Error
                } else {
                    Status::Pass
                }
            }
        };
        if s > worst {
            worst = s;
        }
    }
    // Vérifier aussi la comparaison
    if let Some(variation) = row.variation {
        // Chercher une assertion range sur la variation
        for a in assertions {
            if let Assertion::Range { warn, error } = a {
                let s = if variation.abs() > *error {
                    Status::Error
                } else if variation.abs() > *warn {
                    Status::Warn
                } else {
                    Status::Pass
                };
                if s > worst {
                    worst = s;
                }
            }
        }
    }
    worst
}

// ─────────────────────────────────────────────────────────────────────────────
//  API publique : exécution d'un contrôle et d'un jeu
// ─────────────────────────────────────────────────────────────────────────────

/// Exécute un contrôle individuel.
pub fn run_control(
    con: &Connection,
    code: &str,
    params: &RunParams,
) -> Result<ControlReport, String> {
    let (libelle, def_json) = con
        .query_row(
            "SELECT libelle, definition FROM dim_control WHERE code = ?",
            params![code],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|e| format!("contrôle '{code}' introuvable : {e}"))?;

    let def: ControlDefinition =
        serde_json::from_str(&def_json).map_err(|e| format!("JSON invalide : {e}"))?;

    let mut levels = BTreeMap::new();

    for level in &def.levels {
        // Les niveaux pipeline (corporate, converted, consolidated) nécessitent
        // un consolidation_id. Si absent, on saute silencieusement.
        if level != "raw" && params.consolidation_id.is_none() {
            continue;
        }
        let mut rows = match run_control_at_level(con, &def, level, params) {
            Ok(rows) => rows,
            Err(e) => {
                // En mode set, on ne veut pas faire échouer tout le jeu pour un
                // niveau manquant. On reporte l'erreur comme no_data.
                levels.insert(
                    level.clone(),
                    ControlLevelResult {
                        status: Status::NoData,
                        rows: vec![ControlRowResult {
                            grain: BTreeMap::new(),
                            value: None,
                            baseline: None,
                            variation: None,
                            status: Status::NoData,
                            row_count: 0,
                        }],
                        error: Some(e),
                    },
                );
                continue;
            }
        };

        // Comparaison inter-périodes
        if def.compare.is_some() && level != "raw" {
            enrich_with_comparison(con, &def, level, params, &mut rows)?;
        }

        // Évaluer les assertions
        let result_rows: Vec<ControlRowResult> = rows
            .into_iter()
            .map(|r| {
                let status = evaluate_assertions(&def.assertions, &r);
                ControlRowResult {
                    grain: r.grain,
                    value: r.value,
                    baseline: r.baseline,
                    variation: r.variation,
                    status,
                    row_count: r.row_count,
                }
            })
            .collect();

        let level_status = worst_status(result_rows.iter().map(|r| &r.status));
        levels.insert(
            level.clone(),
            ControlLevelResult {
                status: level_status,
                rows: result_rows,
                error: None,
            },
        );
    }

    Ok(ControlReport {
        control_code: code.to_string(),
        control_libelle: libelle,
        levels,
    })
}

fn worst_status<'a>(statuses: impl Iterator<Item = &'a Status>) -> Status {
    statuses
        .cloned()
        .max()
        .unwrap_or(Status::Pass)
}

/// Exécute un jeu de contrôles.
pub fn run_control_set(
    con: &Connection,
    set_code: &str,
    params: &RunParams,
) -> Result<ControlSetReport, String> {
    // Charger les contrôles du jeu
    let mut stmt = con
        .prepare(
            "SELECT c.code FROM dim_control_set_item i \
             JOIN dim_control c ON c.code = i.control_code \
             WHERE i.set_code = ? ORDER BY i.ord",
        )
        .map_err(|e| format!("erreur préparation : {e}"))?;
    let codes: Vec<String> = stmt
        .query_map(params![set_code], |r| r.get(0))
        .map_err(|e| format!("erreur lecture : {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("erreur collecte : {e}"))?;

    if codes.is_empty() {
        return Err(format!("jeu '{set_code}' vide ou introuvable"));
    }

    let mut details = Vec::new();
    let mut by_level: BTreeMap<String, LevelSummary> = BTreeMap::new();

    for code in &codes {
        let report = run_control(con, code, params)?;
        for (level, lr) in &report.levels {
            let entry = by_level.entry(level.clone()).or_insert(LevelSummary {
                pass: 0,
                warn: 0,
                error: 0,
                no_data: 0,
            });
            for row in &lr.rows {
                match row.status {
                    Status::Pass => entry.pass += 1,
                    Status::Warn => entry.warn += 1,
                    Status::Error => entry.error += 1,
                    Status::NoData => entry.no_data += 1,
                }
            }
        }
        details.push(report);
    }

    let total = by_level
        .values()
        .map(|l| l.pass + l.warn + l.error + l.no_data)
        .sum();

    let now: String = con
        .query_row("SELECT now()::VARCHAR", [], |r| r.get(0))
        .unwrap_or_default();

    Ok(ControlSetReport {
        set_code: set_code.to_string(),
        executed_at: now,
        consolidation_id: params.consolidation_id,
        phase: params.phase.clone(),
        entry_period: params.entry_period.clone(),
        summary: Summary { total, by_level },
        details,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  CRUD handlers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ControlBody {
    code: String,
    libelle: Option<String>,
    definition: ControlDefinition,
}

#[derive(Deserialize)]
struct ControlSetBody {
    code: String,
    libelle: Option<String>,
    controls: Vec<ControlSetItemBody>,
}

#[derive(Deserialize)]
struct ControlSetItemBody {
    code: String,
    ord: Option<i64>,
}

#[derive(Deserialize)]
struct RunBody {
    consolidation_id: Option<i64>,
    phase: Option<String>,
    entry_period: Option<String>,
}

#[derive(Serialize)]
struct ControlOut {
    code: String,
    libelle: Option<String>,
    definition: ControlDefinition,
}

#[derive(Serialize)]
struct ControlSetOut {
    code: String,
    libelle: Option<String>,
    controls: Vec<ControlSetItemOut>,
}

#[derive(Serialize)]
struct ControlSetItemOut {
    code: String,
    libelle: Option<String>,
    ord: i64,
}

async fn list_controls(State(state): State<Arc<AppState>>) -> Result<Json<Vec<ControlOut>>, AppError> {
    let con = lock_con(&state)?;
    let mut stmt = con
        .prepare("SELECT code, libelle, definition FROM dim_control ORDER BY code")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ControlOut {
                code: r.get(0)?,
                libelle: r.get(1)?,
                definition: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
            })
        })
        .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(db_err)?);
    }
    Ok(Json(out))
}

async fn get_control(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<ControlOut>, AppError> {
    let con = lock_con(&state)?;
    con.query_row(
        "SELECT code, libelle, definition FROM dim_control WHERE code = ?",
        params![code],
        |r| {
            Ok(ControlOut {
                code: r.get(0)?,
                libelle: r.get(1)?,
                definition: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
            })
        },
    )
    .map_err(|e| match e {
        duckdb::Error::QueryReturnedNoRows => AppError::not_found(format!("contrôle '{code}'")),
        other => db_err(other),
    })
    .map(Json)
}

async fn create_control(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ControlBody>,
) -> Result<Json<ControlOut>, AppError> {
    let con = lock_con(&state)?;
    validate_definition(&con, &body.definition).map_err(AppError::bad_request)?;
    let def_json =
        serde_json::to_string(&body.definition).map_err(|e| AppError::bad_request(e.to_string()))?;
    con.execute(
        "INSERT INTO dim_control (code, libelle, definition) VALUES (?, ?, ?)",
        params![body.code, body.libelle, def_json],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") || e.to_string().contains("unique") {
            AppError::conflict(format!("contrôle '{}' existe déjà", body.code))
        } else {
            db_err(e)
        }
    })?;
    Ok(Json(ControlOut {
        code: body.code,
        libelle: body.libelle,
        definition: body.definition,
    }))
}

async fn update_control(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<ControlBody>,
) -> Result<Json<ControlOut>, AppError> {
    let con = lock_con(&state)?;
    validate_definition(&con, &body.definition).map_err(AppError::bad_request)?;
    let def_json =
        serde_json::to_string(&body.definition).map_err(|e| AppError::bad_request(e.to_string()))?;
    let rows = con
        .execute(
            "UPDATE dim_control SET libelle = ?, definition = ? WHERE code = ?",
            params![body.libelle, def_json, code],
        )
        .map_err(db_err)?;
    if rows == 0 {
        return Err(AppError::not_found(format!("contrôle '{code}'")));
    }
    Ok(Json(ControlOut {
        code,
        libelle: body.libelle,
        definition: body.definition,
    }))
}

async fn delete_control(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    let rows = con
        .execute("DELETE FROM dim_control WHERE code = ?", params![code])
        .map_err(db_err)?;
    if rows == 0 {
        return Err(AppError::not_found(format!("contrôle '{code}'")));
    }
    Ok(Json(serde_json::json!({ "deleted": code })))
}

async fn run_single_handler(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<RunBody>,
) -> Result<Json<ControlReport>, AppError> {
    let con = lock_con(&state)?;
    let params = RunParams {
        consolidation_id: body.consolidation_id,
        phase: body.phase,
        entry_period: body.entry_period,
    };
    run_control(&con, &code, &params)
        .map(Json)
        .map_err(AppError::bad_request)
}

// ── Jeux de contrôles ──

async fn list_control_sets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ControlSetOut>>, AppError> {
    let con = lock_con(&state)?;
    let mut stmt = con
        .prepare(
            "SELECT s.code, s.libelle FROM dim_control_set s ORDER BY s.code",
        )
        .map_err(db_err)?;
    let sets: Vec<(String, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;

    let mut out = Vec::new();
    for (code, libelle) in sets {
        let items = load_set_items(&con, &code)?;
        out.push(ControlSetOut {
            code,
            libelle,
            controls: items,
        });
    }
    Ok(Json(out))
}

fn load_set_items(con: &Connection, set_code: &str) -> Result<Vec<ControlSetItemOut>, AppError> {
    let mut stmt = con
        .prepare(
            "SELECT i.control_code, c.libelle, i.ord \
             FROM dim_control_set_item i \
             LEFT JOIN dim_control c ON c.code = i.control_code \
             WHERE i.set_code = ? ORDER BY i.ord",
        )
        .map_err(db_err)?;
    let items = stmt
        .query_map(params![set_code], |r| {
            Ok(ControlSetItemOut {
                code: r.get(0)?,
                libelle: r.get(1)?,
                ord: r.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(items)
}

async fn get_control_set(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<ControlSetOut>, AppError> {
    let con = lock_con(&state)?;
    let (set_code, libelle): (String, Option<String>) = con
        .query_row(
            "SELECT code, libelle FROM dim_control_set WHERE code = ?",
            params![code],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            duckdb::Error::QueryReturnedNoRows => {
                AppError::not_found(format!("jeu '{code}'"))
            }
            other => db_err(other),
        })?;
    let items = load_set_items(&con, &set_code)?;
    Ok(Json(ControlSetOut {
        code: set_code,
        libelle,
        controls: items,
    }))
}

async fn create_control_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ControlSetBody>,
) -> Result<Json<ControlSetOut>, AppError> {
    let con = lock_con(&state)?;
    con.execute(
        "INSERT INTO dim_control_set (code, libelle) VALUES (?, ?)",
        params![body.code, body.libelle],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") || e.to_string().contains("unique") {
            AppError::conflict(format!("jeu '{}' existe déjà", body.code))
        } else {
            db_err(e)
        }
    })?;
    for (i, item) in body.controls.iter().enumerate() {
        let ord = item.ord.unwrap_or(i as i64 + 1);
        con.execute(
            "INSERT INTO dim_control_set_item (set_code, control_code, ord) VALUES (?, ?, ?)",
            params![body.code, item.code, ord],
        )
        .map_err(db_err)?;
    }
    let items = load_set_items(&con, &body.code)?;
    Ok(Json(ControlSetOut {
        code: body.code,
        libelle: body.libelle,
        controls: items,
    }))
}

async fn update_control_set(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<ControlSetBody>,
) -> Result<Json<ControlSetOut>, AppError> {
    let con = lock_con(&state)?;
    let rows = con
        .execute(
            "UPDATE dim_control_set SET libelle = ? WHERE code = ?",
            params![body.libelle, code],
        )
        .map_err(db_err)?;
    if rows == 0 {
        return Err(AppError::not_found(format!("jeu '{code}'")));
    }
    con.execute(
        "DELETE FROM dim_control_set_item WHERE set_code = ?",
        params![code],
    )
    .map_err(db_err)?;
    for (i, item) in body.controls.iter().enumerate() {
        let ord = item.ord.unwrap_or(i as i64 + 1);
        con.execute(
            "INSERT INTO dim_control_set_item (set_code, control_code, ord) VALUES (?, ?, ?)",
            params![code, item.code, ord],
        )
        .map_err(db_err)?;
    }
    let items = load_set_items(&con, &code)?;
    Ok(Json(ControlSetOut {
        code,
        libelle: body.libelle,
        controls: items,
    }))
}

async fn delete_control_set(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    con.execute(
        "DELETE FROM dim_control_set_item WHERE set_code = ?",
        params![code],
    )
    .map_err(db_err)?;
    let rows = con
        .execute("DELETE FROM dim_control_set WHERE code = ?", params![code])
        .map_err(db_err)?;
    if rows == 0 {
        return Err(AppError::not_found(format!("jeu '{code}'")));
    }
    Ok(Json(serde_json::json!({ "deleted": code })))
}

async fn run_set_handler(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<RunBody>,
) -> Result<Json<ControlSetReport>, AppError> {
    let con = lock_con(&state)?;
    let params = RunParams {
        consolidation_id: body.consolidation_id,
        phase: body.phase,
        entry_period: body.entry_period,
    };
    run_control_set(&con, &code, &params)
        .map(Json)
        .map_err(AppError::bad_request)
}

async fn get_results_handler(
    State(_state): State<Arc<AppState>>,
    Path(_code): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    // Pour l'instant, pas de stockage des résultats — retourner un placeholder.
    Ok(Json(serde_json::json!({ "message": "pas d'historique stocké" })))
}

async fn operands_catalog(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let con = lock_con(&state)?;
    let mut out = Vec::new();
    // Postes
    {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_aggregate ORDER BY code")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "token": r.get::<_, String>(0)?,
                    "label": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "kind": "poste"
                }))
            })
            .map_err(db_err)?;
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
    }
    // Indicateurs
    {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_indicator ORDER BY code")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "token": r.get::<_, String>(0)?,
                    "label": r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    "kind": "indicateur"
                }))
            })
            .map_err(db_err)?;
        for row in rows {
            out.push(row.map_err(db_err)?);
        }
    }
    Ok(Json(out))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Renommage de code (étape 12 B1)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct RenameBody {
    new: String,
}

/// Renomme le code d'un contrôle + cascade sur `dim_control_set_item.control_code`.
fn rename_control(con: &Connection, old: &str, new: &str) -> Result<(), AppError> {
    if new.is_empty() {
        return Err(AppError::bad_request("nouveau code vide"));
    }
    if new == old {
        return Ok(());
    }
    // Vérifier existence.
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_control WHERE code = ?",
            params![old],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::not_found(format!("contrôle introuvable : {old}")));
    }
    // Vérifier unicité du nouveau code.
    let taken: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_control WHERE code = ?",
            params![new],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if taken {
        return Err(AppError::conflict(format!("code déjà utilisé : {new}")));
    }
    // Cascade sur les items de jeux de contrôles.
    con.execute(
        "UPDATE dim_control_set_item SET control_code = ? WHERE control_code = ?",
        params![new, old],
    )
    .map_err(db_err)?;
    // Renommer le contrôle lui-même.
    con.execute(
        "UPDATE dim_control SET code = ? WHERE code = ?",
        params![new, old],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Renomme le code d'un jeu de contrôles + cascade sur `dim_control_set_item.set_code`.
fn rename_control_set(con: &Connection, old: &str, new: &str) -> Result<(), AppError> {
    if new.is_empty() {
        return Err(AppError::bad_request("nouveau code vide"));
    }
    if new == old {
        return Ok(());
    }
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_control_set WHERE code = ?",
            params![old],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::not_found(format!("jeu de contrôles introuvable : {old}")));
    }
    let taken: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_control_set WHERE code = ?",
            params![new],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if taken {
        return Err(AppError::conflict(format!("code déjà utilisé : {new}")));
    }
    // Cascade sur les items.
    con.execute(
        "UPDATE dim_control_set_item SET set_code = ? WHERE set_code = ?",
        params![new, old],
    )
    .map_err(db_err)?;
    // Renommer le jeu lui-même.
    con.execute(
        "UPDATE dim_control_set SET code = ? WHERE code = ?",
        params![new, old],
    )
    .map_err(db_err)?;
    Ok(())
}

/// POST /api/controls/{code}/rename — renomme le code d'un contrôle.
async fn rename_control_handler(
    Path(old): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameBody>,
) -> Result<Json<JsonValue>, AppError> {
    {
        let con = lock_con(&state)?;
        rename_control(&con, &old, &body.new)?;
        let _ = con.execute("CHECKPOINT", []);
    }
    Ok(Json(serde_json::json!({ "renamed": { "old": old, "new": body.new } })))
}

/// POST /api/control-sets/{code}/rename — renomme le code d'un jeu de contrôles.
async fn rename_control_set_handler(
    Path(old): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameBody>,
) -> Result<Json<JsonValue>, AppError> {
    {
        let con = lock_con(&state)?;
        rename_control_set(&con, &old, &body.new)?;
        let _ = con.execute("CHECKPOINT", []);
    }
    Ok(Json(serde_json::json!({ "renamed": { "old": old, "new": body.new } })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Router
// ─────────────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/controls", get(list_controls).post(create_control))
        .route(
            "/api/controls/{code}",
            get(get_control)
                .put(update_control)
                .delete(delete_control),
        )
        .route("/api/controls/{code}/run", post(run_single_handler))
        .route("/api/controls/{code}/rename", post(rename_control_handler))
        .route("/api/controls/operands", get(operands_catalog))
        .route(
            "/api/control-sets",
            get(list_control_sets).post(create_control_set),
        )
        .route(
            "/api/control-sets/{code}",
            get(get_control_set)
                .put(update_control_set)
                .delete(delete_control_set),
        )
        .route("/api/control-sets/{code}/run", post(run_set_handler))
        .route("/api/control-sets/{code}/rename", post(rename_control_set_handler))
        .route(
            "/api/control-sets/{code}/results",
            get(get_results_handler),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
//  Default pour désérialisation JSON vide
// ─────────────────────────────────────────────────────────────────────────────

impl Default for ControlDefinition {
    fn default() -> Self {
        Self {
            levels: Vec::new(),
            grain: Vec::new(),
            selection: Vec::new(),
            expression: None,
            assertions: Vec::new(),
            compare: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let con = Connection::open_in_memory().unwrap();
        crate::schema::create_schema(&con).unwrap();
        crate::seed::seed_all(&con).unwrap();
        con
    }

    #[allow(dead_code)]
    fn mk_control(con: &Connection, code: &str, def: &str) {
        con.execute(
            "INSERT INTO dim_control (code, libelle, definition) VALUES (?, ?, ?)",
            params![code, code, def],
        )
        .unwrap();
    }

    #[test]
    fn validate_ok() {
        let con = db();
        let def = ControlDefinition {
            levels: vec!["consolidated".to_string()],
            grain: vec!["entity".to_string()],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: None,
        };
        assert!(validate_definition(&con, &def).is_ok());
    }

    #[test]
    fn validate_empty_levels_rejected() {
        let con = db();
        let def = ControlDefinition {
            levels: vec![],
            grain: vec![],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: None,
        };
        assert!(validate_definition(&con, &def).is_err());
    }

    #[test]
    fn validate_empty_assertions_rejected() {
        let con = db();
        let def = ControlDefinition {
            levels: vec!["consolidated".to_string()],
            grain: vec![],
            selection: vec![],
            expression: None,
            assertions: vec![],
            compare: None,
        };
        assert!(validate_definition(&con, &def).is_err());
    }

    #[test]
    fn validate_bad_level_rejected() {
        let con = db();
        let def = ControlDefinition {
            levels: vec!["nimporte quoi".to_string()],
            grain: vec![],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: None,
        };
        assert!(validate_definition(&con, &def).is_err());
    }

    #[test]
    fn validate_raw_only_with_compare_rejected() {
        let con = db();
        let def = ControlDefinition {
            levels: vec!["raw".to_string()],
            grain: vec![],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: Some(Compare {
                metric: "variation_pct".to_string(),
                baseline_consolidation_id: None,
                warn: 10.0,
                error: 50.0,
            }),
        };
        assert!(validate_definition(&con, &def).is_err());
    }

    #[test]
    fn run_raw_on_seed() {
        let con = db();
        let def = ControlDefinition {
            levels: vec!["raw".to_string()],
            grain: vec!["entity".to_string()],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: None,
        };
        let params = RunParams {
            consolidation_id: None,
            phase: Some("REEL".to_string()),
            entry_period: Some("2024".to_string()),
        };
        let rows = run_control_at_level(&con, &def, "raw", &params).unwrap();
        // Le seed contient des entrées pour REEL/2024
        assert!(!rows.is_empty());
    }

    #[test]
    fn run_pipeline_on_seed() {
        let con = db();
        let params = crate::ConvertParams::load_params(&con, 1).unwrap();
        crate::run_pipeline(&con, &params).unwrap();
        let def = ControlDefinition {
            levels: vec!["consolidated".to_string()],
            grain: vec!["entity".to_string()],
            selection: vec![],
            expression: None,
            assertions: vec![Assertion::Nonzero],
            compare: None,
        };
        let run_params = RunParams {
            consolidation_id: Some(1),
            phase: None,
            entry_period: None,
        };
        let rows = run_control_at_level(&con, &def, "consolidated", &run_params).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn assertion_range_works() {
        let row = RawRow {
            grain: BTreeMap::new(),
            value: Some(500.0),
            baseline: None,
            variation: None,
            row_count: 1,
        };
        let assertions = vec![Assertion::Range {
            warn: 100.0,
            error: 1000.0,
        }];
        assert_eq!(evaluate_assertions(&assertions, &row), Status::Warn);

        let row2 = RawRow {
            grain: BTreeMap::new(),
            value: Some(1500.0),
            baseline: None,
            variation: None,
            row_count: 1,
        };
        assert_eq!(evaluate_assertions(&assertions, &row2), Status::Error);
    }

    #[test]
    fn assertion_nonzero_works() {
        let row = RawRow {
            grain: BTreeMap::new(),
            value: Some(0.0),
            baseline: None,
            variation: None,
            row_count: 1,
        };
        let assertions = vec![Assertion::Nonzero];
        assert_eq!(evaluate_assertions(&assertions, &row), Status::Error);

        let row2 = RawRow {
            grain: BTreeMap::new(),
            value: Some(100.0),
            baseline: None,
            variation: None,
            row_count: 1,
        };
        assert_eq!(evaluate_assertions(&assertions, &row2), Status::Pass);
    }

    #[test]
    fn assertion_existence_nodata() {
        let row = RawRow {
            grain: BTreeMap::new(),
            value: None,
            baseline: None,
            variation: None,
            row_count: 0,
        };
        let assertions = vec![Assertion::Existence];
        assert_eq!(evaluate_assertions(&assertions, &row), Status::NoData);
    }

    // ── Renommage (étape 12 B1) ────────────────────────────────────────

    #[test]
    fn rename_control_ok() {
        let con = db();
        mk_control(&con, "CTRL_A", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        rename_control(&con, "CTRL_A", "CTRL_B").unwrap();
        // Vérifier que le nouveau code existe.
        let code: String = con
            .query_row("SELECT code FROM dim_control WHERE code = 'CTRL_B'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(code, "CTRL_B");
        // Vérifier que l'ancien n'existe plus.
        let count: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_control WHERE code = 'CTRL_A'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn rename_control_cascade_sur_set_items() {
        let con = db();
        mk_control(&con, "CTRL_X", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        // Créer un jeu de contrôles qui référence CTRL_X.
        con.execute(
            "INSERT INTO dim_control_set (code, libelle) VALUES ('SET_1', 'Test set')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_control_set_item (set_code, control_code, ord) VALUES ('SET_1', 'CTRL_X', 1)",
            [],
        )
        .unwrap();
        // Renommer le contrôle.
        rename_control(&con, "CTRL_X", "CTRL_Y").unwrap();
        // Vérifier que le lien a été mis à jour.
        let cc: String = con
            .query_row(
                "SELECT control_code FROM dim_control_set_item WHERE set_code = 'SET_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cc, "CTRL_Y", "control_code doit être mis à jour dans les items");
    }

    #[test]
    fn rename_control_set_ok() {
        let con = db();
        con.execute(
            "INSERT INTO dim_control_set (code, libelle) VALUES ('OLD_SET', 'Old')",
            [],
        )
        .unwrap();
        rename_control_set(&con, "OLD_SET", "NEW_SET").unwrap();
        let code: String = con
            .query_row("SELECT code FROM dim_control_set WHERE code = 'NEW_SET'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(code, "NEW_SET");
    }

    #[test]
    fn rename_control_set_cascade_sur_items() {
        let con = db();
        mk_control(&con, "C1", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        con.execute(
            "INSERT INTO dim_control_set (code, libelle) VALUES ('S1', 'Set 1')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_control_set_item (set_code, control_code, ord) VALUES ('S1', 'C1', 1)",
            [],
        )
        .unwrap();
        rename_control_set(&con, "S1", "S2").unwrap();
        let sc: String = con
            .query_row(
                "SELECT set_code FROM dim_control_set_item WHERE control_code = 'C1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sc, "S2", "set_code doit être mis à jour dans les items");
    }

    #[test]
    fn rename_control_inconnu_erreur() {
        let con = db();
        let err = rename_control(&con, "INEXISTANT", "NEW").unwrap_err();
        assert!(err.1.contains("introuvable"));
    }

    #[test]
    fn rename_control_deja_utilise_erreur() {
        let con = db();
        mk_control(&con, "A", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        mk_control(&con, "B", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        let err = rename_control(&con, "A", "B").unwrap_err();
        assert!(err.1.contains("déjà utilisé"));
    }

    #[test]
    fn rename_control_noop_si_meme_code() {
        let con = db();
        mk_control(&con, "SAME", r#"{"levels":["consolidated"],"grain":[],"selection":[],"assertions":[{"type":"nonzero"}]}"#);
        // Ne doit pas échouer.
        rename_control(&con, "SAME", "SAME").unwrap();
    }
}
