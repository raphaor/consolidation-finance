//! Caractéristiques de regroupement (**N1**) et leurs attributs typés (**N2**).
//!
//! Une **caractéristique N1** classe les membres d'une dimension de base (ex.
//! `comportement` sur les comptes). Chaque valeur N1 (ligne de `car_<code>`)
//! porte des **attributs N2**, chacun étant une référence vers une dimension
//! (`compte_destination → dim_account`, `nature → dim_nature`…). Une règle
//! pourra (incrément ultérieur) router une écriture en traversant ces attributs.
//!
//! # Modèle physique
//!
//! Cohérent avec les dimensions custom de [`crate::dimensions`] :
//! - registres `dim_characteristic` (N1) et `dim_characteristic_attribute` (N2),
//!   qui **survivent au reset** (CREATE IF NOT EXISTS, hors `ALL_DROP`) ;
//! - une table de valeurs `car_<code>` par N1 (PK `code` + une colonne par N2) ;
//! - une colonne `<code>` sur la master data de la dimension de base, qui
//!   référence `car_<code>.code`.
//!
//! # Sécurité
//!
//! Les identifiants (`code`, `name`) sont validés (alphanumérique + underscore)
//! avant toute interpolation dans le DDL ; les noms de tables/colonnes cibles
//! proviennent du registre [`crate::references`] (jamais de l'entrée utilisateur).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::dimensions::is_valid_custom_name;
use crate::references;
use crate::state::{db_err, lock_con, AppError, AppState};

/// Nom de la table de valeurs d'une caractéristique N1.
fn value_table(code: &str) -> String {
    format!("car_{code}")
}

// ───────────────────────────── Modèle / chargement ─────────────────────────────

/// Un attribut N2 (colonne typée sur la table de valeurs d'une N1).
#[derive(Serialize)]
pub struct AttributeDef {
    pub name: String,
    pub libelle: String,
    pub target_dimension: String,
}

/// Une caractéristique N1 avec ses attributs N2.
#[derive(Serialize)]
pub struct CharacteristicDef {
    pub code: String,
    pub libelle: String,
    pub base_dimension: String,
    pub value_table: String,
    pub attributes: Vec<AttributeDef>,
}

