//! Moteur de règles de consolidation (post-MVP).
//!
//! Une règle est un JSON stocké dans `dim_rule.definition` composé de :
//!
//! - **scope** : conditions sur le périmètre (`sat_perimeter`) qui filtrent
//!   les entités / partenaires éligibles. Chaque condition cible `"entity"` ou
//!   `"partner"` et porte sur une colonne de `sat_perimeter`
//!   (`methode`, `pct_interet`, `pct_integration`, `entree`, `sortie`).
//! - **operations** : liste ordonnée d'opérations. Chaque opération sélectionne
//!   des lignes à un niveau de `fact_entry`, leur applique un coefficient
//!   (`pct_integration`, `pct_interet`, `constant` ou `1.0` par défaut), un
//!   multiplicateur (typiquement `1` ou `-1`), puis écrit le résultat au même
//!   niveau avec une `destination` par dimension (héritée, surchargée ou nulle).
//!
//! Un *ruleset* (table `dim_ruleset` + items ordonnés dans `dim_ruleset_item`)
//! enchaîne plusieurs règles. [`run_ruleset`] exécute un ruleset contre
//! `fact_entry` et renvoie un [`RulesetReport`].
//!
//! # Sécurité SQL
//!
//! Les noms de colonnes (`selection.dim`, `scope.dim`, clés de `destination`,
//! `level`) sont validés contre des whitelists statiques : aucun identifiant
//! n'est interpolé depuis l'utilisateur. Les valeurs passent par des `?`
//! paramétrés.
//!
//! # Reconstruction des clôtures
//!
//! Après chaque règle, [`crate::pipeline::materialize_closures`] est appelée
//! pour chaque niveau touché : les F99 sont reconstruites depuis leurs
//! constituants (y compris les nouvelles écritures générées par la règle).

use crate::pipeline::materialize_closures::materialize_closures;
use duckdb::{params_from_iter, types::Value as DbValue, Connection};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;

// ─────────────────────────────────────────────────────────────────────────────
//  Whitelists — sécurité : aucun identifiant utilisateur n'est interpolé.
// ─────────────────────────────────────────────────────────────────────────────

/// Niveaux de stockage autorisés pour la sélection / l'écriture.
const ALLOWED_LEVELS: &[&str] = &["corporate", "reclassified", "converted", "consolidated"];

/// Colonnes de `fact_entry` autorisées dans `selection.dim`.
const ALLOWED_SELECTION_DIMS: &[&str] = &[
    "scenario",
    "entity",
    "entry_period",
    "period",
    "account",
    "flow",
    "currency",
    "nature",
    "partner",
    "share",
    "analysis",
    "analysis2",
    "level",
];

/// Colonnes de `sat_perimeter` autorisées dans `scope.dim`.
const ALLOWED_SCOPE_DIMS: &[&str] = &[
    "methode",
    "pct_interet",
    "pct_integration",
    "entree",
    "sortie",
];

/// Dimensions pilotables via `destination`. Les autres sont toujours héritées.
const PILOTABLE_DIMS: &[&str] = &["entity", "account", "flow", "nature", "partner", "share"];

/// Cibles autorisées pour `scope.target`.
const ALLOWED_TARGETS: &[&str] = &["entity", "partner"];

/// Opérateurs acceptés sur les conditions (scope et sélection).
const ALLOWED_OPS: &[&str] = &["=", "!=", ">", "<", ">=", "<=", "IN", "IS NULL", "IS NOT NULL"];

// ─────────────────────────────────────────────────────────────────────────────
//  Report
// ─────────────────────────────────────────────────────────────────────────────

/// Résultat d'exécution d'une règle individuelle (agrégat par niveau).
#[derive(Debug, Clone, Serialize)]
pub struct RuleResult {
    /// Code de la règle dans `dim_rule`.
    pub rule_code: String,
    /// Niveau de `fact_entry` touched (un RuleResult par niveau touché).
    pub level: String,
    /// Nombre de lignes générées par la règle à ce niveau.
    pub generated: usize,
}

