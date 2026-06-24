//! Bibliothèque de **coefficients** (volet 1 du moteur de formules).
//! Spec : `docs/FORMULES.md` §3.
//!
//! Un coefficient est une **formule nommée** (table `dim_coefficient`) évaluée au
//! grain d'une écriture de règle, dont les opérandes sont des valeurs de
//! `sat_perimeter` aux quatre perspectives (`entity` / `partner` / `entity_n1` /
//! `partner_n1`). [`resolve_expr`] compile une formule en `(expression SQL,
//! CoeffJoins)` — le couple consommé par `rules::exec_operation`, en
//! remplacement de l'ancien `coefficient_expr` codé en dur.
//!
//! Les **coefficients natifs** (`pct_integration`, `pct_interet`, élim. IC) sont
//! seedés comme formules (`kind = 'builtin'`) : l'ancienne enum devient des
//! données. Les coefficients **utilisateur** (`kind = 'user'`) survivent au reset
//! (registre hors `ALL_DROP`).

use crate::formula::{self, CoeffJoins, PerimeterResolver};
use crate::state::{db_err, lock_con, AppError, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use duckdb::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Perspectives disponibles : (suffixe de token, libellé d'affichage).
const PERSPECTIVES: &[(&str, &str)] = &[
    ("entity", "entité"),
    ("partner", "partenaire"),
    ("entity_n1", "entité N-1"),
    ("partner_n1", "partenaire N-1"),
];

/// Coefficients natifs, exprimés **comme formules** (code, libellé, expression).
///
/// Équivalents stricts des anciens `Coefficient::*` codés en dur : `pct_*` lisent
/// l'entité ; les `elim_ic_corp_*` éliminent au prorata du plus faible taux
/// d'intégration des deux entités liées (`MIN(1, PA/EN)`), le N-1 venant de
/// l'à-nouveau (cf. `docs/FORMULES.md` §3.3, [Q40]).
pub const BUILTINS: &[(&str, &str, &str)] = &[
    ("pct_integration", "% d'intégration (entité)", "[pct_integration.entity]"),
    ("pct_interet", "% d'intérêt (entité)", "[pct_interet.entity]"),
    (
        "elim_ic_corp_n",
        "Élimination IC — taux N",
        "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))",
    ),
    (
        "elim_ic_corp_n1",
        "Élimination IC — taux N-1",
        "MIN(1; SAFE_DIV([pct_integration.partner_n1]; [pct_integration.entity_n1]))",
    ),
    (
        "elim_ic_corp_var",
        "Élimination IC — variation N vs N-1",
        "MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity])) \
         - MIN(1; SAFE_DIV([pct_integration.partner_n1]; [pct_integration.entity_n1]))",
    ),
];

/// Migration idempotente au **démarrage** : garantit la présence de
/// `dim_coefficient` (table introduite après les premières bases) et (re)seede
/// les natifs. Permet au volet coefficients de fonctionner sur une base
/// **existante** sans reset (qui effacerait les éditions UI). Même esprit que
/// `custom_references::migrate_native`.
pub fn ensure_schema(con: &Connection) -> duckdb::Result<()> {
    con.execute(crate::schema::DDL_DIM_COEFFICIENT, [])?;
    seed_builtins(con)
}

/// Seede (idempotent) les coefficients natifs comme formules `kind='builtin'`.
/// Appelée par `create_schema` (la table survit au reset, mais les natifs sont
/// toujours (re)posés via `INSERT OR IGNORE`).
pub fn seed_builtins(con: &Connection) -> duckdb::Result<()> {
    for (code, libelle, expr) in BUILTINS {
        con.execute(
            "INSERT OR IGNORE INTO dim_coefficient (code, libelle, expression, kind) \
             VALUES (?, ?, ?, 'builtin')",
            params![code, libelle, expr],
        )?;
    }
    Ok(())
}

/// Colonnes **numériques** de `sat_perimeter` (whitelist des champs d'opérande).
/// Data-driven : tout champ numérique du périmètre devient disponible aux 4
/// perspectives. Repli sur les deux champs connus si information_schema est vide.
pub fn perimeter_fields(con: &Connection) -> Vec<String> {
    con.prepare(
        "SELECT column_name \
         FROM information_schema.columns \
         WHERE table_name = 'sat_perimeter' \
           AND (data_type LIKE 'DECIMAL%' \
                OR upper(data_type) IN \
                   ('DOUBLE','FLOAT','REAL','BIGINT','INTEGER','SMALLINT','HUGEINT','TINYINT')) \
         ORDER BY ordinal_position",
    )
    .and_then(|mut stmt| {
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<duckdb::Result<Vec<_>>>()
    })
    .unwrap_or_else(|_| vec!["pct_interet".into(), "pct_integration".into()])
}

/// Libellé lisible d'un champ de périmètre (pour le catalogue de l'éditeur).
fn field_label(field: &str) -> String {
    match field {
        "pct_integration" => "Intégration".to_string(),
        "pct_interet" => "Intérêt".to_string(),
        other => other.to_string(),
    }
}

/// Catalogue des opérandes disponibles : `(token, libellé)`.
/// `token` = `champ.perspective` (inséré dans la formule entre `[ ]`) ;
/// `libellé` = « Champ · perspective » (affiché dans le panneau de références).
pub fn operand_catalog(con: &Connection) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for field in perimeter_fields(con) {
        let flabel = field_label(&field);
        for (persp, plabel) in PERSPECTIVES {
            out.push((format!("{field}.{persp}"), format!("{flabel} · {plabel}")));
        }
    }
    out
}

/// Construit le résolveur d'opérandes périmètre depuis la base.
fn resolver(con: &Connection) -> PerimeterResolver {
    PerimeterResolver::new(perimeter_fields(con))
}

/// Compile l'expression brute d'un coefficient → `(SQL, CoeffJoins)`.
pub fn compile_expression(con: &Connection, expression: &str) -> Result<(String, CoeffJoins), String> {
    formula::compile(expression, &resolver(con))
}

/// Valide une expression de coefficient (parsing + résolution des opérandes).
/// Appelée à la création / modification d'un coefficient utilisateur.
pub fn validate_expression(con: &Connection, expression: &str) -> Result<(), String> {
    compile_expression(con, expression).map(|_| ())
}

/// Résout un coefficient **nommé** en `(SQL, CoeffJoins)` : lit son expression
/// dans `dim_coefficient` puis la compile. Erreur si le code est inconnu.
pub fn resolve_expr(con: &Connection, code: &str) -> Result<(String, CoeffJoins), String> {
    let expr: Option<String> = con
        .query_row(
            "SELECT expression FROM dim_coefficient WHERE code = ?",
            params![code],
            |r| r.get(0),
        )
        .ok();
    let expr = expr.ok_or_else(|| format!("coefficient inconnu : '{code}'"))?;
    compile_expression(con, &expr)
        .map_err(|e| format!("coefficient '{code}' : {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  API REST (volet 1 — bibliothèque de coefficients)
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `GET /api/coefficients`.
#[derive(Serialize)]
struct CoefOut {
    code: String,
    libelle: Option<String>,
    expression: String,
    kind: String,
}

/// Opérande du catalogue `GET /api/coefficients/operands`.
#[derive(Serialize)]
struct OperandOut {
    token: String,
    label: String,
}

/// Corps de `POST` / `PUT`.
#[derive(Deserialize)]
struct CoefBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    expression: String,
}

/// Corps de `POST /api/coefficients/preview`.
#[derive(Deserialize)]
struct PreviewBody {
    expression: String,
    /// Valeurs d'exemple des opérandes (token → valeur). Absentes → 0.
    #[serde(default)]
    samples: HashMap<String, f64>,
}

/// Réponse de la preview live.
#[derive(Serialize)]
struct PreviewOut {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sql: Option<String>,
    /// Opérandes référencés (pour pré-remplir les champs d'exemple de l'éditeur).
    operands: Vec<String>,
}

/// GET /api/coefficients — liste la bibliothèque (natifs + utilisateur).
async fn list(State(state): State<Arc<AppState>>) -> Result<Json<Vec<CoefOut>>, AppError> {
    let con = lock_con(&state)?;
    let mut stmt = con
        .prepare(
            "SELECT code, libelle, expression, kind FROM dim_coefficient \
             ORDER BY kind DESC, code",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CoefOut {
                code: r.get(0)?,
                libelle: r.get(1)?,
                expression: r.get(2)?,
                kind: r.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(db_err)?;
    Ok(Json(rows))
}

/// GET /api/coefficients/operands — catalogue des opérandes de périmètre.
async fn operands_catalog(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<OperandOut>>, AppError> {
    let con = lock_con(&state)?;
    let out = operand_catalog(&con)
        .into_iter()
        .map(|(token, label)| OperandOut { token, label })
        .collect();
    Ok(Json(out))
}

/// POST /api/coefficients/preview — valide + évalue une formule (sans la sauver).
async fn preview(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PreviewBody>,
) -> Result<Json<PreviewOut>, AppError> {
    let con = lock_con(&state)?;
    let operands = formula::operands(&body.expression).unwrap_or_default();
    // Compilation = validation (parsing + résolution des opérandes).
    match compile_expression(&con, &body.expression) {
        Err(e) => Ok(Json(PreviewOut {
            ok: false,
            value: None,
            error: Some(e),
            sql: None,
            operands,
        })),
        Ok((sql, _)) => match formula::evaluate(&body.expression, &body.samples) {
            Ok(v) => Ok(Json(PreviewOut {
                ok: true,
                value: Some(v),
                error: None,
                sql: Some(sql),
                operands,
            })),
            Err(e) => Ok(Json(PreviewOut {
                ok: false,
                value: None,
                error: Some(e),
                sql: Some(sql),
                operands,
            })),
        },
    }
}

/// POST /api/coefficients — crée un coefficient **utilisateur**.
async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CoefBody>,
) -> Result<(StatusCode, Json<CoefOut>), AppError> {
    let con = lock_con(&state)?;
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_coefficient WHERE code = ?",
            params![body.code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!(
            "coefficient {} existe déjà",
            body.code
        )));
    }
    validate_expression(&con, &body.expression).map_err(AppError::bad_request)?;
    con.execute(
        "INSERT INTO dim_coefficient (code, libelle, expression, kind) VALUES (?, ?, ?, 'user')",
        params![body.code, body.libelle, body.expression],
    )
    .map_err(db_err)?;
    Ok((
        StatusCode::CREATED,
        Json(CoefOut {
            code: body.code,
            libelle: body.libelle,
            expression: body.expression,
            kind: "user".to_string(),
        }),
    ))
}

/// Vérifie qu'un coefficient existe et n'est pas natif (édition/suppression).
fn ensure_user(con: &Connection, code: &str) -> Result<(), AppError> {
    let kind: Option<String> = con
        .query_row(
            "SELECT kind FROM dim_coefficient WHERE code = ?",
            params![code],
            |r| r.get(0),
        )
        .ok();
    match kind.as_deref() {
        None => Err(AppError::not_found(format!("coefficient {code} introuvable"))),
        Some("builtin") => Err(AppError::bad_request(format!(
            "coefficient natif '{code}' : non modifiable (créez-en une copie)"
        ))),
        _ => Ok(()),
    }
}

/// PUT /api/coefficients/{code} — modifie un coefficient utilisateur (en place,
/// décision F4). Les natifs sont verrouillés.
async fn update(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<CoefBody>,
) -> Result<Json<CoefOut>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request(
            "le `code` du body ne correspond pas à l'URL",
        ));
    }
    let con = lock_con(&state)?;
    ensure_user(&con, &code)?;
    validate_expression(&con, &body.expression).map_err(AppError::bad_request)?;
    con.execute(
        "UPDATE dim_coefficient SET libelle = ?, expression = ? WHERE code = ?",
        params![body.libelle, body.expression, code],
    )
    .map_err(db_err)?;
    Ok(Json(CoefOut {
        code,
        libelle: body.libelle,
        expression: body.expression,
        kind: "user".to_string(),
    }))
}

