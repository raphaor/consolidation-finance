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
//! `level`) sont validés contre des whitelists : aucun identifiant
//! n'est interpolé depuis l'utilisateur. Les valeurs passent par des `?`
//! paramétrés.
//!
//! Les whitelists `selection.dim`, `destination.<dim>` et `scope.dim` sont
//! **dynamiques** : elles sont calculées depuis le registre central des
//! dimensions ([`crate::dimensions`]) pour les deux premières, et depuis
//! `information_schema` (colonnes de `sat_perimeter`) pour la troisième, au
//! début de chaque exécution d'un ruleset. Les dimensions built-in (12) et les
//! dimensions custom (ajoutées par l'utilisateur via l'API) y figurent toutes.
//! Les noms de colonnes custom proviennent du registre (créés via l'API et
//! validés à la création), jamais du JSON de la règle → pas de risque
//! d'injection SQL via ces noms.
//!
//! # Reconstruction des clôtures (débranchée — voir [`RECONSTRUCT_CLOSURES_AFTER_RULE`])
//!
//! Historiquement, après chaque règle, [`crate::pipeline::materialize_closures`]
//! était appelée pour chaque niveau touché afin de reconstruire les F99.
//!
//! **Décision de modélisation (2026-06-21)** : ce comportement est désormais
//! **débranché**. Le F99 est calculé par la mécanique de transition de niveau
//! (notamment la conversion) ; il n'est pas reconstruit après les règles. Une
//! règle d'élimination sélectionne et génère donc **elle-même chaque flux**, F99
//! compris (flux à flux). Le mécanisme reste **conservé derrière un flag** car il
//! pourra resservir pour d'autres usages.

use crate::characteristics;
use crate::dimensions;
use crate::formula::CoeffJoins;
use crate::pipeline::materialize_closures::materialize_closures;
use crate::references;
use duckdb::{params, params_from_iter, types::Value as DbValue, Connection};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;

/// Reconstruire les clôtures (F99) **après chaque règle** ?
///
/// Débranché (`false`, 2026-06-21) : le F99 relève de la transition de niveau
/// (conversion), pas d'une reconstruction post-règle ; les règles gèrent F99
/// flux à flux (cf. doc du module et discussion #13). Conservé — repasser à
/// `true` pour réactiver le comportement historique.
const RECONSTRUCT_CLOSURES_AFTER_RULE: bool = false;

// ─────────────────────────────────────────────────────────────────────────────
//  Whitelists — sécurité : aucun identifiant utilisateur n'est interpolé.
// ─────────────────────────────────────────────────────────────────────────────

/// Niveaux de stockage autorisés pour la sélection / l'écriture.
const ALLOWED_LEVELS: &[&str] = &["corporate", "converted", "consolidated"];

/// Colonnes de `sat_perimeter` autorisées dans `scope.dim`.
///
/// Calculées dynamiquement depuis `information_schema` (voir
/// [`RuleContext::from_registry`]). Fallback codé dur ci-dessous si la table
/// n'existe pas encore ou est vide.
fn allowed_scope_dims(con: &Connection) -> Vec<String> {
    con.prepare(
        "SELECT column_name \
         FROM information_schema.columns \
         WHERE table_name = 'sat_perimeter' \
         ORDER BY ordinal_position",
    )
    .and_then(|mut stmt| {
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<duckdb::Result<Vec<_>>>()
    })
    .unwrap_or_else(|_| {
        vec![
            "methode".into(),
            "pct_interet".into(),
            "pct_integration".into(),
            "entree".into(),
            "sortie".into(),
        ]
    })
}

/// Cibles autorisées pour `scope.target` : l'entité, son partenaire (`partner`)
/// ou sa quote-part (`share`) — ces trois dimensions portent un code d'entité,
/// joint à `sat_perimeter` pour filtrer sur les attributs de périmètre.
const ALLOWED_TARGETS: &[&str] = &["entity", "partner", "share"];

/// Opérateurs acceptés sur les conditions (scope et sélection).
const ALLOWED_OPS: &[&str] = &[
    "=",
    "!=",
    ">",
    "<",
    ">=",
    "<=",
    "IN",
    "IS NULL",
    "IS NOT NULL",
];

// ─────────────────────────────────────────────────────────────────────────────
//  Contexte de parsing dynamique (registre des dimensions)
// ─────────────────────────────────────────────────────────────────────────────

/// Contexte de validation construit depuis le registre des dimensions et
/// passé aux fonctions de parsing.
///
/// - `selection_dims` : toutes les dimensions propagées + `level` (qui est une
///   colonne de `fact_entry` sans être une dimension pilotable). Construit
///   dynamiquement depuis [`dimensions::load_all`].
/// - `pilotable_dims` : dimensions pilotables (Active + Analytical), cibles
///   autorisées pour `destination.<dim>`.
/// - `scope_dims` : colonnes de `sat_perimeter` autorisées dans `scope.dim`.
///   Construit dynamiquement depuis `information_schema` (cf.
///   [`allowed_scope_dims`]).
#[derive(Debug, Clone)]
pub struct RuleContext {
    pub selection_dims: Vec<String>,
    pub pilotable_dims: Vec<String>,
    pub scope_dims: Vec<String>,
}