/// `true` si les deux registres existent (faux au tout premier démarrage avant DDL).
fn registries_exist(con: &duckdb::Connection) -> bool {
    con.query_row(
        "SELECT COUNT(*) = 2 FROM information_schema.tables \
         WHERE table_schema = 'main' \
           AND table_name IN ('dim_characteristic', 'dim_characteristic_attribute')",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Charge toutes les caractéristiques N1 avec leurs attributs N2.
pub fn load_all(con: &duckdb::Connection) -> duckdb::Result<Vec<CharacteristicDef>> {
    if !registries_exist(con) {
        return Ok(Vec::new());
    }
    let chars: Vec<(String, String, String)> = {
        let mut stmt =
            con.prepare("SELECT code, libelle, base_dimension FROM dim_characteristic ORDER BY code")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<duckdb::Result<_>>()?
    };
    let mut out = Vec::with_capacity(chars.len());
    for (code, libelle, base_dimension) in chars {
        let attributes = load_attributes(con, &code)?;
        out.push(CharacteristicDef {
            value_table: value_table(&code),
            code,
            libelle,
            base_dimension,
            attributes,
        });
    }
    Ok(out)
}

fn load_attributes(
    con: &duckdb::Connection,
    char_code: &str,
) -> duckdb::Result<Vec<AttributeDef>> {
    let mut stmt = con.prepare(
        "SELECT name, libelle, target_dimension FROM dim_characteristic_attribute \
         WHERE characteristic_code = ? ORDER BY name",
    )?;
    let rows = stmt.query_map([char_code], |row| {
        Ok(AttributeDef {
            name: row.get::<_, String>(0)?,
            libelle: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            target_dimension: row.get::<_, String>(2)?,
        })
    })?;
    rows.collect()
}

/// Ré-applique, après un reset, la colonne `<code>` sur la master data de chaque
/// dimension de base (perdue lors du DROP des tables de dimension). Les tables
/// de valeurs `car_<code>` survivent au reset (hors `ALL_DROP`), donc ne sont
/// **pas** recréées ici. Idempotent : l'`ALTER ... ADD COLUMN` est silencieux si
/// la colonne existe déjà.
pub fn reapply(con: &duckdb::Connection) -> duckdb::Result<()> {
    if !registries_exist(con) {
        return Ok(());
    }
    for c in load_all(con)? {
        if let Some((base_table, _)) = references::dimension_master(&c.base_dimension) {
            let _ = con.execute(
                &format!("ALTER TABLE {base_table} ADD COLUMN {} TEXT", c.code),
                [],
            );
        }
    }
    Ok(())
}

// ───────────────────────────── Mutations (DDL dynamique) ────────────────────────

fn ensure_valid_ident(kind: &str, name: &str) -> Result<(), AppError> {
    if !is_valid_custom_name(name) {
        return Err(AppError::bad_request(format!(
            "{kind} invalide : {name:?} (alphanumérique + underscore, 1-50 caractères, \
             premier caractère lettre ou underscore, réservés : level/amount/id)"
        )));
    }
    Ok(())
}

/// Crée une caractéristique N1 : registre + table de valeurs `car_<code>` +
/// colonne de rattachement sur la master data de la dimension de base.
pub fn create_characteristic(
    con: &duckdb::Connection,
    code: &str,
    libelle: &str,
    base_dimension: &str,
) -> Result<(), AppError> {
    ensure_valid_ident("code de caractéristique", code)?;
    let (base_table, _) = references::dimension_master(base_dimension).ok_or_else(|| {
        AppError::bad_request(format!(
            "dimension de base inconnue ou sans master data : {base_dimension}"
        ))
    })?;
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!(
            "caractéristique déjà existante : {code}"
        )));
    }
    let vtable = value_table(code);
    con.execute(
        &format!("CREATE TABLE {vtable} (code TEXT PRIMARY KEY, libelle TEXT)"),
        [],
    )
    .map_err(db_err)?;
    con.execute(
        &format!("ALTER TABLE {base_table} ADD COLUMN {code} TEXT"),
        [],
    )
    .map_err(db_err)?;
    con.execute(
        "INSERT INTO dim_characteristic (code, libelle, base_dimension) VALUES (?, ?, ?)",
        &[&code, &libelle, &base_dimension],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une caractéristique N1 : table de valeurs + colonne de rattachement
/// + attributs N2 + registre.
pub fn delete_characteristic(con: &duckdb::Connection, code: &str) -> Result<(), AppError> {
    ensure_valid_ident("code de caractéristique", code)?;
    let base_dimension: String = con
        .query_row(
            "SELECT base_dimension FROM dim_characteristic WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .map_err(|_| AppError::not_found(format!("caractéristique inexistante : {code}")))?;
    let vtable = value_table(code);
    con.execute(&format!("DROP TABLE IF EXISTS {vtable}"), [])
        .map_err(db_err)?;
    if let Some((base_table, _)) = references::dimension_master(&base_dimension) {
        // Silencieux si la colonne a déjà disparu (ex. après un reset partiel).
        let _ = con.execute(
            &format!("ALTER TABLE {base_table} DROP COLUMN {code}"),
            [],
        );
    }
    con.execute(
        "DELETE FROM dim_characteristic_attribute WHERE characteristic_code = ?",
        [code],
    )
    .map_err(db_err)?;
    con.execute("DELETE FROM dim_characteristic WHERE code = ?", [code])
        .map_err(db_err)?;
    Ok(())
}

/// Ajoute un attribut N2 : colonne sur `car_<char>` + registre. La dimension
/// cible doit avoir une master data (déclarée dans [`crate::references`]).
pub fn add_attribute(
    con: &duckdb::Connection,
    char_code: &str,
    name: &str,
    libelle: &str,
    target_dimension: &str,
) -> Result<(), AppError> {
    ensure_valid_ident("nom d'attribut", name)?;
    let char_exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic WHERE code = ?",
            [char_code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !char_exists {
        return Err(AppError::not_found(format!(
            "caractéristique inexistante : {char_code}"
        )));
    }
    if references::dimension_master(target_dimension).is_none() {
        return Err(AppError::bad_request(format!(
            "dimension cible inconnue ou sans master data : {target_dimension}"
        )));
    }
    let attr_exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic_attribute \
             WHERE characteristic_code = ? AND name = ?",
            [char_code, name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if attr_exists {
        return Err(AppError::conflict(format!("attribut déjà existant : {name}")));
    }
    let vtable = value_table(char_code);
    con.execute(&format!("ALTER TABLE {vtable} ADD COLUMN {name} TEXT"), [])
        .map_err(db_err)?;
    con.execute(
        "INSERT INTO dim_characteristic_attribute \
         (characteristic_code, name, libelle, target_dimension) VALUES (?, ?, ?, ?)",
        &[&char_code, &name, &libelle, &target_dimension],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime un attribut N2 : colonne sur `car_<char>` + registre.
pub fn delete_attribute(
    con: &duckdb::Connection,
    char_code: &str,
    name: &str,
) -> Result<(), AppError> {
    ensure_valid_ident("nom d'attribut", name)?;
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM dim_characteristic_attribute \
             WHERE characteristic_code = ? AND name = ?",
            [char_code, name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!(
            "attribut inexistant : {char_code}.{name}"
        )));
    }
    let vtable = value_table(char_code);
    con.execute(&format!("ALTER TABLE {vtable} DROP COLUMN {name}"), [])
        .map_err(db_err)?;
    con.execute(
        "DELETE FROM dim_characteristic_attribute WHERE characteristic_code = ? AND name = ?",
        [char_code, name],
    )
    .map_err(db_err)?;
    Ok(())
}

// ───────────────────────────────── HTTP ─────────────────────────────────────────

#[derive(Deserialize)]
struct CreateCharacteristicBody {
    code: String,
    #[serde(default)]
    libelle: String,
    base_dimension: String,
}

#[derive(Deserialize)]
struct AddAttributeBody {
    name: String,
    #[serde(default)]
    libelle: String,
    target_dimension: String,
}

/// GET /api/meta/characteristics — liste les N1 avec leurs N2.
async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<CharacteristicDef>>, AppError> {
    let con = lock_con(&state)?;
    Ok(Json(load_all(&con).map_err(db_err)?))
}

/// POST /api/meta/characteristics — crée une caractéristique N1.
async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCharacteristicBody>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let con = lock_con(&state)?;
    create_characteristic(&con, &body.code, &body.libelle, &body.base_dimension)?;
    Ok((StatusCode::CREATED, Json(json!({ "code": body.code }))))
}

/// DELETE /api/meta/characteristics/{code} — supprime une N1 (et ses N2).
async fn remove(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    delete_characteristic(&con, &code)?;
    Ok(Json(json!({ "deleted": code })))
}

/// POST /api/meta/characteristics/{code}/attributes — ajoute un attribut N2.
async fn add_attr(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<AddAttributeBody>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let con = lock_con(&state)?;
    add_attribute(&con, &code, &body.name, &body.libelle, &body.target_dimension)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "characteristic": code, "name": body.name })),
    ))
}