/// Rapport d'exécution d'un ruleset.
#[derive(Debug, Clone, Serialize)]
pub struct RulesetReport {
    /// Code du ruleset exécuté.
    pub ruleset: String,
    /// Détail par règle et par niveau touché.
    pub rules: Vec<RuleResult>,
    /// Nombre total de lignes générées (somme de `rules[].generated`).
    pub total_generated: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Erreur interne (mapping vers duckdb::Error via string)
// ─────────────────────────────────────────────────────────────────────────────

/// Erreur produite par le parsing / la génération SQL.
///
/// On utilise un type String pour pouvoir propager via `duckdb::Error`
/// (cf. `SynthesisError` de duckdb-rs).
type RuleResult_<T> = Result<T, String>;

// ─────────────────────────────────────────────────────────────────────────────
//  Structures parsées depuis le JSON de définition
// ─────────────────────────────────────────────────────────────────────────────

/// Définition complète d'une règle (champ `dim_rule.definition`).
#[derive(Debug, Clone)]
struct Definition {
    scope: Vec<ScopeCond>,
    operations: Vec<Operation>,
}

/// Une condition de périmètre (`scope[]`).
#[derive(Debug, Clone)]
struct ScopeCond {
    target: String, // "entity" ou "partner"
    dim: String,
    op: String,
    val: JsonValue,
}

/// Une opération (`operations[]`).
#[derive(Debug, Clone)]
struct Operation {
    seq: i64,
    level: String,
    selection: Vec<SelectionCond>,
    coefficient: Coefficient,
    multiplicateur: f64,
    destination: Vec<(String, Destination)>,
}

/// Une condition de sélection (`operations[].selection[]`).
#[derive(Debug, Clone)]
struct SelectionCond {
    dim: String,
    op: String,
    val: Option<JsonValue>,
}

/// Coefficient appliqué au montant source.
#[derive(Debug, Clone)]
enum Coefficient {
    PctIntegration,
    PctInteret,
    Constant(f64),
}

/// Destination d'une dimension pilotable.
#[derive(Debug, Clone)]
struct Destination {
    mode: String,        // "inherit" | "override" | "null"
    value: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Parsing JSON → structures fortement typées
// ─────────────────────────────────────────────────────────────────────────────

fn parse_definition(json: &str) -> RuleResult_<Definition> {
    let v: JsonValue = serde_json::from_str(json)
        .map_err(|e| format!("définition JSON invalide : {e}"))?;
    let obj = v
        .as_object()
        .ok_or("la définition doit être un objet JSON")?;

    // scope (optionnel, défaut [])
    let scope = match obj.get("scope") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(parse_scope_cond)
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("scope doit être un tableau".into()),
    };

    // operations (obligatoire, non vide idéalement — on tolère vide)
    let operations = match obj.get("operations") {
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(parse_operation)
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("operations doit être un tableau".into()),
        None => return Err("operations manquant".into()),
    };

