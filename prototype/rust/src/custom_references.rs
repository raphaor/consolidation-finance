//! Références directes (**patron B**) : une colonne ajoutée à l'exécution sur la
//! master data d'une dimension, déclarée comme **référence** vers une dimension
//! existante — y compris elle-même (hiérarchie).
//!
//! C'est la généralisation, pilotable par l'utilisateur, du patron jusqu'ici codé
//! en dur pour `dim_account.compte_parent` (→ `dim_account.code`) et
//! `dim_entity.entite_parent` (→ `dim_entity.code`). Contrairement à une
//! [`crate::characteristics`] (regroupement N1 avec table de valeurs `car_<code>`
//! et attributs N2), une référence directe **n'introduit aucune table
//! intermédiaire** : la colonne pointe directement vers la master data cible.
//!
//! # Modèle physique
//!
//! - registre `dim_custom_reference` (host_dimension, column_name,
//!   target_dimension) qui **survit au reset** (CREATE IF NOT EXISTS, hors
//!   `ALL_DROP`) ;
//! - une colonne `<column_name>` sur la master data de la dimension hôte, perdue
//!   au DROP des dimensions et ré-appliquée par [`reapply`] ;
//! - le lien est exposé au reste du moteur par [`crate::references::dynamic_references`]
//!   (validation à l'écriture, santé des données, dropdowns).
//!
//! # Sécurité
//!
//! Le nom de colonne est validé (alphanumérique + underscore) avant toute
//! interpolation dans le DDL ; les noms de tables/colonnes cibles proviennent du
//! registre [`crate::references`] (jamais de l'entrée utilisateur) ; les valeurs
//! passent par des `?` paramétrés.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, put},
    Json, Router,
};
use duckdb::{params_from_iter, types::Value as DbValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::dimensions::is_valid_custom_name;
use crate::references;
use crate::state::{db_err, lock_con, AppError, AppState};

// ───────────────────────────── Modèle / chargement ─────────────────────────────

/// Une référence directe : `dim_<host>.<column> → dim_<target>.<clé>`.
#[derive(Serialize)]
pub struct CustomReferenceDef {
    pub host_dimension: String,
    pub column: String,
    pub target_dimension: String,
}

/// `true` si le registre existe (faux au tout premier démarrage avant DDL).
fn registry_exists(con: &duckdb::Connection) -> bool {
    con.query_row(
        "SELECT COUNT(*) = 1 FROM information_schema.tables \
         WHERE table_schema = 'main' AND table_name = 'dim_custom_reference'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Charge toutes les références directes déclarées.
pub fn load_all(con: &duckdb::Connection) -> duckdb::Result<Vec<CustomReferenceDef>> {
    if !registry_exists(con) {
        return Ok(Vec::new());
    }
    let mut stmt = con.prepare(
        "SELECT host_dimension, column_name, target_dimension \
         FROM dim_custom_reference ORDER BY host_dimension, column_name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(CustomReferenceDef {
            host_dimension: row.get::<_, String>(0)?,
            column: row.get::<_, String>(1)?,
            target_dimension: row.get::<_, String>(2)?,
        })
    })?;
    rows.collect()
}

/// Ré-applique, après un reset, la colonne `<column_name>` sur la master data de
/// chaque dimension hôte (perdue lors du DROP des tables de dimension). Le
/// registre, lui, survit au reset (hors `ALL_DROP`). Idempotent : l'`ALTER ...
/// ADD COLUMN` est silencieux si la colonne existe déjà.
pub fn reapply(con: &duckdb::Connection) -> duckdb::Result<()> {
    if !registry_exists(con) {
        return Ok(());
    }
    for r in load_all(con)? {
        if let Some((host_table, _)) = references::dimension_master(&r.host_dimension) {
            let _ = con.execute(
                &format!("ALTER TABLE {host_table} ADD COLUMN {} TEXT", r.column),
                [],
            );
        }
    }
    Ok(())
}

// ───────────────────────────── Mutations (DDL dynamique) ────────────────────────

fn ensure_valid_column(name: &str) -> Result<(), AppError> {
    if !is_valid_custom_name(name) {
        return Err(AppError::bad_request(format!(
            "nom de colonne invalide : {name:?} (alphanumérique + underscore, 1-50 caractères, \
             premier caractère lettre ou underscore, réservés : level/amount/id)"
        )));
    }
    Ok(())
}