/// DELETE /api/meta/characteristics/{code}/attributes/{name} — supprime un N2.
async fn remove_attr(
    State(state): State<Arc<AppState>>,
    Path((code, name)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    delete_attribute(&con, &code, &name)?;
    Ok(Json(json!({ "deleted": name })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/meta/characteristics", get(list).post(create))
        .route("/api/meta/characteristics/{code}", delete(remove))
        .route(
            "/api/meta/characteristics/{code}/attributes",
            post(add_attr),
        )
        .route(
            "/api/meta/characteristics/{code}/attributes/{name}",
            delete(remove_attr),
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

    fn table_exists(con: &Connection, table: &str) -> bool {
        con.query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables WHERE table_name = ?",
            [table],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn cree_n1_et_n2_avec_artefacts_physiques() {
        let con = setup();
        create_characteristic(&con, "comportement", "Comportement", "account").unwrap();
        assert!(table_exists(&con, "car_comportement"), "table de valeurs créée");
        assert!(
            col_exists(&con, "dim_account", "comportement"),
            "colonne de rattachement sur dim_account"
        );

        add_attribute(
            &con,
            "comportement",
            "compte_destination",
            "Compte de destination",
            "account",
        )
        .unwrap();
        assert!(
            col_exists(&con, "car_comportement", "compte_destination"),
            "colonne d'attribut N2 sur car_comportement"
        );

        let all = load_all(&con).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].base_dimension, "account");
        assert_eq!(all[0].attributes.len(), 1);
        assert_eq!(all[0].attributes[0].target_dimension, "account");
    }

    #[test]
    fn refs_dynamiques_exposees() {
        let con = setup();
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        add_attribute(&con, "comportement", "compte_destination", "L", "account").unwrap();
        add_attribute(&con, "comportement", "nat", "Nature d'élim", "nature").unwrap();

        let refs = references::dynamic_references(&con);
        // N1 : dim_account.comportement → car_comportement.code
        assert!(refs.iter().any(|r| r.table == "dim_account"
            && r.column == "comportement"
            && r.target_table == "car_comportement"
            && r.target_column == "code"));
        // N2 : car_comportement.compte_destination → dim_account.code
        assert!(refs.iter().any(|r| r.table == "car_comportement"
            && r.column == "compte_destination"
            && r.target_table == "dim_account"
            && r.target_column == "code"));
        // N2 typée vers une autre dimension : nat → dim_nature.code
        assert!(refs.iter().any(|r| r.table == "car_comportement"
            && r.column == "nat"
            && r.target_table == "dim_nature"
            && r.target_column == "code"));
    }

    #[test]
    fn refuse_dimension_sans_master_data() {
        let con = setup();
        assert!(
            create_characteristic(&con, "x", "X", "analysis2").is_err(),
            "analysis2 n'a pas de master data"
        );
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        assert!(
            add_attribute(&con, "comportement", "y", "Y", "analysis").is_err(),
            "analysis n'a pas de master data comme cible N2"
        );
    }

    #[test]
    fn survit_au_reset() {
        let con = setup();
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        add_attribute(&con, "comportement", "compte_destination", "L", "account").unwrap();

        // Reset complet du schéma.
        crate::schema::create_schema(&con).expect("re-create_schema");

        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_characteristic", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "registre N1 survit au reset");
        assert!(
            col_exists(&con, "dim_account", "comportement"),
            "colonne de rattachement réappliquée après reset"
        );
        assert!(
            table_exists(&con, "car_comportement"),
            "table de valeurs survit au reset"
        );
        assert!(
            col_exists(&con, "car_comportement", "compte_destination"),
            "colonne d'attribut survit au reset"
        );
    }

    #[test]
    fn suppression_nettoie_artefacts() {
        let con = setup();
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        add_attribute(&con, "comportement", "compte_destination", "L", "account").unwrap();

        delete_characteristic(&con, "comportement").unwrap();
        assert!(!table_exists(&con, "car_comportement"), "table supprimée");
        assert!(
            !col_exists(&con, "dim_account", "comportement"),
            "colonne de rattachement retirée"
        );
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_characteristic_attribute", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0, "attributs N2 supprimés avec la N1");
    }
}