impl RuleContext {
    /// Construit le contexte depuis le registre des dimensions (built-in +
    /// custom). `level` est ajouté manuellement à `selection_dims` (c'est une
    /// colonne de `fact_entry`, pas une dimension au sens du registre).
    pub fn from_registry(con: &Connection) -> Result<Self, duckdb::Error> {
        let dims = dimensions::load_all(con)?;
        let mut selection_dims: Vec<String> = dims.iter().map(|d| d.name.clone()).collect();
        selection_dims.push("level".to_string());
        let pilotable_dims: Vec<String> = dimensions::pilotable_cols(&dims)
            .into_iter()
            .map(String::from)
            .collect();
        let scope_dims = allowed_scope_dims(con);
        Ok(Self {
            selection_dims,
            pilotable_dims,
            scope_dims,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Validation référentielle d'une définition (à l'enregistrement)
// ─────────────────────────────────────────────────────────────────────────────

/// Valide les **références** d'une définition de règle, au-delà des noms de
/// dimensions déjà contrôlés au parsing : chaque valeur de `selection`, de
/// `destination` (mode `override`) et de `scope` doit exister dans sa table
/// cible (cf. [`crate::references`]). Appelée à l'enregistrement d'une règle
/// (`POST`/`PUT /api/rules`) pour refuser une règle qui pointe vers un compte,
/// une nature, une méthode… inexistants.
///
/// Les dimensions sans cible référentielle (`analysis`, customs) et les
/// opérateurs `IS NULL` / `IS NOT NULL` sont ignorés.
pub fn validate_definition(con: &Connection, definition_json: &str) -> Result<(), String> {
    let ctx = RuleContext::from_registry(con).map_err(|e| e.to_string())?;
    let def = parse_definition(definition_json, &ctx)?;

    for op in &def.operations {
        // Coefficient nommé : doit exister dans la bibliothèque et compiler.
        if let Coefficient::Named(code) = &op.coefficient {
            crate::coefficients::resolve_expr(con, code)?;
        }
        for s in &op.selection {
            // Cible référentielle pour valider `val` : par défaut c'est la master
            // data de `dim` ; en cas de traversée (`via` ou `ref`), la valeur
            // comparée vit dans une autre table.
            let target: Option<(String, String)> = if let Some(via) = &s.via {
                // Caractéristique N1 : on filtre sur car_<via>.code (valeur N1).
                // La caractéristique doit exister avec base_dimension = dim.
                match characteristics::base_dimension_of(con, via).map_err(|e| e.to_string())? {
                    Some(base) if base == s.dim => {
                        Some((format!("car_{via}"), "code".to_string()))
                    }
                    Some(other) => {
                        return Err(format!(
                            "selection.{dim} via : la caractéristique '{via}' a pour base \
                             '{other}', pas '{dim}'",
                            dim = s.dim
                        ))
                    }
                    None => {
                        return Err(format!(
                            "selection.{} via : caractéristique inconnue : {via}",
                            s.dim
                        ))
                    }
                }
            } else if let Some(rf) = &s.ref_field {
                // Référence directe (patron B) : on filtre sur la master data de
                // la dimension cible de la référence (souvent = dim en cas
                // d'auto-référence hiérarchique type compte_parent).
                match crate::custom_references::target_of(con, &s.dim, rf)
                    .map_err(|e| e.to_string())?
                {
                    Some(target_dim) => references::target_master(con, &target_dim),
                    None => {
                        return Err(format!(
                            "selection.{} ref : référence inconnue : {}.{}",
                            s.dim, s.dim, rf
                        ))
                    }
                }
            } else if let Some(attr) = &s.attr {
                // Enum natif (CHECK du DDL) : pas de table cible, pas de check
                // référentiel. On valide simplement que la valeur (ou chaque
                // valeur d'un IN) fait partie de la liste autorisée.
                let values = references::native_enum_lookup(&s.dim, attr).ok_or_else(|| {
                    format!(
                        "selection.{} attr : enum natif inconnu : {}.{}",
                        s.dim, s.dim, attr
                    )
                })?;
                if s.op != "IS NULL" && s.op != "IS NOT NULL" {
                    let vals: Vec<String> = match &s.val {
                        Some(JsonValue::Array(a)) => a
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect(),
                        Some(JsonValue::String(x)) => vec![x.clone()],
                        _ => vec![],
                    };
                    for v in &vals {
                        if !values.contains(&v.as_str()) {
                            return Err(format!(
                                "selection.{}.{} : valeur '{}' invalide (autorisées : {:?})",
                                s.dim, attr, v, values
                            ));
                        }
                    }
                }
                None
            } else {
                references::entry_dimension_target(&s.dim)
                    .map(|r| (r.target_table.to_string(), r.target_column.to_string()))
            };
            // Conversion vers Option<(&str, &str)> pour check_ref_value.
            let target_ref: Option<(&str, &str)> =
                target.as_ref().map(|(t, c)| (t.as_str(), c.as_str()));
            check_ref_value(
                con,
                target_ref,
                &s.op,
                &s.val,
                &format!("selection.{}", s.dim),
            )?;
        }
        for (dim, dest) in &op.destination {
            if dest.mode == "override" {
                if let (Some(v), Some(r)) = (&dest.value, references::entry_dimension_target(dim)) {
                    if !v.is_empty()
                        && !references::value_exists(con, r.target_table, r.target_column, v)
                            .map_err(|e| e.to_string())?
                    {
                        return Err(format!(
                            "destination.{dim} : '{v}' inexistant dans {}.{}",
                            r.target_table, r.target_column
                        ));
                    }
                }
            } else if dest.mode == "map" {
                // La caractéristique N1 (`via`) et l'attribut N2 (`attr`) doivent
                // exister, et l'attribut doit pointer vers la dimension écrite.
                let via = dest.via.as_deref().unwrap_or("");
                let attr = dest.attr.as_deref().unwrap_or("");
                if characteristics::base_dimension_of(con, via)
                    .map_err(|e| e.to_string())?
                    .is_none()
                {
                    return Err(format!(
                        "destination.{dim} map : caractéristique inconnue : {via}"
                    ));
                }
                match characteristics::attribute_target(con, via, attr)
                    .map_err(|e| e.to_string())?
                {
                    None => {
                        return Err(format!(
                            "destination.{dim} map : attribut inconnu : {via}.{attr}"
                        ))
                    }
                    Some(t) if t != *dim => {
                        return Err(format!(
                            "destination.{dim} map : l'attribut {via}.{attr} pointe vers '{t}', \
                             incompatible avec la dimension '{dim}'"
                        ))
                    }
                    _ => {}
                }
            } else if dest.mode == "map_ref" {
                // Référence directe (patron B) portée par la dimension écrite.
                // `ref` doit exister, avoir `host_dimension = dim` et
                // `target_dimension = dim` (la valeur écrite doit être un code
                // valide pour `dim`).
                let r = dest.ref_field.as_deref().unwrap_or("");
                match crate::custom_references::target_of(con, dim, r)
                    .map_err(|e| e.to_string())?
                {
                    None => {
                        return Err(format!(
                            "destination.{dim} map_ref : référence inconnue : {dim}.{r}"
                        ))
                    }
                    Some(t) if t != *dim => {
                        return Err(format!(
                            "destination.{dim} map_ref : la référence {dim}.{r} pointe vers \
                             '{t}', incompatible avec la dimension écrite '{dim}'"
                        ))
                    }
                    _ => {}
                }
            }
        }
    }
    for c in &def.scope {
        let target_ref: Option<(&str, &str)> =
            references::perimeter_target(&c.dim).map(|r| (r.target_table, r.target_column));
        check_ref_value(
            con,
            target_ref,
            &c.op,
            &c.val,
            &format!("scope.{} ({})", c.dim, c.target),
        )?;
    }
    Ok(())
}

/// Vérifie qu'une valeur de condition (ou chaque élément d'un `IN`) existe dans
/// la table cible. `None` (dimension libre) et ops de nullité → rien à vérifier.
///
/// `target` est passé en `(table, column)` plutôt que `&Reference` pour permettre
/// de cibler une table non-statique (ex. `car_<via>` d'une caractéristique N1
/// lors d'une sélection traversée).
fn check_ref_value(
    con: &Connection,
    target: Option<(&str, &str)>,
    op: &str,
    val: &Option<JsonValue>,
    label: &str,
) -> Result<(), String> {
    let Some((table, col)) = target else { return Ok(()) };
    if op == "IS NULL" || op == "IS NOT NULL" {
        return Ok(());
    }
    let vals: Vec<String> = match val {
        Some(JsonValue::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(JsonValue::String(s)) => vec![s.clone()],
        _ => return Ok(()),
    };
    for v in vals {
        if v.is_empty() {
            continue;
        }
        if !references::value_exists(con, table, col, &v).map_err(|e| e.to_string())? {
            return Err(format!(
                "{label} : '{v}' inexistant dans {table}.{col}"
            ));
        }
    }
    Ok(())
}

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
    target: String, // "entity", "partner" ou "share"
    dim: String,
    op: String,
    val: Option<JsonValue>,
}

/// Une opération (`operations[]`).
#[derive(Debug, Clone)]
struct Operation {
    /// Numéro d'ordre (métadonnée UI). Validé au parsing comme entier, mais
    /// l'exécuteur n'en dépend pas (les opérations s'exécutent dans l'ordre du
    /// tableau JSON). Conservé pour la cohérence avec la définition stockée.
    #[allow(dead_code)]
    seq: i64,
    level: String,
    selection: Vec<SelectionCond>,
    coefficient: Coefficient,
    multiplicateur: f64,
    destination: Vec<(String, Destination)>,
}

/// Une condition de sélection (`operations[].selection[]`).
///
/// Par défaut, la condition porte directement sur la colonne dimensionnelle de
/// `fact_entry` (`e.<dim>`). Trois traversées optionnelles permettent de filtrer
/// par **attribut** de la dimension :
///
/// - `via` : traverse une caractéristique N1 (regroupement). Le filtre porte
///   alors sur `car_<via>.code` (la valeur N1 du membre). Ex. : tous les
///   comptes dont le `comportement` = `VENTES_IC`.
/// - `ref_field` (sérialisé `ref` en JSON) : traverse une référence directe
///   (patron B, colonne sur la master data). Ex. : tous les comptes dont le
///   `compte_parent` = `60`. Couvre aussi les **FK natives** auto-peuplées
///   (ex. `account.sous_classe`, `entity.entite_parent`) depuis le catalogue
///   `references::NATIVE_MASTER_REFS`.
/// - `attr` : traverse un **enum natif** (`CHECK` du DDL, ex. `account.classe`
///   ∈ {bilan, resultat, flux}). Pas de table cible : la condition porte
///   directement sur la colonne de la master data hôte via un JOIN simple
///   `dim_<host> smda_<host>_<attr>`.
///
/// Les trois sont mutuellement exclusives (exactement une traverse au plus).
#[derive(Debug, Clone)]
struct SelectionCond {
    dim: String,
    op: String,
    val: Option<JsonValue>,
    via: Option<String>,
    ref_field: Option<String>,
    attr: Option<String>,
}

/// Coefficient appliqué au montant source.
///
/// Depuis le moteur de formules ([Q43], `docs/FORMULES.md`), un coefficient est
/// soit un **littéral inline** (`Constant`), soit une **référence nommée** vers
/// la bibliothèque `dim_coefficient` (`Named`). Les anciens coefficients en dur
/// (`pct_integration`, `pct_interet`, `elim_ic_corp_*`) sont désormais des
/// formules seedées dans cette bibliothèque — résolues par
/// [`crate::coefficients::resolve_expr`].
#[derive(Debug, Clone)]
enum Coefficient {
    /// Littéral numérique inline (cas trivial : `{"type": "constant", "value": …}`).
    Constant(f64),
    /// Référence à un coefficient de la bibliothèque (`{"type": "<code>"}`),
    /// natif ou utilisateur. Résolu/compilé à l'exécution.
    Named(String),
}

/// Destination d'une dimension pilotable.
#[derive(Debug, Clone)]
struct Destination {
    mode: String, // "inherit" | "override" | "null" | "map" | "map_ref"
    value: Option<String>,
    /// Mode `map` : caractéristique N1 (`via`) traversée et attribut N2 (`attr`)
    /// dont la valeur surcharge la dimension. La valeur est résolue par jointure
    /// sur la dimension de base de la caractéristique (cf. `exec_operation`).
    via: Option<String>,
    attr: Option<String>,
    /// Mode `map_ref` : référence directe (patron B) portée par la dimension
    /// écrite (ex. `compte_parent` sur `account`). La valeur est résolue par un
    /// seul JOIN sur la master data de la dimension (cf. `exec_operation`).
    /// Stocké `ref_field` car `ref` est un mot-clé Rust ; sérialisé `ref` en JSON.
    ref_field: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Parsing JSON → structures fortement typées
// ─────────────────────────────────────────────────────────────────────────────

fn parse_definition(json: &str, ctx: &RuleContext) -> RuleResult_<Definition> {
    let v: JsonValue =
        serde_json::from_str(json).map_err(|e| format!("définition JSON invalide : {e}"))?;
    let obj = v
        .as_object()
        .ok_or("la définition doit être un objet JSON")?;

    // scope (optionnel, défaut [])
    let scope = match obj.get("scope") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(|v| parse_scope_cond(v, ctx))
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("scope doit être un tableau".into()),
    };

    // operations (obligatoire, non vide idéalement — on tolère vide)
    let operations = match obj.get("operations") {
        Some(JsonValue::Array(a)) => a
            .iter()
            .map(|v| parse_operation(v, ctx))
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("operations doit être un tableau".into()),
        None => return Err("operations manquant".into()),
    };

    Ok(Definition { scope, operations })
}

fn parse_scope_cond(v: &JsonValue, ctx: &RuleContext) -> RuleResult_<ScopeCond> {
    let obj = v.as_object().ok_or("each scope item doit être un objet")?;
    let target = expect_str(obj, "target")?;
    if !ALLOWED_TARGETS.contains(&target.as_str()) {
        return Err(format!(
            "scope.target invalide : {target} (attendu parmi {ALLOWED_TARGETS:?})"
        ));
    }
    let dim = expect_str(obj, "dim")?;
    if !ctx.scope_dims.contains(&dim) {
        return Err(format!(
            "scope.dim invalide : {dim} (attendu parmi {:?})",
            ctx.scope_dims
        ));
    }
    let op = expect_str(obj, "op")?;
    if !ALLOWED_OPS.contains(&op.as_str()) {
        return Err(format!("scope.op invalide : {op}"));
    }
    // `val` est requis (présence) sauf pour IS NULL / IS NOT NULL, et `val: null`
    // explicite n'est accepté que pour ces deux ops (pour les ops binaires, c'est
    // presque sûrement une erreur — l'utilisateur voulait probablement IS NULL).
    // Cohérent avec `parse_selection_cond`.
    let val = obj.get("val").cloned();
    if val.is_none() && op != "IS NULL" && op != "IS NOT NULL" {
        return Err(format!("scope.val manquant pour op='{op}'"));
    }
    if matches!(val, Some(JsonValue::Null)) && op != "IS NULL" && op != "IS NOT NULL" {
        return Err(format!(
            "scope.val null pour op='{op}' — utilisez IS NULL pour tester la nullité"
        ));
    }
    Ok(ScopeCond {
        target,
        dim,
        op,
        val,
    })
}

fn parse_operation(v: &JsonValue, ctx: &RuleContext) -> RuleResult_<Operation> {
    let obj = v.as_object().ok_or("each operation doit être un objet")?;
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
            .map(|v| parse_selection_cond(v, ctx))
            .collect::<RuleResult_<Vec<_>>>()?,
        Some(_) => return Err("selection doit être un tableau".into()),
    };
    let coefficient = match obj.get("coefficient") {
        None | Some(JsonValue::Null) => Coefficient::Constant(1.0),
        Some(c) => parse_coefficient(c)?,
    };
    let multiplicateur = match obj.get("multiplicateur") {
        // Absent → défaut implicite = 1.0 (documenté REGLES_CONSO §4.2).
        None => 1.0,
        // null explicite → presque sûrement un bug client (ex: Number("") = NaN
        // en JS → JSON.stringify(NaN) produit null). On rejette plutôt que de
        // silently tomber sur 1.0, pour rendre le bug visible.
        Some(JsonValue::Null) => {
            return Err(
                "multiplicateur est null — valeur probablement issue d'un champ \
                 vide non validé côté client (Number(\"\") = NaN → JSON null)"
                    .into(),
            )
        }
        Some(JsonValue::Number(n)) => n.as_f64().ok_or("multiplicateur doit être un nombre")?,
        Some(_) => return Err("multiplicateur doit être un nombre".into()),
    };
    let destination = match obj.get("destination") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(JsonValue::Object(map)) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map {
                if !ctx.pilotable_dims.contains(k) {
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

fn parse_selection_cond(v: &JsonValue, ctx: &RuleContext) -> RuleResult_<SelectionCond> {
    let obj = v
        .as_object()
        .ok_or("each selection item doit être un objet")?;
    let dim = expect_str(obj, "dim")?;
    if !ctx.selection_dims.contains(&dim) {
        return Err(format!(
            "selection.dim invalide : {dim} (attendu parmi {:?})",
            ctx.selection_dims
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
    // `val: null` explicite n'est accepté que pour IS NULL / IS NOT NULL
    // (cohérent avec `parse_scope_cond`). Pour les ops binaires, c'est presque
    // sûrement une erreur — l'utilisateur voulait probablement IS NULL.
    if matches!(val, Some(JsonValue::Null)) && op != "IS NULL" && op != "IS NOT NULL" {
        return Err(format!(
            "selection.val null pour op='{op}' — utilisez IS NULL pour tester la nullité"
        ));
    }
    // Traversée optionnelle par attribut : `via` (caractéristique N1), `ref`
    // (référence directe patron B, y compris FK natives auto-peuplées), ou
    // `attr` (enum natif `CHECK` du DDL, ex. `account.classe`). Mutuellement
    // exclusives. Validées à l'enregistrement (validate_definition) et à
    // l'exécution (exec_operation). `level` n'est pas une dimension pilotable
    // et ne peut pas être traversé.
    let via = obj.get("via").and_then(JsonValue::as_str).map(String::from);
    let ref_field = obj
        .get("ref")
        .and_then(JsonValue::as_str)
        .map(String::from);
    let attr = obj
        .get("attr")
        .and_then(JsonValue::as_str)
        .map(String::from);
    let n_traverses = [via.is_some(), ref_field.is_some(), attr.is_some()]
        .iter()
        .filter(|&&b| b)
        .count();
    if n_traverses > 1 {
        return Err(format!(
            "selection.{dim} : 'via', 'ref' et 'attr' sont mutuellement exclusives (traversée d'attribut)"
        ));
    }
    if n_traverses > 0 && dim == "level" {
        return Err(format!(
            "selection.{dim} : 'level' n'est pas une dimension traversable (pas de master data)"
        ));
    }
    Ok(SelectionCond {
        dim,
        op,
        val,
        via,
        ref_field,
        attr,
    })
}

fn parse_coefficient(v: &JsonValue) -> RuleResult_<Coefficient> {
    let obj = v.as_object().ok_or("coefficient doit être un objet")?;
    let t = expect_str(obj, "type")?;
    // `constant` = littéral inline ; tout autre `type` est un **code** de la
    // bibliothèque `dim_coefficient` (natif ou utilisateur), résolu à
    // l'exécution / la validation. L'existence du code est vérifiée par
    // `validate_definition` (POST/PUT) et à l'exécution (`resolve_coefficient`).
    if t == "constant" {
        let value = obj
            .get("value")
            .and_then(|x| x.as_f64())
            .ok_or("coefficient.value doit être un nombre")?;
        Ok(Coefficient::Constant(value))
    } else {
        Ok(Coefficient::Named(t))
    }
}

fn parse_destination(v: &JsonValue) -> RuleResult_<Destination> {
    let obj = v
        .as_object()
        .ok_or("destination.<dim> doit être un objet")?;
    let mode = expect_str(obj, "mode")?;
    let (value, via, attr, ref_field) = match mode.as_str() {
        "inherit" | "null" => (None, None, None, None),
        "override" => (Some(expect_str(obj, "value")?), None, None, None),
        // Mode `map` : la valeur provient de l'attribut N2 `attr` de la
        // caractéristique N1 `via` (existence validée à l'enregistrement et à
        // l'exécution — cf. validate_definition / exec_operation).
        "map" => {
            let via = expect_str(obj, "via")?;
            let attr = expect_str(obj, "attr")?;
            (None, Some(via), Some(attr), None)
        }
        // Mode `map_ref` : la valeur provient d'une référence directe (patron B)
        // portée par la dimension écrite (ex. `compte_parent`). La référence
        // doit être auto-référentielle ou cibler la dimension écrite (validé à
        // l'enregistrement et à l'exécution). `ref` est lu depuis le JSON.
        "map_ref" => {
            let r = expect_str(obj, "ref")?;
            (None, None, None, Some(r))
        }
        other => return Err(format!("destination.mode inconnu : {other}")),
    };
    Ok(Destination {
        mode,
        value,
        via,
        attr,
        ref_field,
    })
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
            // Mode `map` : valeur tirée de l'attribut N2 via l'alias de jointure
            // `cg_<via>` (cf. `exec_operation`). `via`/`attr` sont validés
            // (alphanumériques + existence) avant interpolation.
            "map" => {
                let via = d.via.clone().unwrap_or_default();
                let attr = d.attr.clone().unwrap_or_default();
                format!("cg_{via}.\"{attr}\"")
            }
            // Mode `map_ref` : valeur tirée d'une référence directe via l'alias
            // `mdr_<ref>` (master data de la dimension écrite, cf. exec_operation).
            // `ref` est validé (alphanumérique + existence) avant interpolation.
            "map_ref" => {
                let r = d.ref_field.clone().unwrap_or_default();
                format!("mdr_{r}.\"{r}\"")
            }
            _ => format!("e.{dim}"),
        },
    }
}

/// Résout le coefficient d'une opération en `(expr_sql, joins)`.
///
/// - `Constant(v)` : littéral inline (point décimal garanti).
/// - `Named(code)` : formule de la bibliothèque `dim_coefficient`, compilée par
///   le moteur de formules (cf. [`crate::coefficients::resolve_expr`]). Les
///   coefficients natifs (`pct_integration`, `elim_ic_corp_*`…) sont seedés comme
///   formules — l'ancienne enum codée en dur a disparu (cf. `docs/FORMULES.md`).
///
/// `joins` indique quelles perspectives de `sat_perimeter` l'expression lit, pour
/// que `exec_operation` ajoute les JOINs correspondants.
fn resolve_coefficient(
    con: &Connection,
    c: &Coefficient,
) -> Result<(String, CoeffJoins), duckdb::Error> {
    match c {
        Coefficient::Constant(v) => Ok((format_float(*v), CoeffJoins::default())),
        Coefficient::Named(code) => {
            crate::coefficients::resolve_expr(con, code).map_err(duckdb_synthesis_error)
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
///
/// Le SELECT liste toutes les dimensions propagées (built-in + custom) dans
/// l'ordre du registre, en appliquant les destinations. Les paramètres `?`
/// sont poussés dans l'ordre textuel suivant :
///
/// 1. Pour chaque dim propagated : si `destination.<dim>` est `override`, un
///    `?` est poussé. Toutes les dimensions suivent leur destination, **y
///    compris `analysis2`** (plus de tag `RULE:` — voir la note du corps).
/// 2. `level` : `?` (toujours).
/// 3. Conditions des JOINs `sat_perimeter` (scope).
/// 4. Conditions du WHERE (sélection).
fn exec_operation(
    con: &Connection,
    _rule_code: &str,
    op: &Operation,
    scope: &[ScopeCond],
    ctx: &RuleContext,
) -> Result<usize, duckdb::Error> {
    let snap = format!("_rule_snap_{}", op.level);

    // Valider les destinations `map` et `map_ref` AVANT toute interpolation
    // (via/attr/ref viennent du JSON de la règle). On vérifie : identifiants sûrs
    // (alphanumériques), existence de la cible, et compatibilité de type
    // (la dimension écrite doit correspondre à la dimension cible de la traversée).
    for (dim, d) in &op.destination {
        if d.mode == "map" {
            let via = d.via.as_deref().unwrap_or("");
            let attr = d.attr.as_deref().unwrap_or("");
            if !dimensions::is_valid_custom_name(via) || !dimensions::is_valid_custom_name(attr) {
                return Err(duckdb_synthesis_error(format!(
                    "destination.{dim} map : identifiants invalides (via={via:?}, attr={attr:?})"
                )));
            }
            let target = characteristics::attribute_target(con, via, attr)?.ok_or_else(|| {
                duckdb_synthesis_error(format!(
                    "destination.{dim} map : attribut inconnu {via}.{attr}"
                ))
            })?;
            if target != *dim {
                return Err(duckdb_synthesis_error(format!(
                    "destination.{dim} map : l'attribut {via}.{attr} pointe vers '{target}', \
                     incompatible avec la dimension '{dim}'"
                )));
            }
        } else if d.mode == "map_ref" {
            // Validation `map_ref` : référence directe (patron B) portée par la
            // dimension écrite. Identifiant sûr + existence + target = dim.
            let r = d.ref_field.as_deref().unwrap_or("");
            if !dimensions::is_valid_custom_name(r) {
                return Err(duckdb_synthesis_error(format!(
                    "destination.{dim} map_ref : identifiant invalide (ref={r:?})"
                )));
            }
            let target = crate::custom_references::target_of(con, dim, r)?
                .ok_or_else(|| {
                    duckdb_synthesis_error(format!(
                        "destination.{dim} map_ref : référence inconnue {dim}.{r}"
                    ))
                })?;
            if target != *dim {
                return Err(duckdb_synthesis_error(format!(
                    "destination.{dim} map_ref : la référence {dim}.{r} pointe vers '{target}', \
                     incompatible avec la dimension écrite '{dim}'"
                )));
            }
        }
    }

    // Noms des colonnes propagées = `selection_dims` privé de `level`.
    let propagated: Vec<&str> = ctx
        .selection_dims
        .iter()
        .filter(|s| s.as_str() != "level")
        .map(|s| s.as_str())
        .collect();

    // Pour savoir quels JOINs ajouter :
    //  - p_ent  : si scope sur entity, ou coefficient lisant le périmètre entité
    //  - p_part : si scope sur partner, ou coefficient lisant le périmètre partenaire
    //  - p_ent_n1 / p_part_n1 : coefficients d'élimination IC N-1 (via à-nouveau)
    let (coeff_expr, cj) = resolve_coefficient(con, &op.coefficient)?;
    let scope_has_entity = scope.iter().any(|c| c.target == "entity");
    let scope_has_partner = scope.iter().any(|c| c.target == "partner");
    let scope_has_share = scope.iter().any(|c| c.target == "share");
    let need_p_ent = scope_has_entity || cj.p_ent;
    let need_p_part = scope_has_partner || cj.p_part;
    let need_p_share = scope_has_share;
    let need_p_ent_n1 = cj.p_ent_n1;
    let need_p_part_n1 = cj.p_part_n1;

    // Construction du SELECT et de la liste INSERT dans le même ordre.
    // Chaque dim propagated devient une colonne du SELECT + une colonne INSERT,
    // en suivant sa destination (inherit / override / null) — **y compris
    // `analysis2`**.
    //
    // ⚠ Historiquement `analysis2` recevait un tag `RULE:{code}:{seq}` pour la
    // traçabilité. Mais `analysis2` est une dimension **Analytical** : une ligne
    // dont une analytique est renseignée est un « dont » exclu des totaux
    // (cf. `dimensions::analytical_cols`). Le tag rendait donc **toute** ligne
    // générée par une règle invisible dans le bilan / compte de résultat. On
    // n'écrit plus ce tag : la traçabilité passe par `Nature` (ex. `2ELI`).
    let mut params: Vec<DbValue> = Vec::new();
    let mut select_parts: Vec<String> = Vec::with_capacity(propagated.len() + 2);
    let mut insert_parts: Vec<&str> = Vec::with_capacity(propagated.len() + 2);

    for dim in &propagated {
        insert_parts.push(dim);
        let expr = dest_expr(dim, &op.destination, &mut params);
        select_parts.push(format!("{expr} AS {dim}"));
    }
    // `consolidation_id` : colonne technique (hors dimensions propagées) recopiée
    // depuis le snapshot pour que les écritures de règle restent isolées dans le
    // run courant (le snapshot est filtré par consolidation_id).
    insert_parts.push("consolidation_id");
    select_parts.push("e.consolidation_id AS consolidation_id".to_string());
    insert_parts.push("level");
    insert_parts.push("amount");
    select_parts.push("? AS level".to_string());
    params.push(DbValue::Text(op.level.clone()));
    // Parenthèses autour du coefficient : il peut contenir une opération de
    // niveau supérieur (ex. soustraction de `elim_ic_corp_var`) qui, sans
    // parenthèses, serait mal associée vis-à-vis du `*` (précédence SQL).
    select_parts.push(format!(
        "e.amount * ({coeff_expr}) * {mult} AS amount",
        mult = format_float(op.multiplicateur),
    ));

    let select_clause = format!(
        "SELECT\n    {}\n FROM {} e",
        select_parts.join(",\n    "),
        snap,
    );

    // JOINs sur sat_perimeter.
    let mut joins = String::new();
    if need_p_ent {
        joins.push_str(
            "\nJOIN sat_perimeter p_ent\n  \
                ON p_ent.entity = e.entity\n \
                AND p_ent.perimeter_set = (SELECT c.perimeter_set FROM dim_consolidation c WHERE c.id = e.consolidation_id)\n \
                AND p_ent.period = e.entry_period",
        );
        for c in scope.iter().filter(|c| c.target == "entity") {
            let operand = format!("p_ent.{}", c.dim);
            let cond = push_condition(&operand, &c.op, &c.val, &mut params)
                .map_err(duckdb_synthesis_error)?;
            joins.push_str(&format!("\n AND {cond}"));
        }
    }
    if need_p_part {
        joins.push_str(
            "\nJOIN sat_perimeter p_part\n  \
                ON p_part.entity = e.partner\n \
                AND p_part.perimeter_set = (SELECT c.perimeter_set FROM dim_consolidation c WHERE c.id = e.consolidation_id)\n \
                AND p_part.period = e.entry_period",
        );
        for c in scope.iter().filter(|c| c.target == "partner") {
            let operand = format!("p_part.{}", c.dim);
            let cond = push_condition(&operand, &c.op, &c.val, &mut params)
                .map_err(duckdb_synthesis_error)?;
            joins.push_str(&format!("\n AND {cond}"));
        }
    }
    if need_p_share {
        joins.push_str(
            "\nJOIN sat_perimeter p_share\n  \
                ON p_share.entity = e.share\n \
                AND p_share.perimeter_set = (SELECT c.perimeter_set FROM dim_consolidation c WHERE c.id = e.consolidation_id)\n \
                AND p_share.period = e.entry_period",
        );
        for c in scope.iter().filter(|c| c.target == "share") {
            let operand = format!("p_share.{}", c.dim);
            let cond = push_condition(&operand, &c.op, &c.val, &mut params)
                .map_err(duckdb_synthesis_error)?;
            joins.push_str(&format!("\n AND {cond}"));
        }
    }

    // JOINs de périmètre **N-1** (coefficients d'élimination IC N-1 / Var). Le
    // taux N-1 est lu dans le `sat_perimeter` de la consolidation d'à-nouveau du
    // run courant : `dim_consolidation.a_nouveau_consolidation_id` → son
    // `perimeter_set` à son `exercice`. Même source que le carry d'à-nouveau
    // (cf. `pipeline/a_nouveau.rs`) — pas de duplication, cohérence N-1 garantie.
    //
    // LEFT JOIN volontaire : une entité / un partenaire absent du périmètre N-1
    // (entrant) n'a pas de ligne → `pct_integration` NULL → COALESCE 0 dans
    // l'expression du coefficient (entrant traité comme intégralement nouveau,
    // `Var = N`). Aucun `?` : sous-requêtes scalaires sur `e.consolidation_id`
    // — n'affectent pas l'ordre des paramètres.
    //
    // Si la consolidation n'a pas d'à-nouveau, les sous-requêtes renvoient NULL
    // → la condition de JOIN est fausse → taux N-1 = 0 (dégradation documentée).
    for (alias, key_col) in [("p_ent_n1", "entity"), ("p_part_n1", "partner")] {
        let needed = if alias == "p_ent_n1" {
            need_p_ent_n1
        } else {
            need_p_part_n1
        };
        if needed {
            joins.push_str(&format!(
                "\nLEFT JOIN sat_perimeter {alias}\n  \
                    ON {alias}.entity = e.{key_col}\n \
                    AND {alias}.perimeter_set = (\
                        SELECT s_an.perimeter_set FROM dim_consolidation s_cur \
                        JOIN dim_consolidation s_an ON s_an.id = s_cur.a_nouveau_consolidation_id \
                        WHERE s_cur.id = e.consolidation_id)\n \
                    AND {alias}.period = (\
                        SELECT s_an.exercice FROM dim_consolidation s_cur \
                        JOIN dim_consolidation s_an ON s_an.id = s_cur.a_nouveau_consolidation_id \
                        WHERE s_cur.id = e.consolidation_id)"
            ));
        }
    }

    // JOINs des caractéristiques (destinations `map`). Pour chaque N1 distincte
    // `via`, joindre la dimension de base puis la table de valeurs `car_<via>`.
    // INNER JOIN volontaire : seules les lignes dont le membre est **classé**
    // (a une valeur N1, présente dans `car_<via>`) génèrent une écriture.
    let mut map_vias: Vec<String> = Vec::new();
    for (_, d) in &op.destination {
        if d.mode == "map" {
            if let Some(via) = &d.via {
                if !map_vias.contains(via) {
                    map_vias.push(via.clone());
                }
            }
        }
    }
    for via in &map_vias {
        // `via` déjà validé (identifiant sûr) par la boucle de validation map.
        let base_dim = characteristics::base_dimension_of(con, via)?.ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "destination map : caractéristique inconnue : {via}"
            ))
        })?;
        let (base_table, base_key) = references::dimension_master(&base_dim).ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "destination map : dimension de base sans master data : {base_dim}"
            ))
        })?;
        let value_table = format!("car_{via}");
        joins.push_str(&format!(
            "\nJOIN {base_table} md_{via}\n  ON md_{via}.{base_key} = e.{base_dim}\
             \nJOIN {value_table} cg_{via}\n  ON cg_{via}.code = md_{via}.\"{via}\""
        ));
    }

    // JOINs des références directes (destinations `map_ref`, patron B). Pour
    // chaque `ref` distincte, joindre la master data de la dimension écrite
    // (alias `mdr_<ref>` pour éviter toute collision avec les alias `md_<via>`
    // / `cg_<via>` des caractéristiques). INNER JOIN volontaire : seules les
    // lignes dont le membre a une valeur de référence génèrent une écriture
    // (symétrique au comportement du mode `map`).
    let mut map_refs: Vec<String> = Vec::new();
    for (_, d) in &op.destination {
        if d.mode == "map_ref" {
            if let Some(r) = &d.ref_field {
                if !map_refs.contains(r) {
                    map_refs.push(r.clone());
                }
            }
        }
    }
    for r in &map_refs {
        // `r` déjà validé (identifiant sûr + existence + target = dim) par la
        // boucle de validation map_ref. On retrouve la dimension hôte (écrite)
        // via le registre.
        let host_dim = op
            .destination
            .iter()
            .find(|(_, d)| d.mode == "map_ref" && d.ref_field.as_deref() == Some(r.as_str()))
            .map(|(dim, _)| dim.clone())
            .ok_or_else(|| {
                duckdb_synthesis_error(format!(
                    "destination map_ref : dimension hôte introuvable pour ref={r}"
                ))
            })?;
        let (host_table, host_key) = references::dimension_master(&host_dim).ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "destination map_ref : dimension hôte sans master data : {host_dim}"
            ))
        })?;
        // La condition `IS NOT NULL` sur la colonne référencée est essentielle :
        // sans elle, l'INNER JOIN réussirait même pour les membres sans valeur
        // (la master data existe toujours), écrivant un `NULL` dans fact_entry
        // (rejeté par la contrainte NOT NULL). Symétrique au comportement `map`
        // (qui exclut les membres non classés via le double JOIN sur car_<via>).
        joins.push_str(&format!(
            "\nJOIN {host_table} mdr_{r}\n  ON mdr_{r}.{host_key} = e.{host_dim}\
             \n  AND mdr_{r}.\"{r}\" IS NOT NULL"
        ));
    }

    // JOINs des traversées de sélection (caractéristiques N1 `via`, références
    // directes `ref`, et enums natifs `attr`). Alias **préfixés par `s`**
    // (smd_/scg_/smdr_/smda_) pour éviter toute collision avec les JOINs de
    // destination (md_/cg_/mdr_). INNER JOIN volontaire : seules les lignes dont
    // le membre est classé / a une valeur de référence sont éligibles au filtre.
    let mut sel_vias: Vec<String> = Vec::new();
    let mut sel_refs: Vec<(String, String)> = Vec::new(); // (ref_column, dim) — dim utile pour le JOIN
    let mut sel_attrs: Vec<(String, String)> = Vec::new(); // (attr_column, dim)
    for s in &op.selection {
        if let Some(via) = &s.via {
            // Validation runtime (identifiant sûr + base_dimension = dim).
            if !dimensions::is_valid_custom_name(via) {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} via : identifiant invalide ({via:?})",
                    s.dim
                )));
            }
            let base = characteristics::base_dimension_of(con, via)?.ok_or_else(|| {
                duckdb_synthesis_error(format!(
                    "selection.{} via : caractéristique inconnue : {via}",
                    s.dim
                ))
            })?;
            if base != s.dim {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} via : la caractéristique '{via}' a pour base '{base}', pas '{}'",
                    s.dim, s.dim
                )));
            }
            if !sel_vias.contains(via) {
                sel_vias.push(via.clone());
            }
        } else if let Some(rf) = &s.ref_field {
            if !dimensions::is_valid_custom_name(rf) {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} ref : identifiant invalide ({rf:?})",
                    s.dim
                )));
            }
            let target =
                crate::custom_references::target_of(con, &s.dim, rf)?.ok_or_else(|| {
                    duckdb_synthesis_error(format!(
                        "selection.{} ref : référence inconnue : {}.{}",
                        s.dim, s.dim, rf
                    ))
                })?;
            // `target` doit avoir une master data (dimension d'écriture ou
            // master data secondaire résolvable par `target_master`).
            if references::target_master(con, &target).is_none() {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} ref : la cible '{target}' n'a pas de master data",
                    s.dim
                )));
            }
            let key = (rf.clone(), s.dim.clone());
            if !sel_refs.contains(&key) {
                sel_refs.push(key);
            }
        } else if let Some(attr) = &s.attr {
            // Validation runtime : l'enum natif doit exister dans le catalogue
            // (re-validation défensive, déjà faite dans validate_definition).
            if !dimensions::is_valid_custom_name(attr) {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} attr : identifiant invalide ({attr:?})",
                    s.dim
                )));
            }
            if references::native_enum_lookup(&s.dim, attr).is_none() {
                return Err(duckdb_synthesis_error(format!(
                    "selection.{} attr : enum natif inconnu : {}.{}",
                    s.dim, s.dim, attr
                )));
            }
            let key = (attr.clone(), s.dim.clone());
            if !sel_attrs.contains(&key) {
                sel_attrs.push(key);
            }
        }
    }
    // JOINs caractéristiques N1 de sélection : dim_<base> smd_<via> + car_<via> scg_<via>.
    for via in &sel_vias {
        let base_dim = characteristics::base_dimension_of(con, via)?.ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "selection via : caractéristique inconnue : {via}"
            ))
        })?;
        let (base_table, base_key) = references::dimension_master(&base_dim).ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "selection via : dimension de base sans master data : {base_dim}"
            ))
        })?;
        let value_table = format!("car_{via}");
        joins.push_str(&format!(
            "\nJOIN {base_table} smd_{via}\n  ON smd_{via}.{base_key} = e.{base_dim}\
             \nJOIN {value_table} scg_{via}\n  ON scg_{via}.code = smd_{via}.\"{via}\""
        ));
    }
    // JOINs références directes de sélection : dim_<host> smdr_<ref>.
    // `IS NOT NULL` sur la colonne référencée pour exclure les membres sans
    // valeur de référence (symétrique au comportement destination `map_ref`).
    for (rf, host_dim) in &sel_refs {
        let (host_table, host_key) = references::dimension_master(host_dim).ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "selection ref : dimension hôte sans master data : {host_dim}"
            ))
        })?;
        joins.push_str(&format!(
            "\nJOIN {host_table} smdr_{rf}\n  ON smdr_{rf}.{host_key} = e.{host_dim}\
             \n  AND smdr_{rf}.\"{rf}\" IS NOT NULL"
        ));
    }
    // JOINs enums natifs de sélection : dim_<host> smda_<host>_<attr>.
    // Alias suffixé par la dimension pour éviter les collisions si plusieurs
    // dimensions portent un enum du même nom (ex. classe sur account/sous_classe).
    for (attr, host_dim) in &sel_attrs {
        let (host_table, host_key) = references::dimension_master(host_dim).ok_or_else(|| {
            duckdb_synthesis_error(format!(
                "selection attr : dimension hôte sans master data : {host_dim}"
            ))
        })?;
        joins.push_str(&format!(
            "\nJOIN {host_table} smda_{host_dim}_{attr}\n  ON smda_{host_dim}_{attr}.{host_key} = e.{host_dim}"
        ));
    }

    // WHERE (sélection). L'opérande dépend de la traverse éventuelle :
    // - via N1 → `scg_<via>.code` (valeur N1 du membre).
    // - ref    → `smdr_<ref>.<ref>` (colonne de référence directe).
    // - attr   → `smda_<dim>_<attr>.<attr>` (colonne enum natif sur master data).
    // - sinon  → `e.<dim>` (comportement historique).
    let mut where_clauses: Vec<String> = Vec::new();
    for sel in &op.selection {
        let operand = if let Some(via) = &sel.via {
            format!("scg_{via}.code")
        } else if let Some(rf) = &sel.ref_field {
            format!("smdr_{rf}.\"{rf}\"")
        } else if let Some(attr) = &sel.attr {
            format!("smda_{}_{}.{}", sel.dim, attr, attr)
        } else {
            format!("e.{}", sel.dim)
        };
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
        "INSERT INTO fact_entry\n    ({})\n{}{}{}",
        insert_parts.join(", "),
        select_clause,
        joins,
        where_clause,
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
///    d. Pour chaque niveau `L` : `DROP TABLE _rule_snap_L` (et, si
///       [`RECONSTRUCT_CLOSURES_AFTER_RULE`] est réactivé, `materialize_closures(L)`).
///
/// Les snapshots empêchent qu'une opération lise les lignes générées par une
/// opération précédente (idempotence de la règle).
pub fn run_ruleset(
    con: &Connection,
    ruleset_code: &str,
    consolidation_id: Option<i64>,
) -> Result<RulesetReport, duckdb::Error> {
    // Contexte dynamique : whitelists calculées depuis le registre des dimensions.
    let ctx = RuleContext::from_registry(con)?;

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
        let definition = parse_definition(definition_json, &ctx).map_err(duckdb_synthesis_error)?;

        // Niveaux distincts touchés par les opérations de la règle.
        let levels: BTreeSet<&str> = definition
            .operations
            .iter()
            .map(|op| op.level.as_str())
            .collect();

        // Snapshots (un par niveau). Filtrés par `consolidation_id` quand fourni
        // (isolation du run courant ; `None` = lecture globale, pour les tests).
        for lvl in &levels {
            let sql = if consolidation_id.is_some() {
                format!(
                    "CREATE TEMP TABLE _rule_snap_{lvl} AS \
                     SELECT * FROM fact_entry WHERE level = ? AND consolidation_id = ?"
                )
            } else {
                format!(
                    "CREATE TEMP TABLE _rule_snap_{lvl} AS \
                     SELECT * FROM fact_entry WHERE level = ?"
                )
            };
            if let Some(cid) = consolidation_id {
                con.execute(&sql, params![lvl.to_string(), cid])?;
            } else {
                con.execute(&sql, [lvl.to_string()])?;
            }
        }

        // Exécution des opérations. On agrège le nombre de lignes générées
        // par niveau pour produire un RuleResult par (règle, niveau).
        let mut generated_per_level: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for op in &definition.operations {
            let n = exec_operation(con, rule_code, op, &definition.scope, &ctx)?;
            *generated_per_level.entry(op.level.clone()).or_default() += n;
        }

        // Cleanup des snapshots (+ reconstruction des clôtures si réactivée).
        for lvl in &levels {
            let drop_sql = format!("DROP TABLE _rule_snap_{lvl}");
            // On ignore l'erreur du DROP (la table peut déjà être absente).
            let _ = con.execute(&drop_sql, []);
            if RECONSTRUCT_CLOSURES_AFTER_RULE {
                materialize_closures(con, lvl)?;
            }
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

/// Exécute uniquement les opérations d'un ruleset qui **ciblent `level`**, sur
/// le `fact_entry` courant à ce niveau.
///
/// Destinée à être appelée par le **hook du pipeline** ([`crate::pipeline::
/// run_pipeline_with_hook`]) juste après l'étape qui produit `level` : les
/// lignes générées sont alors propagées par les étapes suivantes (ex. une règle
/// `converted` est consolidée par l'étape D). Les rulesets dont aucune règle ne
/// cible `level` ne génèrent rien.
///
/// Même sémantique de snapshot que [`run_ruleset`] (isolation des opérations
/// d'une règle, chaque règle voyant la sortie de la précédente), mais restreinte
/// au niveau donné. Renvoie un `RuleResult` par règle ayant généré des lignes.
pub fn run_ruleset_at_level(
    con: &Connection,
    ruleset_code: &str,
    level: &str,
    consolidation_id: Option<i64>,
) -> Result<Vec<RuleResult>, duckdb::Error> {
    let ctx = RuleContext::from_registry(con)?;
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
            v.push(r?);
        }
        v
    };

    let mut results: Vec<RuleResult> = Vec::new();
    for (rule_code, definition_json) in &rules {
        let definition = parse_definition(definition_json, &ctx).map_err(duckdb_synthesis_error)?;
        let has_ops_here = definition.operations.iter().any(|op| op.level == level);
        if !has_ops_here {
            continue;
        }
        // Snapshot du niveau (isolation des opérations de la règle), filtré par
        // `consolidation_id` quand fourni (isolation du run courant).
        if let Some(cid) = consolidation_id {
            con.execute(
                &format!(
                    "CREATE TEMP TABLE _rule_snap_{level} AS \
                     SELECT * FROM fact_entry WHERE level = ? AND consolidation_id = ?"
                ),
                params![level, cid],
            )?;
        } else {
            con.execute(
                &format!(
                    "CREATE TEMP TABLE _rule_snap_{level} AS \
                     SELECT * FROM fact_entry WHERE level = ?"
                ),
                [level],
            )?;
        }
        let mut n = 0usize;
        for op in definition.operations.iter().filter(|op| op.level == level) {
            n += exec_operation(con, rule_code, op, &definition.scope, &ctx)?;
        }
        let _ = con.execute(&format!("DROP TABLE _rule_snap_{level}"), []);
        if RECONSTRUCT_CLOSURES_AFTER_RULE {
            materialize_closures(con, level)?;
        }
        if n > 0 {
            results.push(RuleResult {
                rule_code: rule_code.clone(),
                level: level.to_string(),
                generated: n,
            });
        }
    }
    Ok(results)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests unitaires — parsing et helpers SQL (fonctions privées).
//
//  Ces tests valident le parsing JSON → structures fortement typées et les
//  helpers de génération SQL, indépendamment de toute base. Ils ciblent
//  spécifiquement les branches de rejet par les whitelists (sécurité SQL) que
//  les tests d'intégration tests/rules.rs et rules_test.py ne couvrent qu'im-
//  plicitement (via un code d'erreur HTTP générique).
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::types::Value as DbValue;

    /// Contexte de test minimal : reproduit la forme d'un RuleContext construit
    /// depuis `from_registry` sur un schéma standard (12 dimensions built-in,
    /// 5 colonnes sat_perimeter, pas de custom). On l'utilise pour valider le
    /// parsing sans avoir à ouvrir une DuckDB.
    fn ctx_fixture() -> RuleContext {
        RuleContext {
            selection_dims: vec![
                "phase".into(),
                "entity".into(),
                "entry_period".into(),
                "period".into(),
                "account".into(),
                "flow".into(),
                "currency".into(),
                "nature".into(),
                "partner".into(),
                "share".into(),
                "analysis".into(),
                "analysis2".into(),
                "level".into(),
            ],
            pilotable_dims: vec![
                "entity".into(),
                "account".into(),
                "flow".into(),
                "nature".into(),
                "partner".into(),
                "share".into(),
                "analysis".into(),
                "analysis2".into(),
            ],
            scope_dims: vec![
                "methode".into(),
                "pct_interet".into(),
                "pct_integration".into(),
                "entree".into(),
                "sortie".into(),
            ],
        }
    }

    /// Définition JSON complète de la règle d'élimination interco à 4 opérations
    /// (miroir de celle du test Python `rules_test.py`).
    fn elim_interco_json() -> &'static str {
        r#"{
            "scope": [
                {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
                {"target": "partner", "dim": "methode", "op": "=", "val": "globale"}
            ],
            "operations": [
                {
                    "seq": 1, "level": "consolidated",
                    "selection": [
                        {"dim": "account", "op": "=", "val": "700"},
                        {"dim": "partner", "op": "IS NOT NULL"}
                    ],
                    "coefficient": {"type": "pct_integration"},
                    "multiplicateur": -1,
                    "destination": {
                        "nature":  {"mode": "override", "value": "2ELI"},
                        "partner": {"mode": "inherit"}
                    }
                },
                {
                    "seq": 2, "level": "consolidated",
                    "selection": [
                        {"dim": "account", "op": "=", "val": "600"},
                        {"dim": "partner", "op": "IS NOT NULL"}
                    ],
                    "coefficient": {"type": "pct_integration"},
                    "multiplicateur": -1,
                    "destination": {
                        "nature":  {"mode": "override", "value": "2ELI"},
                        "partner": {"mode": "null"}
                    }
                }
            ]
        }"#
    }

    // ── Parsing valide ───────────────────────────────────────────────────

    #[test]
    fn parse_definition_valide_interco() {
        let ctx = ctx_fixture();
        let def = parse_definition(elim_interco_json(), &ctx).expect("définition valide");
        assert_eq!(def.scope.len(), 2, "2 conditions de scope");
        assert_eq!(def.operations.len(), 2, "2 opérations");
        // Le scope croisé entity + partner est reconnu.
        assert!(def.scope.iter().any(|c| c.target == "entity"));
        assert!(def.scope.iter().any(|c| c.target == "partner"));
        // Opération 1 : multiplicateur -1, coefficient PctIntegration, partner hérité.
        let op1 = &def.operations[0];
        assert_eq!(op1.seq, 1);
        assert_eq!(op1.level, "consolidated");
        assert!(matches!(&op1.coefficient, Coefficient::Named(c) if c == "pct_integration"));
        assert!((op1.multiplicateur - (-1.0)).abs() < 1e-9);
        // Destination nature override 2ELI, partner inherit.
        let dest_nature = op1.destination.iter().find(|(k, _)| k == "nature");
        assert!(matches!(dest_nature, Some((_, d)) if d.mode == "override"
                                              && d.value.as_deref() == Some("2ELI")));
    }

    #[test]
    fn parse_definition_accepte_scope_vide_et_sans_coefficient() {
        // Une opération minimale : coefficient implicite = Constant(1.0),
        // multiplicateur implicite = 1.0, destination vide.
        let ctx = ctx_fixture();
        let json = r#"{ "operations": [ { "seq": 1, "level": "converted" } ] }"#;
        let def = parse_definition(json, &ctx).expect("définition minimale valide");
        assert!(def.scope.is_empty(), "scope par défaut = vide");
        let op = &def.operations[0];
        assert!(
            matches!(op.coefficient, Coefficient::Constant(v) if (v - 1.0).abs() < 1e-9),
            "coefficient implicite = Constant(1.0)"
        );
        assert!(
            (op.multiplicateur - 1.0).abs() < 1e-9,
            "multiplicateur implicite = 1.0"
        );
        assert!(op.destination.is_empty(), "destination vide par défaut");
        assert!(op.selection.is_empty(), "sélection vide par défaut");
    }

    #[test]
    fn parse_coefficients_elim_ic() {
        // Les coefficients d'élimination IC sont désormais des **références
        // nommées** vers la bibliothèque (Coefficient::Named), résolues plus tard.
        let ctx = ctx_fixture();
        for ty in ["elim_ic_corp_n", "elim_ic_corp_n1", "elim_ic_corp_var"] {
            let json = format!(
                r#"{{ "operations":[{{
                    "seq":1,"level":"corporate",
                    "coefficient":{{"type":"{ty}"}}
                }}] }}"#
            );
            let def = parse_definition(&json, &ctx)
                .unwrap_or_else(|e| panic!("coefficient {ty} devrait parser : {e}"));
            assert!(
                matches!(&def.operations[0].coefficient, Coefficient::Named(c) if c == ty),
                "coefficient {ty} : devrait être Named({ty})"
            );
        }
    }

    #[test]
    fn resolve_coefficient_elim_ic_joins() {
        // Les besoins de JOIN sont correctement signalés via la résolution des
        // coefficients natifs seedés : N → p_ent+p_part, N-1 → p_ent_n1+p_part_n1,
        // Var → les quatre. (Résolution = parsing + compilation de la formule.)
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let (_, jn) = resolve_coefficient(&con, &Coefficient::Named("elim_ic_corp_n".into())).unwrap();
        assert!(jn.p_ent && jn.p_part && !jn.p_ent_n1 && !jn.p_part_n1);
        let (_, j1) =
            resolve_coefficient(&con, &Coefficient::Named("elim_ic_corp_n1".into())).unwrap();
        assert!(!j1.p_ent && !j1.p_part && j1.p_ent_n1 && j1.p_part_n1);
        let (expr_var, jv) =
            resolve_coefficient(&con, &Coefficient::Named("elim_ic_corp_var".into())).unwrap();
        assert!(jv.p_ent && jv.p_part && jv.p_ent_n1 && jv.p_part_n1);
        // L'expression Var est bien une soustraction de deux ratios.
        assert!(expr_var.contains(" - "), "Var = ratio N - ratio N-1");
        // On n'utilise jamais LEAST (sémantique NULL dangereuse sous DuckDB).
        assert!(!expr_var.to_uppercase().contains("LEAST"));
    }

    // ── Rejets par les whitelists (sécurité SQL) ─────────────────────────

    #[test]
    fn parse_rejette_scope_target_invalide() {
        let ctx = ctx_fixture();
        let json = r#"{ "scope": [{"target":"company","dim":"methode","op":"=","val":"globale"}],
                        "operations":[{"seq":1,"level":"consolidated"}] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("ALLOWED_TARGETS") || err.contains("target"),
            "message devrait mentionner target invalide : {err}"
        );
    }

    #[test]
    fn parse_rejette_scope_dim_hors_whitelist() {
        let ctx = ctx_fixture();
        // "pct_inconnu" n'est pas une colonne de sat_perimeter.
        let json = r#"{ "scope": [{"target":"entity","dim":"pct_inconnu","op":"=","val":1}],
                        "operations":[{"seq":1,"level":"consolidated"}] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("scope.dim") || err.contains("invalide"),
            "message devrait mentionner scope.dim invalide : {err}"
        );
    }

    #[test]
    fn parse_rejette_scope_val_null_avec_op_eq() {
        let ctx = ctx_fixture();
        let json = r#"{ "scope": [{"target":"entity","dim":"methode","op":"=","val":null}],
                        "operations":[{"seq":1,"level":"consolidated"}] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("null"),
            "message devrait mentionner null : {err}"
        );
    }

    #[test]
    fn parse_rejette_selection_val_null_avec_op_eq() {
        // Miroir de parse_rejette_scope_val_null_avec_op_eq pour la sélection :
        // `val: null` explicite avec op binaire doit être rejeté (cohérence).
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","op":"=","val":null}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("null"),
            "message devrait mentionner null : {err}"
        );
    }

    #[test]
    fn parse_accepte_scope_sans_val_avec_is_null() {
        // Pour IS NULL / IS NOT NULL, l'absence de `val` est tolérée (cohérent
        // avec `parse_selection_cond`).
        let ctx = ctx_fixture();
        for op in &["IS NULL", "IS NOT NULL"] {
            let json = format!(
                "{{ \"scope\": [{{\"target\":\"entity\",\"dim\":\"entree\",\"op\":\"{op}\"}}], \
                 \"operations\":[{{\"seq\":1,\"level\":\"consolidated\"}}] }}"
            );
            let def = parse_definition(&json, &ctx)
                .unwrap_or_else(|e| panic!("op={op} devrait tolérer l'absence de val : {e}"));
            assert_eq!(def.scope.len(), 1);
            assert!(
                def.scope[0].val.is_none(),
                "op={op} : val devrait être None"
            );
        }
    }

    #[test]
    fn parse_accepte_scope_avec_val_explicite() {
        // `val` présent reste valide pour tous les opérateurs binaires.
        let ctx = ctx_fixture();
        let json = r#"{ "scope": [{"target":"entity","dim":"methode","op":"=","val":"globale"}],
                        "operations":[{"seq":1,"level":"consolidated"}] }"#;
        let def = parse_definition(json, &ctx).expect("val explicite valide");
        assert_eq!(def.scope.len(), 1);
        match &def.scope[0].val {
            Some(JsonValue::String(s)) => assert_eq!(s, "globale"),
            other => panic!("val inattendu : {other:?}"),
        }
    }

    #[test]
    fn parse_rejette_level_invalide() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{"seq":1,"level":"consolid"}] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("level"),
            "message devrait mentionner level : {err}"
        );
    }

    #[test]
    fn parse_rejette_selection_dim_hors_whitelist() {
        let ctx = ctx_fixture();
        // "level" est un nom réservé : il est dans selection_dims, donc accepté.
        // "foobar" n'est pas une dimension connue → rejeté.
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"foobar","op":"=","val":"x"}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("selection.dim") || err.contains("invalide"),
            "message devrait mentionner selection.dim : {err}"
        );
    }

    #[test]
    fn parse_rejette_destination_dim_non_pilotable() {
        let ctx = ctx_fixture();
        // "currency" est une dimension Fixed (non pilotable) → refusée en
        // destination (elle doit rester héritée).
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "destination":{"currency":{"mode":"override","value":"USD"}}
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("pilotable") || err.contains("destination"),
            "message devrait mentionner pilotable/destination : {err}"
        );
    }

    #[test]
    fn parse_rejette_destination_mode_inconnu() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "destination":{"nature":{"mode":"swap","value":"X"}}
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("mode"),
            "message devrait mentionner mode : {err}"
        );
    }

    #[test]
    fn coefficient_type_inconnu_accepte_au_parsing_rejete_a_la_validation() {
        // Depuis le moteur de formules : un `type` libre est un **code** de
        // coefficient (Named), accepté au parsing ; son existence est vérifiée à
        // la validation (`validate_definition`, qui résout contre dim_coefficient).
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "coefficient":{"type":"pct_share"}
        }] }"#;
        let def = parse_definition(json, &ctx).expect("parsing accepte un code libre");
        assert!(matches!(&def.operations[0].coefficient, Coefficient::Named(c) if c == "pct_share"));

        // Validation contre la base : le code 'pct_share' n'existe pas → rejet.
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let err = validate_definition(&con, json).unwrap_err();
        assert!(
            err.contains("pct_share") || err.contains("inconnu"),
            "message devrait mentionner le coefficient inconnu : {err}"
        );
    }

    #[test]
    fn parse_rejette_coefficient_constant_sans_value() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "coefficient":{"type":"constant"}
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("value"),
            "message devrait mentionner value : {err}"
        );
    }

    #[test]
    fn parse_rejette_operations_manquant() {
        let ctx = ctx_fixture();
        let json = r#"{ "scope": [] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("operations"),
            "message devrait mentionner operations : {err}"
        );
    }

    #[test]
    fn parse_rejette_operations_non_tableau() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations": "oops" }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("tableau"),
            "message devrait mentionner tableau : {err}"
        );
    }

    // ── Sélection étendue (via / ref) ────────────────────────────────────

    #[test]
    fn parse_accepte_selection_avec_via_n1() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","via":"regroupement","op":"=","val":"PROD"}]
        }] }"#;
        let def = parse_definition(json, &ctx).expect("via N1 valide");
        let s = &def.operations[0].selection[0];
        assert_eq!(s.dim, "account");
        assert_eq!(s.via.as_deref(), Some("regroupement"));
        assert!(s.ref_field.is_none(), "via et ref sont exclusifs");
    }

    #[test]
    fn parse_accepte_selection_avec_ref_patron_b() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","ref":"compte_parent","op":"=","val":"700"}]
        }] }"#;
        let def = parse_definition(json, &ctx).expect("ref patron B valide");
        let s = &def.operations[0].selection[0];
        assert_eq!(s.dim, "account");
        assert_eq!(s.ref_field.as_deref(), Some("compte_parent"));
        assert!(s.via.is_none(), "via et ref sont exclusifs");
    }

    #[test]
    fn parse_rejette_selection_via_et_ref_simultanes() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","via":"x","ref":"y","op":"=","val":"z"}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("exclusives") || err.contains("via") || err.contains("ref"),
            "doit rejeter via+ref simultanés : {err}"
        );
    }

    #[test]
    fn parse_rejette_selection_via_sur_level() {
        // `level` n'est pas une dimension traversable (pas de master data).
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"level","via":"x","op":"=","val":"z"}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("level") || err.contains("traversable"),
            "doit rejeter la traversée de level : {err}"
        );
    }

    // ── Destination `map_ref` ─────────────────────────────────────────────

    #[test]
    fn parse_accepte_destination_map_ref_avec_ref() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "destination":{"account":{"mode":"map_ref","ref":"compte_parent"}}
        }] }"#;
        let def = parse_definition(json, &ctx).expect("map_ref valide");
        let (_, d) = def.operations[0]
            .destination
            .iter()
            .find(|(k, _)| k == "account")
            .expect("destination account");
        assert_eq!(d.mode, "map_ref");
        assert_eq!(d.ref_field.as_deref(), Some("compte_parent"));
        assert!(d.via.is_none() && d.attr.is_none());
    }

    #[test]
    fn parse_rejette_destination_map_ref_sans_ref() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "destination":{"account":{"mode":"map_ref"}}
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("ref") || err.contains("manquant"),
            "doit exiger ref pour map_ref : {err}"
        );
    }

    #[test]
    fn parse_coefficient_constant_avec_value() {
        let v = serde_json::json!({"type":"constant","value":0.25});
        let c = parse_coefficient(&v).expect("coefficient constant valide");
        assert!(matches!(c, Coefficient::Constant(x) if (x - 0.25).abs() < 1e-9));
    }

    #[test]
    fn parse_multiplicateur_absent_donne_1() {
        // Absence de la clé → défaut implicite = 1.0 (documenté).
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{"seq":1,"level":"consolidated"}] }"#;
        let def = parse_definition(json, &ctx).expect("multiplicateur absent = 1.0");
        assert!((def.operations[0].multiplicateur - 1.0).abs() < 1e-9);
    }

    #[test]
    fn parse_multiplicateur_null_explicite_rejete() {
        // null explicite (bug UI typique : Number("") = NaN → JSON null)
        // doit être rejeté plutôt que silencieusement remplacé par 1.0.
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{"seq":1,"level":"consolidated","multiplicateur":null}] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("null"),
            "le message doit mentionner null : {err}"
        );
    }

    // ── Helpers SQL ──────────────────────────────────────────────────────

    #[test]
    fn resolve_coefficient_pct_integration_necessite_join_perimeter() {
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let (expr, joins) =
            resolve_coefficient(&con, &Coefficient::Named("pct_integration".into())).unwrap();
        assert!(
            joins.p_ent,
            "pct_integration doit déclencher le JOIN sat_perimeter (p_ent)"
        );
        assert!(
            expr.contains("pct_integration"),
            "expression doit lire pct_integration : {expr}"
        );
    }

    #[test]
    fn resolve_coefficient_constant_ne_necessite_pas_join() {
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let (expr, joins) = resolve_coefficient(&con, &Coefficient::Constant(0.5)).unwrap();
        assert!(
            !joins.p_ent && !joins.p_part && !joins.p_ent_n1 && !joins.p_part_n1,
            "Constant n'a pas besoin du JOIN sat_perimeter"
        );
        assert!(
            !expr.contains("pct_"),
            "expression ne doit pas lire le périmètre : {expr}"
        );
    }

    #[test]
    fn format_float_point_decimal_et_un_chiffre() {
        // Entier : "1.0" (force un chiffre après le point pour SQL).
        assert_eq!(format_float(1.0), "1.0");
        assert_eq!(format_float(2.0), "2.0");
        assert_eq!(format_float(-1.0), "-1.0");
        // Fractionnaire : représentation Rust par défaut (point décimal).
        assert_eq!(format_float(0.5), "0.5");
        assert_eq!(format_float(-0.25), "-0.25");
    }

    #[test]
    fn push_condition_in_liste_vide_rend_faux() {
        // Une sélection IN () est invalide en SQL ; le helper doit produire 1=0
        // plutôt que d'injecter une liste vide.
        let mut params: Vec<DbValue> = Vec::new();
        let cond = push_condition(
            "e.account",
            "IN",
            &Some(JsonValue::Array(vec![])),
            &mut params,
        )
        .expect("IN liste vide → 1=0");
        assert_eq!(cond, "1=0", "IN liste vide doit donner 1=0, eu {cond}");
        assert!(params.is_empty(), "aucun paramètre bindé pour 1=0");
    }

    #[test]
    fn push_condition_in_avec_plusieurs_valeurs() {
        let mut params: Vec<DbValue> = Vec::new();
        let val = JsonValue::Array(vec![
            JsonValue::String("100".into()),
            JsonValue::String("200".into()),
        ]);
        let cond = push_condition("e.account", "IN", &Some(val), &mut params).expect("IN ok");
        assert_eq!(cond, "e.account IN (?, ?)");
        assert_eq!(params.len(), 2, "deux valeurs bindées");
    }

    #[test]
    fn push_condition_op_simple_binde_une_valeur() {
        let mut params: Vec<DbValue> = Vec::new();
        let cond = push_condition(
            "e.amount",
            ">",
            &Some(JsonValue::Number(serde_json::Number::from(100))),
            &mut params,
        )
        .expect("op > ok");
        assert_eq!(cond, "e.amount > ?");
        assert_eq!(params.len(), 1, "une valeur bindée");
    }

    #[test]
    fn push_condition_is_null_ne_bind_rien() {
        let mut params: Vec<DbValue> = Vec::new();
        let cond = push_condition("e.partner", "IS NULL", &None, &mut params).expect("IS NULL ok");
        assert_eq!(cond, "e.partner IS NULL");
        assert!(params.is_empty(), "IS NULL ne bind rien");
    }

    #[test]
    fn dest_expr_héritage_par_défaut_quand_dim_absente() {
        // Une dimension absente de la liste des destinations est héritée.
        let mut params: Vec<DbValue> = Vec::new();
        let expr = dest_expr("account", &[], &mut params);
        assert_eq!(expr, "e.account", "héritage par défaut");
        assert!(params.is_empty());
    }

    #[test]
    fn dest_expr_null_pousse_null_littéral() {
        let dests = vec![(
            "partner".to_string(),
            Destination {
                mode: "null".into(),
                value: None,
                via: None,
                attr: None,
                ref_field: None,
            },
        )];
        let mut params: Vec<DbValue> = Vec::new();
        let expr = dest_expr("partner", &dests, &mut params);
        assert_eq!(expr, "NULL", "mode null → NULL littéral");
        assert!(params.is_empty(), "mode null ne bind rien");
    }

    #[test]
    fn dest_expr_override_binde_la_valeur() {
        let dests = vec![(
            "nature".to_string(),
            Destination {
                mode: "override".into(),
                value: Some("2ELI".into()),
                via: None,
                attr: None,
                ref_field: None,
            },
        )];
        let mut params: Vec<DbValue> = Vec::new();
        let expr = dest_expr("nature", &dests, &mut params);
        assert_eq!(expr, "?", "mode override → placeholder");
        assert_eq!(params.len(), 1, "une valeur bindée");
        match &params[0] {
            DbValue::Text(s) => assert_eq!(s, "2ELI"),
            other => panic!("attendu Text(2ELI), eu {other:?}"),
        }
    }

    #[test]
    fn dest_expr_map_ref_pointe_vers_alias_mdr() {
        // map_ref : la valeur est lue depuis la colonne de référence, via l'alias
        // `mdr_<ref>` (master data de la dimension écrite, joinée à exec_operation).
        let dests = vec![(
            "account".to_string(),
            Destination {
                mode: "map_ref".into(),
                value: None,
                via: None,
                attr: None,
                ref_field: Some("compte_parent".into()),
            },
        )];
        let mut params: Vec<DbValue> = Vec::new();
        let expr = dest_expr("account", &dests, &mut params);
        assert_eq!(
            expr,
            "mdr_compte_parent.\"compte_parent\"",
            "mode map_ref → colonne via alias mdr_<ref>"
        );
        assert!(params.is_empty(), "map_ref ne bind rien");
    }

    #[test]
    fn json_to_dbvalue_convertit_types_primitifs() {
        assert!(matches!(json_to_dbvalue(&JsonValue::Null), DbValue::Null));
        assert!(matches!(
            json_to_dbvalue(&JsonValue::Bool(true)),
            DbValue::Boolean(true)
        ));
        assert!(matches!(
            json_to_dbvalue(&JsonValue::String("x".into())),
            DbValue::Text(_)
        ));
        // Entier → BigInt ; flottant → Double.
        assert!(matches!(
            json_to_dbvalue(&JsonValue::Number(serde_json::Number::from(42))),
            DbValue::BigInt(42)
        ));
        assert!(matches!(
            json_to_dbvalue(&serde_json::json!(3.14)),
            DbValue::Double(_)
        ));
    }

    // ── Sélection étendue (attr — enums natifs) ────────────────────────────

    #[test]
    fn parse_accepte_selection_avec_attr_enum() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","attr":"classe","op":"=","val":"bilan"}]
        }] }"#;
        let def = parse_definition(json, &ctx).expect("attr enum valide");
        let s = &def.operations[0].selection[0];
        assert_eq!(s.dim, "account");
        assert_eq!(s.attr.as_deref(), Some("classe"));
        assert!(s.via.is_none() && s.ref_field.is_none(), "attr exclusif");
    }

    #[test]
    fn parse_rejette_attr_et_via_simultanes() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","via":"x","attr":"classe","op":"=","val":"bilan"}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("exclusives"),
            "doit rejeter via+attr simultanés : {err}"
        );
    }

    #[test]
    fn parse_rejette_attr_et_ref_simultanes() {
        let ctx = ctx_fixture();
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","ref":"x","attr":"classe","op":"=","val":"bilan"}]
        }] }"#;
        let err = parse_definition(json, &ctx).unwrap_err();
        assert!(
            err.contains("exclusives"),
            "doit rejeter ref+attr simultanés : {err}"
        );
    }

    #[test]
    fn validate_rejette_attr_inconnu() {
        // `account.toto` n'est pas dans le catalogue NATIVE_ENUMS.
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","attr":"toto","op":"=","val":"x"}]
        }] }"#;
        let err = validate_definition(&con, json).unwrap_err();
        assert!(
            err.contains("enum natif inconnu") || err.contains("toto"),
            "doit rejeter un attr non catalogué : {err}"
        );
    }

    #[test]
    fn validate_rejette_attr_avec_valeur_invalide() {
        // `account.classe = 'inexistant'` — hors enum.
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","attr":"classe","op":"=","val":"inexistant"}]
        }] }"#;
        let err = validate_definition(&con, json).unwrap_err();
        assert!(
            err.contains("invalide") || err.contains("inexistant"),
            "doit rejeter une valeur hors enum : {err}"
        );
    }

    #[test]
    fn validate_accepte_attr_avec_valeurs_enum() {
        // `account.classe IN ('bilan', 'flux')` — toutes deux autorisées.
        let con = duckdb::Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        let json = r#"{ "operations":[{
            "seq":1,"level":"consolidated",
            "selection":[{"dim":"account","attr":"classe","op":"IN","val":["bilan","flux"]}]
        }] }"#;
        validate_definition(&con, json).expect("valeurs enum autorisées");
    }
}