/// Crée une référence directe : registre + colonne sur la master data de la
/// dimension hôte. `host` et `target` doivent avoir une master data ; `target`
/// peut être égal à `host` (hiérarchie auto-référentielle).
pub fn create(
    con: &duckdb::Connection,
    host: &str,
    column: &str,
    target: &str,
) -> Result<(), AppError> {
    ensure_valid_column(column)?;
    let (host_table, _) = references::dimension_master(host).ok_or_else(|| {
        AppError::bad_request(format!(
            "dimension hôte inconnue ou sans master data : {host}"
        ))
    })?;
    if references::dimension_master(target).is_none() {
        return Err(AppError::bad_request(format!(
            "dimension cible inconnue ou sans master data : {target}"
        )));
    }
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [host, column],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!(
            "référence déjà existante : {host}.{column}"
        )));
    }
    con.execute(
        &format!("ALTER TABLE {host_table} ADD COLUMN {column} TEXT"),
        [],
    )
    .map_err(db_err)?;
    con.execute(
        "INSERT INTO dim_custom_reference (host_dimension, column_name, target_dimension) \
         VALUES (?, ?, ?)",
        &[&host, &column, &target],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une référence directe : colonne sur la master data hôte + registre.
pub fn remove(con: &duckdb::Connection, host: &str, column: &str) -> Result<(), AppError> {
    ensure_valid_column(column)?;
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [host, column],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!(
            "référence inexistante : {host}.{column}"
        )));
    }
    if let Some((host_table, _)) = references::dimension_master(host) {
        // Silencieux si la colonne a déjà disparu (ex. après un reset partiel).
        let _ = con.execute(
            &format!("ALTER TABLE {host_table} DROP COLUMN {column}"),
            [],
        );
    }
    con.execute(
        "DELETE FROM dim_custom_reference WHERE host_dimension = ? AND column_name = ?",
        [host, column],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Affecte (ou retire, si `value` est `None`/vide) une valeur de référence à un
/// membre de la dimension hôte (ex. donner le parent `60` au compte `600`).
pub fn assign(
    con: &duckdb::Connection,
    host: &str,
    column: &str,
    member: &str,
    value: Option<&str>,
) -> Result<(), AppError> {
    let target: String = con
        .query_row(
            "SELECT target_dimension FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [host, column],
            |r| r.get(0),
        )
        .map_err(|_| AppError::not_found(format!("référence inexistante : {host}.{column}")))?;
    let (host_table, host_key) = references::dimension_master(host).ok_or_else(|| {
        AppError::bad_request(format!("dimension hôte sans master data : {host}"))
    })?;
    if !references::value_exists(con, host_table, host_key, member).map_err(db_err)? {
        return Err(AppError::not_found(format!(
            "membre inexistant : {host_table}.{host_key} = {member}"
        )));
    }
    let dbval = match value {
        Some(v) if !v.is_empty() => {
            let (tt, tc) = references::dimension_master(&target).ok_or_else(|| {
                AppError::bad_request(format!("dimension cible sans master data : {target}"))
            })?;
            if !references::value_exists(con, tt, tc, v).map_err(db_err)? {
                return Err(AppError::bad_request(format!(
                    "valeur cible inexistante : {tt}.{tc} = {v}"
                )));
            }
            DbValue::Text(v.to_string())
        }
        _ => DbValue::Null, // désaffectation
    };
    let sql = format!("UPDATE {host_table} SET \"{column}\" = ? WHERE \"{host_key}\" = ?");
    con.execute(
        &sql,
        params_from_iter(vec![dbval, DbValue::Text(member.to_string())]),
    )
    .map_err(db_err)?;
    Ok(())
}

// ───────────────────────────────── HTTP ─────────────────────────────────────────

#[derive(Deserialize)]
struct CreateBody {
    host_dimension: String,
    column: String,
    target_dimension: String,
}

#[derive(Deserialize)]
struct AssignBody {
    member: String,
    #[serde(default)]
    value: Option<String>,
}

/// GET /api/meta/references-custom — liste les références directes.
async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<CustomReferenceDef>>, AppError> {
    let con = lock_con(&state)?;
    Ok(Json(load_all(&con).map_err(db_err)?))
}

/// POST /api/meta/references-custom — crée une référence directe.
async fn create_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let con = lock_con(&state)?;
    create(
        &con,
        &body.host_dimension,
        &body.column,
        &body.target_dimension,
    )?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "host_dimension": body.host_dimension, "column": body.column })),
    ))
}

