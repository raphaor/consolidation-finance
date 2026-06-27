//! Migration des valeurs JSON de codes vers ids (étape 6 B1).
//!
//! Avant cette étape, les JSON stockés dans `dim_rule.definition`,
//! `dim_aggregate.definition` et `dim_indicator.expression` référençaient les
//! membres de dimensions par leur **code** (chaîne mutable). Après migration, ils
//! utilisent l'**id** technique immuable : le renommage d'un code n'a plus aucun
//! effet sur ces JSON.
//!
//! ## Ce qui est migré
//!
//! | Champ JSON                            | Type de valeur            |
//! |---------------------------------------|---------------------------|
//! | `scope[*].val`                        | code de colonne ri() sat_perimeter (ex. méthode) |
//! | `operations[*].selection[*].val`      | code de membre d'une dim fact_entry |
//! | `operations[*].destination.<dim>.value` (mode override) | idem |
//! | Idem pour `dim_aggregate.definition.selection[*].val`   | idem |
//! | `dim_indicator.expression` `[code]`   | code d'agrégat / indicateur |
//! | `app_config.pivot_currency`           | code de devise (cascade) |
//!
//! ## Ce qui n'est pas encore migré (hors scope)
//!
//! - `coefficient.type` (format `{"type": "code"}`) — priorité moindre.
//! - `via` (codes de caractéristiques, noms de tables `car_<code>`) — bloqué sur étape 5.
//! - `ref` (codes de références directes) — pas encore renommable.
//!
//! ## Idempotence
//!
//! Chaque valeur est vérifiée avant traduction : si elle est déjà un entier (id),
//! elle est ignorée. La migration peut être relancée sans risque.

use duckdb::{params, Connection};
use serde_json::Value as JsonValue;

use crate::{formula, references, resolve};

// ─────────────────────────────────────────────────────────────────────────────
//  Primitives de résolution
// ─────────────────────────────────────────────────────────────────────────────

/// Traduit une valeur JSON (code string ou tableau de codes) vers un id entier
/// pour une dimension de `fact_entry` qui a une master data.
/// Retourne la valeur inchangée si elle est déjà un entier, ou si la dimension
/// n'a pas de master data, ou si le code n'existe pas.
fn translate_dim_val(
    con: &Connection,
    dim: &str,
    val: &JsonValue,
) -> duckdb::Result<JsonValue> {
    if val.is_number() {
        return Ok(val.clone()); // déjà un id
    }
    let Some((table, _)) = references::dimension_master(dim) else {
        return Ok(val.clone()); // dim libre, pas de résolution
    };
    match val {
        JsonValue::String(code) => {
            if let Some(id) = resolve::resolve_id(con, table, code)? {
                Ok(JsonValue::Number(id.into()))
            } else {
                Ok(val.clone())
            }
        }
        JsonValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                out.push(translate_dim_val(con, dim, v)?);
            }
            Ok(JsonValue::Array(out))
        }
        _ => Ok(val.clone()),
    }
}