/// DELETE /api/coefficients/{code} — supprime un coefficient utilisateur.
async fn delete_coef(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let con = lock_con(&state)?;
    ensure_user(&con, &code)?;
    con.execute("DELETE FROM dim_coefficient WHERE code = ?", params![code])
        .map_err(db_err)?;
    Ok(Json(serde_json::json!({ "status": "ok", "deleted": code })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/coefficients", get(list).post(create))
        .route("/api/coefficients/operands", get(operands_catalog))
        .route("/api/coefficients/preview", post(preview))
        .route(
            "/api/coefficients/{code}",
            axum::routing::put(update).delete(delete_coef),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let con = Connection::open_in_memory().unwrap();
        crate::schema::create_schema(&con).unwrap();
        con
    }

    #[test]
    fn seed_builtins_idempotent() {
        let con = db();
        // create_schema seede déjà ; un second appel ne doit pas dupliquer.
        seed_builtins(&con).unwrap();
        let n: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM dim_coefficient WHERE kind = 'builtin'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n as usize, BUILTINS.len());
    }

    #[test]
    fn resolve_pct_integration() {
        let con = db();
        let (sql, j) = resolve_expr(&con, "pct_integration").unwrap();
        assert_eq!(sql, "COALESCE(p_ent.pct_integration, 0)");
        assert!(j.p_ent && !j.p_part);
    }

    #[test]
    fn resolve_elim_ic_corp_var_joins_n1() {
        let con = db();
        let (_, j) = resolve_expr(&con, "elim_ic_corp_var").unwrap();
        assert!(j.p_ent && j.p_part && j.p_ent_n1 && j.p_part_n1);
    }

    #[test]
    fn resolve_inconnu_erreur() {
        let con = db();
        assert!(resolve_expr(&con, "nexiste_pas").is_err());
    }

    #[test]
    fn catalogue_operandes_couvre_4_perspectives() {
        let con = db();
        let cat = operand_catalog(&con);
        // pct_integration + pct_interet × 4 perspectives = 8 opérandes.
        assert_eq!(cat.len(), 8);
        assert!(cat.iter().any(|(t, _)| t == "pct_integration.partner_n1"));
    }

    #[test]
    fn validate_expression_rejette_operande_inconnu() {
        let con = db();
        assert!(validate_expression(&con, "[methode.entity]").is_err());
        assert!(validate_expression(&con, "[pct_integration.entity] + 1").is_ok());
    }

    #[test]
    fn coefficient_utilisateur_resolvable() {
        let con = db();
        con.execute(
            "INSERT INTO dim_coefficient (code, libelle, expression, kind) VALUES (?,?,?,'user')",
            params!["minoritaire", "Quote-part minoritaire", "1 - [pct_interet.entity]"],
        )
        .unwrap();
        let (sql, j) = resolve_expr(&con, "minoritaire").unwrap();
        assert!(sql.contains("p_ent.pct_interet"));
        assert!(j.p_ent);
    }
}