/// DELETE /api/meta/references-custom/{host}/{column} — supprime une référence.
async fn remove_handler(
    State(state): State<Arc<AppState>>,
    Path((host, column)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    remove(&con, &host, &column)?;
    Ok(Json(json!({ "deleted": format!("{host}.{column}") })))
}

/// PUT /api/meta/references-custom/{host}/{column}/assign — affecte (ou retire)
/// une valeur de référence à un membre.
async fn assign_handler(
    State(state): State<Arc<AppState>>,
    Path((host, column)): Path<(String, String)>,
    Json(body): Json<AssignBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    assign(&con, &host, &column, &body.member, body.value.as_deref())?;
    Ok(Json(json!({ "member": body.member, "value": body.value })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/meta/references-custom",
            get(list).post(create_handler),
        )
        .route(
            "/api/meta/references-custom/{host}/{column}",
            delete(remove_handler),
        )
        .route(
            "/api/meta/references-custom/{host}/{column}/assign",
            put(assign_handler),
        )
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

    fn col_exists(con: &Connection, table: &str, col: &str) -> bool {
        con.query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.columns \
             WHERE table_name = ? AND column_name = ?",
            [table, col],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn cree_reference_avec_colonne() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();
        assert!(
            col_exists(&con, "dim_account", "compte_parent"),
            "colonne ajoutée sur dim_account"
        );
        let all = load_all(&con).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].host_dimension, "account");
        assert_eq!(all[0].target_dimension, "account");
    }

    #[test]
    fn exposee_dans_le_graphe_de_references() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();
        let refs = references::dynamic_references(&con);
        assert!(
            refs.iter().any(|r| r.table == "dim_account"
                && r.column == "compte_parent"
                && r.target_table == "dim_account"
                && r.target_column == "code"),
            "la référence directe apparaît dans dynamic_references"
        );
    }

    #[test]
    fn refuse_dimension_sans_master_data() {
        let con = setup();
        assert!(
            create(&con, "analysis", "x", "account").is_err(),
            "analysis n'a pas de master data comme hôte"
        );
        assert!(
            create(&con, "account", "x", "analysis").is_err(),
            "analysis n'a pas de master data comme cible"
        );
    }

    #[test]
    fn assign_self_reference_et_validation() {
        let con = setup();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('60', 'Achats', 'resultat')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('600', 'Achats stockés', 'resultat')",
            [],
        )
        .unwrap();
        create(&con, "account", "compte_parent", "account").unwrap();

        // Affecte le parent 60 au compte 600.
        assign(&con, "account", "compte_parent", "600", Some("60")).unwrap();
        let parent: Option<String> = con
            .query_row(
                "SELECT compte_parent FROM dim_account WHERE code = '600'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent.as_deref(), Some("60"));

        // Parent inexistant → rejeté.
        assert!(assign(&con, "account", "compte_parent", "600", Some("NOPE")).is_err());
        // Membre inexistant → rejeté.
        assert!(assign(&con, "account", "compte_parent", "999", Some("60")).is_err());

        // Désaffectation.
        assign(&con, "account", "compte_parent", "600", None).unwrap();
        let after: Option<String> = con
            .query_row(
                "SELECT compte_parent FROM dim_account WHERE code = '600'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, None);
    }

    #[test]
    fn survit_au_reset() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();

        // Reset complet du schéma.
        crate::schema::create_schema(&con).expect("re-create_schema");

        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "registre survit au reset");
        assert!(
            col_exists(&con, "dim_account", "compte_parent"),
            "colonne réappliquée après reset"
        );
    }

    #[test]
    fn suppression_nettoie() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();
        remove(&con, "account", "compte_parent").unwrap();
        assert!(
            !col_exists(&con, "dim_account", "compte_parent"),
            "colonne retirée"
        );
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
    }
}