/// Traduit une valeur JSON pour une colonne ri() de `sat_perimeter` (ex. methode).
/// Retourne None si la colonne n'est pas ri() ou si la valeur est déjà un id.
fn translate_scope_col_val(
    con: &Connection,
    sat_col: &str,
    val: &JsonValue,
) -> duckdb::Result<Option<JsonValue>> {
    if val.is_number() {
        return Ok(None); // déjà un id
    }
    let target_table = references::all_references(con)
        .into_iter()
        .find(|r| {
            r.table == "sat_perimeter"
                && r.column == sat_col
                && r.target_display_column.is_some()
        })
        .map(|r| r.target_table);
    let Some(target) = target_table else {
        return Ok(None); // colonne non ri()
    };
    match val {
        JsonValue::String(code) => {
            if let Some(id) = resolve::resolve_id(con, &target, code)? {
                Ok(Some(JsonValue::Number(id.into())))
            } else {
                Ok(None)
            }
        }
        JsonValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            let mut changed = false;
            for v in arr {
                if let Some(translated) = translate_scope_col_val(con, sat_col, v)? {
                    out.push(translated);
                    changed = true;
                } else {
                    out.push(v.clone());
                }
            }
            if changed {
                Ok(Some(JsonValue::Array(out)))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Dénormalisation (ids → codes) pour l'exposition API
// ─────────────────────────────────────────────────────────────────────────────

/// Traduit un id entier → code string pour une dimension de `fact_entry`.
/// Retourne la valeur inchangée si elle n'est pas un entier, ou si la dim n'a
/// pas de master data. Miroir de [`translate_dim_val`] (code→id).
fn translate_id_to_code(
    con: &Connection,
    dim: &str,
    val: &JsonValue,
) -> duckdb::Result<JsonValue> {
    if !val.is_number() {
        return Ok(val.clone()); // déjà un code (string) ou null
    }
    let Some((table, _)) = references::dimension_master(dim) else {
        return Ok(val.clone()); // dim libre, pas de résolution
    };
    match val.as_i64() {
        Some(id) => {
            if let Some(code) = resolve::code_of(con, table, id)? {
                Ok(JsonValue::String(code))
            } else {
                Ok(val.clone())
            }
        }
        None => Ok(val.clone()),
    }
}

/// Traduit un id entier → code string pour une colonne ri() de `sat_perimeter`.
/// Gère les scalaires et les tableaux. Miroir de [`translate_scope_col_val`].
fn translate_scope_id_to_code(
    con: &Connection,
    sat_col: &str,
    val: &JsonValue,
) -> duckdb::Result<Option<JsonValue>> {
    let target_table = references::all_references(con)
        .into_iter()
        .find(|r| {
            r.table == "sat_perimeter"
                && r.column == sat_col
                && r.target_display_column.is_some()
        })
        .map(|r| r.target_table);
    let Some(target) = target_table else {
        return Ok(None); // colonne non ri()
    };
    match val {
        JsonValue::Number(_) => match val.as_i64() {
            Some(id) => {
                if let Some(code) = resolve::code_of(con, &target, id)? {
                    Ok(Some(JsonValue::String(code)))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        },
        JsonValue::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            let mut changed = false;
            for v in arr {
                if let Some(translated) = translate_scope_id_to_code(con, sat_col, v)? {
                    out.push(translated);
                    changed = true;
                } else {
                    out.push(v.clone());
                }
            }
            if changed {
                Ok(Some(JsonValue::Array(out)))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None), // déjà un code string ou null
    }
}

/// Dénormalise les conditions de sélection d'un JSON de règle/poste (ids → codes).
/// Miroir de [`normalize_selection_conds`].
fn denormalize_selection_conds(con: &Connection, sel: &mut JsonValue) -> duckdb::Result<bool> {
    let Some(arr) = sel.as_array_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    for cond in arr.iter_mut() {
        if cond.get("via").is_some()
            || cond.get("ref").is_some()
            || cond.get("attr").is_some()
        {
            continue; // traversées : hors scope
        }
        let dim = match cond.get("dim").and_then(|x| x.as_str()) {
            Some(d) => d.to_string(),
            None => continue,
        };
        let Some(val) = cond.get("val") else { continue };
        if !val.is_number() && !val.is_array() {
            continue;
        }
        // Traduire tableau ou scalaire.
        let new_val = if let Some(arr) = val.as_array() {
            let mut out = Vec::with_capacity(arr.len());
            let mut any = false;
            for v in arr {
                let t = translate_id_to_code(con, &dim, v)?;
                if t != *v { any = true; }
                out.push(t);
            }
            if any { JsonValue::Array(out) } else { val.clone() }
        } else {
            translate_id_to_code(con, &dim, val)?
        };
        if new_val != *val {
            cond["val"] = new_val;
            changed = true;
        }
    }
    Ok(changed)
}

/// Dénormalise les destinations mode=override d'une opération (ids → codes).
/// Miroir de [`normalize_destinations`].
fn denormalize_destinations(con: &Connection, dest: &mut JsonValue) -> duckdb::Result<bool> {
    let Some(obj) = dest.as_object_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    let dims: Vec<String> = obj.keys().cloned().collect();
    for dim in dims {
        if let Some(entry) = obj.get_mut(&dim) {
            let mode = entry.get("mode").and_then(|m| m.as_str()).map(str::to_string);
            if mode.as_deref() != Some("override") {
                continue;
            }
            let val = match entry.get("value") {
                Some(v) => v.clone(),
                None => continue,
            };
            if !val.is_number() {
                continue; // déjà un code ou null
            }
            let new_val = translate_id_to_code(con, &dim, &val)?;
            if new_val != val {
                entry["value"] = new_val;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Dénormalise un JSON de définition de règle (ids → codes) pour l'exposition
/// via l'API. Miroir de [`normalize_rule_definition`].
///
/// Appelé par `GET /api/rules/{code}` : le stockage utilise des ids immuables ;
/// le contrat API expose des codes (pour que l'éditeur de règles reste lisible).
pub fn denormalize_rule_definition(con: &Connection, json: &str) -> duckdb::Result<String> {
    let Ok(mut v) = serde_json::from_str::<JsonValue>(json) else {
        return Ok(json.to_string());
    };
    let mut changed = false;

    // 1. scope[*].val
    if let Some(scope) = v.get_mut("scope").and_then(|s| s.as_array_mut()) {
        for cond in scope.iter_mut() {
            let sat_col = match cond.get("dim").and_then(|x| x.as_str()) {
                Some(c) => c.to_string(),
                None => continue,
            };
            let Some(val) = cond.get("val") else { continue };
            if let Some(new_val) = translate_scope_id_to_code(con, &sat_col, val)? {
                cond["val"] = new_val;
                changed = true;
            }
        }
    }

    // 2. operations[*].selection[*].val + destination.<dim>.value
    if let Some(ops) = v.get_mut("operations").and_then(|o| o.as_array_mut()) {
        for op in ops.iter_mut() {
            if let Some(sel) = op.get_mut("selection") {
                changed |= denormalize_selection_conds(con, sel)?;
            }
            if let Some(dest) = op.get_mut("destination") {
                changed |= denormalize_destinations(con, dest)?;
            }
        }
    }

    if changed {
        Ok(v.to_string())
    } else {
        Ok(json.to_string())
    }
}

/// Dénormalise un JSON de définition de poste (ids → codes).
/// Miroir de [`normalize_aggregate_definition`].
pub fn denormalize_aggregate_definition(con: &Connection, json: &str) -> duckdb::Result<String> {
    let Ok(mut v) = serde_json::from_str::<JsonValue>(json) else {
        return Ok(json.to_string());
    };
    let mut changed = false;
    if let Some(sel) = v.get_mut("selection") {
        changed |= denormalize_selection_conds(con, sel)?;
    }
    if changed {
        Ok(v.to_string())
    } else {
        Ok(json.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Normalisation des JSON de règles
// ─────────────────────────────────────────────────────────────────────────────

/// Normalise les valeurs d'un tableau de conditions de sélection (règle ou poste) :
/// traduit `val` de code string vers id entier pour les dims avec master data.
/// Modifie le JSON en place ; retourne `true` si des changements ont été faits.
fn normalize_selection_conds(con: &Connection, sel: &mut JsonValue) -> duckdb::Result<bool> {
    let Some(arr) = sel.as_array_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    for cond in arr.iter_mut() {
        // Ignorer les traversées (via / ref / attr) : leurs valeurs ciblent
        // d'autres tables (car_*, master data cible) hors scope de cette migration.
        if cond.get("via").is_some()
            || cond.get("ref").is_some()
            || cond.get("attr").is_some()
        {
            continue;
        }
        let dim = match cond.get("dim").and_then(|x| x.as_str()) {
            Some(d) => d.to_string(),
            None => continue,
        };
        let Some(val) = cond.get("val") else { continue };
        if val.is_null() || val.is_boolean() {
            continue;
        }
        let new_val = translate_dim_val(con, &dim, val)?;
        if new_val != *val {
            cond["val"] = new_val;
            changed = true;
        }
    }
    Ok(changed)
}

/// Normalise les valeurs de destination mode=override d'une opération.
fn normalize_destinations(con: &Connection, dest: &mut JsonValue) -> duckdb::Result<bool> {
    let Some(obj) = dest.as_object_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    // Collecter les dims à modifier pour éviter les problèmes de borrow.
    let dims: Vec<String> = obj.keys().cloned().collect();
    for dim in dims {
        if let Some(entry) = obj.get_mut(&dim) {
            let mode = entry.get("mode").and_then(|m| m.as_str()).map(str::to_string);
            if mode.as_deref() != Some("override") {
                continue;
            }
            let val = match entry.get("value") {
                Some(v) => v.clone(),
                None => continue,
            };
            if val.is_number() {
                continue; // déjà un id
            }
            let new_val = translate_dim_val(con, &dim, &val)?;
            if new_val != val {
                entry["value"] = new_val;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// Normalise un JSON de définition de règle (scope + operations[*].selection + destination).
/// Retourne le JSON modifié (ou le JSON inchangé si rien n'a été traduit).
pub fn normalize_rule_definition(con: &Connection, json: &str) -> duckdb::Result<String> {
    let Ok(mut v) = serde_json::from_str::<JsonValue>(json) else {
        return Ok(json.to_string());
    };
    let mut changed = false;

    // 1. scope[*].val — colonnes ri() de sat_perimeter (ex. methode).
    if let Some(scope) = v.get_mut("scope").and_then(|s| s.as_array_mut()) {
        for cond in scope.iter_mut() {
            let sat_col = match cond.get("dim").and_then(|x| x.as_str()) {
                Some(c) => c.to_string(),
                None => continue,
            };
            let Some(val) = cond.get("val") else { continue };
            if let Some(new_val) = translate_scope_col_val(con, &sat_col, val)? {
                cond["val"] = new_val;
                changed = true;
            }
        }
    }

    // 2. operations[*].selection[*].val + destination.<dim>.value
    if let Some(ops) = v.get_mut("operations").and_then(|o| o.as_array_mut()) {
        for op in ops.iter_mut() {
            if let Some(sel) = op.get_mut("selection") {
                changed |= normalize_selection_conds(con, sel)?;
            }
            if let Some(dest) = op.get_mut("destination") {
                changed |= normalize_destinations(con, dest)?;
            }
        }
    }

    if changed {
        Ok(v.to_string())
    } else {
        Ok(json.to_string())
    }
}

/// Normalise un JSON de définition de poste (selection[*].val au niveau racine).
pub fn normalize_aggregate_definition(con: &Connection, json: &str) -> duckdb::Result<String> {
    let Ok(mut v) = serde_json::from_str::<JsonValue>(json) else {
        return Ok(json.to_string());
    };
    let mut changed = false;
    if let Some(sel) = v.get_mut("selection") {
        changed |= normalize_selection_conds(con, sel)?;
    }
    if changed {
        Ok(v.to_string())
    } else {
        Ok(json.to_string())
    }
}

/// Normalise une expression d'indicateur : remplace `[code]` par `[id]` pour
/// chaque opérande qui correspond à un agrégat ou un indicateur.
pub fn normalize_indicator_expression(con: &Connection, expr: &str) -> duckdb::Result<String> {
    let names = formula::operands(expr).unwrap_or_default();
    if names.is_empty() {
        return Ok(expr.to_string());
    }
    let mut result = expr.to_string();
    for name in &names {
        if name.parse::<i64>().is_ok() {
            continue; // déjà un id
        }
        // Chercher d'abord comme agrégat, puis comme indicateur.
        let id_opt: Option<i64> = {
            let agg_id: duckdb::Result<i64> = con.query_row(
                "SELECT id FROM dim_aggregate WHERE code = ?",
                params![name],
                |r| r.get(0),
            );
            match agg_id {
                Ok(id) => Some(id),
                Err(duckdb::Error::QueryReturnedNoRows) => {
                    con.query_row(
                        "SELECT id FROM dim_indicator WHERE code = ?",
                        params![name],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        duckdb::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })?
                }
                Err(other) => return Err(other),
            }
        };
        if let Some(id) = id_opt {
            result = result.replace(&format!("[{name}]"), &format!("[{id}]"));
        }
    }
    Ok(result)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Migration au démarrage
// ─────────────────────────────────────────────────────────────────────────────

/// Migre toutes les valeurs JSON de codes vers ids pour les règles, postes et
/// indicateurs existants. **Idempotente** : les valeurs déjà entières sont ignorées.
/// Appelée au démarrage du serveur, après `ensure_ids`.
pub fn migrate_json_to_ids(con: &Connection) -> duckdb::Result<()> {
    // dim_rule.definition
    let rules: Vec<(String, String)> = {
        let mut stmt = con.prepare(
            "SELECT code, definition FROM dim_rule WHERE definition IS NOT NULL",
        )?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .and_then(|rows| rows.collect::<duckdb::Result<Vec<_>>>())?
    };
    for (code, def) in &rules {
        let normalized = normalize_rule_definition(con, def)?;
        if normalized != *def {
            con.execute(
                "UPDATE dim_rule SET definition = ? WHERE code = ?",
                params![normalized, code],
            )?;
        }
    }

    // dim_aggregate.definition
    let aggs: Vec<(String, String)> = {
        let mut stmt = con.prepare(
            "SELECT code, definition FROM dim_aggregate WHERE definition IS NOT NULL",
        )?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .and_then(|rows| rows.collect::<duckdb::Result<Vec<_>>>())?
    };
    for (code, def) in &aggs {
        let normalized = normalize_aggregate_definition(con, def)?;
        if normalized != *def {
            con.execute(
                "UPDATE dim_aggregate SET definition = ? WHERE code = ?",
                params![normalized, code],
            )?;
        }
    }

    // dim_indicator.expression
    let inds: Vec<(String, String)> = {
        let mut stmt = con.prepare(
            "SELECT code, expression FROM dim_indicator WHERE expression IS NOT NULL",
        )?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .and_then(|rows| rows.collect::<duckdb::Result<Vec<_>>>())?
    };
    for (code, expr) in &inds {
        let normalized = normalize_indicator_expression(con, expr)?;
        if normalized != *expr {
            con.execute(
                "UPDATE dim_indicator SET expression = ? WHERE code = ?",
                params![normalized, code],
            )?;
        }
    }

    Ok(())
}