    Ok(Definition { scope, operations })
}

fn parse_scope_cond(v: &JsonValue) -> RuleResult_<ScopeCond> {
    let obj = v
        .as_object()
        .ok_or("each scope item doit être un objet")?;
    let target = expect_str(obj, "target")?;
    if !ALLOWED_TARGETS.contains(&target.as_str()) {
        return Err(format!(
            "scope.target invalide : {target} (attendu parmi {ALLOWED_TARGETS:?})"
        ));
    }
    let dim = expect_str(obj, "dim")?;
    if !ALLOWED_SCOPE_DIMS.contains(&dim.as_str()) {
        return Err(format!(
            "scope.dim invalide : {dim} (attendu parmi {ALLOWED_SCOPE_DIMS:?})"
        ));
    }
    let op = expect_str(obj, "op")?;
    if !ALLOWED_OPS.contains(&op.as_str()) {
        return Err(format!("scope.op invalide : {op}"));
    }
    let val = obj
        .get("val")
        .cloned()
        .ok_or("scope.val manquant")?;
    // `val` est requis sauf pour IS NULL / IS NOT NULL (présence tolérée).
    if matches!(val, JsonValue::Null)
        && op != "IS NULL"
        && op != "IS NOT NULL"
    {
        return Err(format!("scope.val null pour op='{op}'"));
    }
    Ok(ScopeCond { target, dim, op, val })
}

fn parse_operation(v: &JsonValue) -> RuleResult_<Operation> {
    let obj = v
        .as_object()
        .ok_or("each operation doit être un objet")?;
    let seq = obj
        .get("seq")
        .and_then(|x| x.as_i64())
        .ok_or("operation.seq doit être un entier")?;
    let level = expect_str(obj, "level")?;
    if !ALLOWED_LEVELS.contains(&level.as_str()) {
        return Err(format!("operation.level invalide : {level}"));
    }
    let selection = match obj.get("selection") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(parse_selection_cond)
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("selection doit être un tableau".into()),
    };
    let coefficient = match obj.get("coefficient") {
        None | Some(JsonValue::Null) => Coefficient::Constant(1.0),
        Some(c) => parse_coefficient(c)?,
    };
    let multiplicateur = match obj.get("multiplicateur") {
        None | Some(JsonValue::Null) => 1.0,
        Some(JsonValue::Number(n)) => n
            .as_f64()
            .ok_or("multiplicateur doit être un nombre")?,
        Some(_) => return Err("multiplicateur doit être un nombre".into()),
    };
    let destination = match obj.get("destination") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                if !PILOTABLE_DIMS.contains(&k.as_str()) {
                    return Err(format!(
                        "destination.{k} n'est pas pilotable (dimension inconnue ou héritée par construction)"
                    ));
                }
                let d = parse_destination(v)?;
                out.push((k.clone(), d));
            }
            out
        }
        Some(_) => return Err("destination doit être un objet".into()),
    };
    Ok(Operation {
        seq,
        level,
        selection,
        coefficient,
        multiplicateur,
        destination,
    })
}

fn parse_selection_cond(v: &JsonValue) -> RuleResult_<SelectionCond> {
    let obj = v
        .as_object()
        .ok_or("each selection item doit être un objet")?;
    let dim = expect_str(obj, "dim")?;
    if !ALLOWED_SELECTION_DIMS.contains(&dim.as_str()) {
        return Err(format!(
            "selection.dim invalide : {dim} (attendu parmi {ALLOWED_SELECTION_DIMS:?})"
        ));
    }
    let op = expect_str(obj, "op")?;
    if !ALLOWED_OPS.contains(&op.as_str()) {
        return Err(format!("selection.op invalide : {op}"));
    }
    let val = obj.get("val").cloned();
    if val.is_none() && op != "IS NULL" && op != "IS NOT NULL" {
        return Err(format!("selection.val manquant pour op='{op}'"));
    }
    Ok(SelectionCond { dim, op, val })
}

fn parse_coefficient(v: &JsonValue) -> RuleResult_<Coefficient> {
    let obj = v
        .as_object()
        .ok_or("coefficient doit être un objet")?;
    let t = expect_str(obj, "type")?;
    match t.as_str() {
        "pct_integration" => Ok(Coefficient::PctIntegration),
        "pct_interet" => Ok(Coefficient::PctInteret),
        "constant" => {
            let value = obj
                .get("value")
                .and_then(|x| x.as_f64())
                .ok_or("coefficient.value doit être un nombre")?;
            Ok(Coefficient::Constant(value))
        }
        other => Err(format!("coefficient.type inconnu : {other}")),
    }
}

fn parse_destination(v: &JsonValue) -> RuleResult_<Destination> {
    let obj = v
        .as_object()
        .ok_or("destination.<dim> doit être un objet")?;
    let mode = expect_str(obj, "mode")?;
    let value = match mode.as_str() {
        "inherit" | "null" => None,
        "override" => Some(expect_str(obj, "value")?),
        other => return Err(format!("destination.mode inconnu : {other}")),
    };
    Ok(Destination { mode, value })
}

fn expect_str(obj: &serde_json::Map<String, JsonValue>, key: &str) -> RuleResult_<String> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("champ '{key}' manquant ou non-chaîne"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers de génération SQL
// ─────────────────────────────────────────────────────────────────────────────

/// Construit un fragment SQL `opérande op valeur` et pousse les paramètres
/// associés dans `params`. Gère `=`, `!=`, `>`, `<`, `>=`, `<=`, `IN`,
/// `IS NULL`, `IS NOT NULL`.
fn push_condition(
    operand: &str,
    op: &str,
    val: &Option<JsonValue>,
    params: &mut Vec<DbValue>,
) -> RuleResult_<String> {
    match op {
        "IS NULL" => Ok(format!("{operand} IS NULL")),
        "IS NOT NULL" => Ok(format!("{operand} IS NOT NULL")),
        "IN" => {
            let arr = val
                .as_ref()
                .and_then(|v| v.as_array())
                .ok_or("op=IN requiert val=liste")?;
            if arr.is_empty() {
                // Liste vide → faux (1=0) — évite `IN ()` invalide.
                return Ok("1=0".to_string());
            }
            let placeholders: Vec<&str> = arr.iter().map(|_| "?").collect();
            for x in arr {
                params.push(json_to_dbvalue(x));
            }
            Ok(format!("{operand} IN ({})", placeholders.join(", ")))
        }
        _ => {
            let v = val
                .as_ref()
                .ok_or_else(|| format!("op={op} requiert val"))?;
            params.push(json_to_dbvalue(v));
            Ok(format!("{operand} {op} ?"))
        }
    }
}

/// Conversion d'une valeur JSON générique vers `DbValue` pour binding.
///
/// Les booléens sont acceptés (utiles pour `entree`/`sortie`), les chaînes
/// deviennent du `TEXT`, les nombres des `Double` (les colonnes `pct_*` sont
/// `DECIMAL` mais DuckDB convertit depuis un Double bindé).
fn json_to_dbvalue(v: &JsonValue) -> DbValue {
    match v {
        JsonValue::Null => DbValue::Null,
        JsonValue::Bool(b) => DbValue::Boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                DbValue::BigInt(i)
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

/// Construit l'expression SQL de la destination pour une dimension pilotable.
///
/// - `"inherit"` → `e.<dim>`
/// - `"override", value` → pousse `value` dans `params`, renvoie `?`
/// - `"null"` → `NULL`
fn dest_expr(
    dim: &str,
    destinations: &[(String, Destination)],
    params: &mut Vec<DbValue>,
) -> String {
    let found = destinations.iter().find(|(k, _)| k == dim);
    match found {
        None => format!("e.{dim}"), // défaut = hérité
        Some((_, d)) => match d.mode.as_str() {
            "inherit" => format!("e.{dim}"),
            "null" => "NULL".to_string(),
            "override" => {
                params.push(DbValue::Text(d.value.clone().unwrap_or_default()));
                "?".to_string()
            }
            _ => format!("e.{dim}"),
        },
    }
}

/// Construit l'expression SQL du coefficient.
///
/// Renvoie `(expr_sql, needs_p_ent_join)` : si le coefficient lit `p_ent`,
/// il faut s'assurer que le JOIN `p_ent` est présent.
fn coefficient_expr(c: &Coefficient) -> (String, bool) {
    match c {
        Coefficient::PctIntegration => (
            "COALESCE(p_ent.pct_integration, 1.0)".to_string(),
            true,
        ),
        Coefficient::PctInteret => (
            "COALESCE(p_ent.pct_interet, 1.0)".to_string(),
            true,
        ),
        Coefficient::Constant(v) => {
            // Littéral inline : un f64 sert de coefficient multiplicatif.
            // On formate sans locale pour garantir un point décimal.
            let s = format_float(*v);
            (s, false)
        }
    }
}

/// Formatage d'un f64 en littéral SQL (point décimal, pas de séparateur de milliers).
fn format_float(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// Construit et exécute l'INSERT d'une opération, renvoie le nombre de lignes
/// générées.
fn exec_operation(
    con: &Connection,
    rule_code: &str,
    op: &Operation,
    scope: &[ScopeCond],
) -> Result<usize, duckdb::Error> {
    let snap = format!("_rule_snap_{}", op.level);

    // Pour savoir quels JOINs ajouter :
    //  - p_ent : si scope sur entity, ou coefficient pct_integration/pct_interet
    //  - p_part : si scope sur partner
    let (coeff_expr, coeff_needs_p_ent) = coefficient_expr(&op.coefficient);
    let scope_has_entity = scope.iter().any(|c| c.target == "entity");
    let scope_has_partner = scope.iter().any(|c| c.target == "partner");
    let need_p_ent = scope_has_entity || coeff_needs_p_ent;
    let need_p_part = scope_has_partner;

    // Analysis2 calculé (tag automatique RULE:{rule_code}:{seq}).
    let analysis2 = format!("RULE:{rule_code}:{}", op.seq);

    // Construction du SELECT (14 colonnes dans l'ordre fact_entry).
    let mut params: Vec<DbValue> = Vec::new();
    let entity_dest = dest_expr("entity", &op.destination, &mut params);
    let account_dest = dest_expr("account", &op.destination, &mut params);
    let flow_dest = dest_expr("flow", &op.destination, &mut params);
    let nature_dest = dest_expr("nature", &op.destination, &mut params);
    let partner_dest = dest_expr("partner", &op.destination, &mut params);
    let share_dest = dest_expr("share", &op.destination, &mut params);

    let select_clause = format!(
        "SELECT\n    \
            e.scenario,\n    \
            {entity_dest} AS entity,\n    \
            e.entry_period,\n    \
            e.period,\n    \
            {account_dest} AS account,\n    \
            {flow_dest} AS flow,\n    \
            e.currency,\n    \
            {nature_dest} AS nature,\n    \
            {partner_dest} AS partner,\n    \
            {share_dest} AS share,\n    \
            e.analysis,\n    \
            ? AS analysis2,\n    \
            ? AS level,\n    \
            e.amount * {coeff_expr} * {mult} AS amount\n\
         FROM {snap} e",
        mult = format_float(op.multiplicateur),
    );
    // Analysis2 + level (bindés après les dest override, avant les conditions).
    params.push(DbValue::Text(analysis2));
    params.push(DbValue::Text(op.level.clone()));

    // JOINs sur sat_perimeter.
    let mut joins = String::new();
    if need_p_ent {
        joins.push_str(
            "\nJOIN sat_perimeter p_ent\n  \
                ON p_ent.entity = e.entity\n \
                AND p_ent.scenario = e.scenario\n \
                AND p_ent.period = e.entry_period",
        );
        for c in scope.iter().filter(|c| c.target == "entity") {
            let operand = format!("p_ent.{}", c.dim);
            let cond = push_condition(&operand, &c.op, &Some(c.val.clone()), &mut params)
                .map_err(duckdb_synthesis_error)?;
            joins.push_str(&format!("\n AND {cond}"));
        }
    }
    if need_p_part {
        joins.push_str(
            "\nJOIN sat_perimeter p_part\n  \
                ON p_part.entity = e.partner\n \
                AND p_part.scenario = e.scenario\n \
                AND p_part.period = e.entry_period",
        );
        for c in scope.iter().filter(|c| c.target == "partner") {
            let operand = format!("p_part.{}", c.dim);
            let cond = push_condition(&operand, &c.op, &Some(c.val.clone()), &mut params)
                .map_err(duckdb_synthesis_error)?;
            joins.push_str(&format!("\n AND {cond}"));
        }
    }

    // WHERE (sélection).
    let mut where_clauses: Vec<String> = Vec::new();
    for sel in &op.selection {
        let operand = format!("e.{}", sel.dim);
        let cond = push_condition(&operand, &sel.op, &sel.val, &mut params)
            .map_err(duckdb_synthesis_error)?;
        where_clauses.push(cond);
    }
    let where_clause = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("\nWHERE {}", where_clauses.join("\n  AND "))
    };

    // Assemblage final.
    let sql = format!(
        "INSERT INTO fact_entry\n    \
            (scenario, entity, entry_period, period, account, flow,\n     \
             currency, nature, partner, share, analysis, analysis2, level, amount)\n\
         {select_clause}{joins}{where_clause}"
    );

    let n = con.execute(&sql, params_from_iter(params.into_iter()))?;
    Ok(n)
}

/// Convertit une erreur `String` (parsing/génération) en `duckdb::Error`.
///
/// duckdb-rs ne propose pas de variant "message générique" ; on utilise
/// `InvalidParameterName` qui accepte une `String`. Le message reste lisible
/// côté API (cf. `state::db_err` qui l'affiche tel quel).
fn duckdb_synthesis_error(msg: String) -> duckdb::Error {
    duckdb::Error::InvalidParameterName(msg)
}

// ─────────────────────────────────────────────────────────────────────────────
//  API publique
// ─────────────────────────────────────────────────────────────────────────────

/// Exécute un ruleset contre `fact_entry` et renvoie un rapport.
///
/// Algorithme :
/// 1. Lit les règles du ruleset (ordonnées par `dim_ruleset_item.ordre`).
/// 2. Pour chaque règle :
///    a. Parse la définition JSON.
///    b. Pour chaque niveau `L` distinct parmi les opérations :
///       `CREATE TEMP TABLE _rule_snap_L AS SELECT * FROM fact_entry WHERE level='L'`.
///    c. Pour chaque opération (toutes lisent le snapshot de leur niveau) :
///       construit et exécute un `INSERT ... SELECT` paramétré.
///    d. Pour chaque niveau `L` : `DROP TABLE _rule_snap_L` puis
///       `materialize_closures(L)` pour reconstruire les clôtures F99.
///
/// Les snapshots empêchent qu'une opération lise les lignes générées par une
/// opération précédente (idempotence de la règle).
pub fn run_ruleset(con: &Connection, ruleset_code: &str) -> Result<RulesetReport, duckdb::Error> {
    // 1. Charger les règles du ruleset dans l'ordre.
    let rules: Vec<(String, String)> = {
        let mut stmt = con.prepare(
            "SELECT i.rule_code, r.definition \
             FROM dim_ruleset_item i \
             JOIN dim_rule r ON r.code = i.rule_code \
             WHERE i.ruleset_code = ? \
             ORDER BY i.ordre",
        )?;
        let rows = stmt.query_map([ruleset_code], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut v = Vec::new();
        for r in rows {
            let (code, def) = r?;
            v.push((code, def));
        }
        v
    };

    let mut results: Vec<RuleResult> = Vec::new();
    let mut total_generated = 0usize;

    for (rule_code, definition_json) in &rules {
        let definition = parse_definition(definition_json).map_err(duckdb_synthesis_error)?;

        // Niveaux distincts touchés par les opérations de la règle.
        let levels: BTreeSet<&str> = definition
            .operations
            .iter()
            .map(|op| op.level.as_str())
            .collect();

        // Snapshots (un par niveau).
        for lvl in &levels {
            let sql = format!(
                "CREATE TEMP TABLE _rule_snap_{lvl} AS \
                 SELECT * FROM fact_entry WHERE level = ?"
            );
            con.execute(&sql, [lvl.to_string()])?;
        }

        // Exécution des opérations. On agrège le nombre de lignes générées
        // par niveau pour produire un RuleResult par (règle, niveau).
        let mut generated_per_level: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for op in &definition.operations {
            let n = exec_operation(con, rule_code, op, &definition.scope)?;
            *generated_per_level.entry(op.level.clone()).or_default() += n;
        }

        // Cleanup + reconstruction des clôtures.
        for lvl in &levels {
            let drop_sql = format!("DROP TABLE _rule_snap_{lvl}");
            // On ignore l'erreur du DROP (la table peut déjà être absente).
            let _ = con.execute(&drop_sql, []);
            materialize_closures(con, lvl)?;
        }

        for (lvl, n) in generated_per_level {
            results.push(RuleResult {
                rule_code: rule_code.clone(),
                level: lvl,
                generated: n,
            });
            total_generated += n;
        }
    }

    Ok(RulesetReport {
        ruleset: ruleset_code.to_string(),
        rules: results,
        total_generated,
    })
}
