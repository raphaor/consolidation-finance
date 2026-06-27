//! Listes de valeurs (**référentiels**) : des nomenclatures `code/libellé`
//! autonomes, éditables et **réutilisables**, mais qui ne sont **pas des
//! dimensions** — elles n'apparaissent jamais sur `fact_entry`/`stg_entry`,
//! n'entrent pas dans `dim_custom_dimension`, et ne sont donc pas un axe
//! d'écriture.
//!
//! C'est la brique qui manquait pour qu'un **champ** de caractéristique (attribut
//! N2) puisse tirer ses valeurs d'une liste simple plutôt que d'une dimension
//! existante : un attribut N2 dont la cible est une liste référence `lst_<code>`.
//!
//! # Modèle physique
//!
//! Cohérent avec les autres registres pilotables ([`crate::characteristics`],
//! [`crate::custom_references`]) :
//! - un registre `dim_value_list` (code, libellé) qui **survit au reset**
//!   (CREATE IF NOT EXISTS, hors `ALL_DROP`) ;
//! - une table de valeurs `lst_<code>(code, libelle)` par liste, qui survit au
//!   reset comme les `car_<code>` (jamais dans `ALL_DROP`, jamais recréée).
//!
//! Contrairement à une caractéristique, une liste **ne pose aucune colonne** sur
//! une dimension : rien à ré-appliquer après un reset (pas de `reapply`).
//!
//! # Sécurité
//!
//! Le `code` est validé (alphanumérique + underscore) avant toute interpolation
//! dans le DDL ; les valeurs passent par des `?` paramétrés.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, put},
    Json, Router,
};
use duckdb::types::Value as DbValue;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};

use crate::dimensions::is_valid_custom_name;
use crate::masterdata;
use crate::state::{db_err, lock_con, AppError, AppState};

/// Nom physique de la table de valeurs d'une liste (B1 étape 5 : par `id`).
pub fn value_table(id: i64) -> String {
    format!("lst_{id}")
}

/// `id` technique d'une liste de valeurs, ou `None` si elle n'existe pas.
pub fn id_of(con: &duckdb::Connection, code: &str) -> Option<i64> {
    con.query_row(
        "SELECT id FROM dim_value_list WHERE code = ?",
        [code],
        |r| r.get(0),
    )
    .ok()
}

/// Nom physique de la table de valeurs pour un code de liste (lookup id).
fn vtable_for(con: &duckdb::Connection, list_code: &str) -> duckdb::Result<String> {
    let id: i64 = con.query_row(
        "SELECT id FROM dim_value_list WHERE code = ?",
        [list_code],
        |r| r.get(0),
    )?;
    Ok(value_table(id))
}

// ───────────────────────────── Modèle / chargement ─────────────────────────────

/// Une liste de valeurs (référentiel).
#[derive(Serialize)]
pub struct ValueListDef {
    pub id: i64,
    pub code: String,
    pub libelle: String,
    pub value_table: String,
}

/// `true` si le registre existe (faux au tout premier démarrage avant DDL).
fn registry_exists(con: &duckdb::Connection) -> bool {
    con.query_row(
        "SELECT COUNT(*) = 1 FROM information_schema.tables \
         WHERE table_schema = 'main' AND table_name = 'dim_value_list'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// `true` si une liste de valeurs de ce code existe (tolère l'absence du
/// registre au premier démarrage). Utilisé par [`crate::references::target_master`]
/// pour résoudre une cible d'attribut N2 vers `lst_<code>`.
pub fn list_exists(con: &duckdb::Connection, code: &str) -> bool {
    if !registry_exists(con) {
        return false;
    }
    con.query_row(
        "SELECT COUNT(*) > 0 FROM dim_value_list WHERE code = ?",
        [code],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Charge toutes les listes de valeurs.
pub fn load_all(con: &duckdb::Connection) -> duckdb::Result<Vec<ValueListDef>> {
    if !registry_exists(con) {
        return Ok(Vec::new());
    }
    let mut stmt =
        con.prepare("SELECT id, code, libelle FROM dim_value_list ORDER BY code")?;
    let rows = stmt.query_map([], |row| {
        let id = row.get::<_, i64>(0)?;
        let code = row.get::<_, String>(1)?;
        Ok(ValueListDef {
            id,
            value_table: value_table(id),
            code,
            libelle: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        })
    })?;
    rows.collect()
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

/// `true` si `name` est déjà le nom d'une dimension (built-in ou custom). Évite
/// qu'une liste et une dimension partagent un code, ce qui rendrait la résolution
/// de cible (`references::target_master`) ambiguë.
fn collides_with_dimension(con: &duckdb::Connection, name: &str) -> bool {
    crate::dimensions::load_all(con)
        .map(|dims| dims.iter().any(|d| d.name == name))
        .unwrap_or(false)
}

/// `true` si un attribut N2 cible cette liste (empêche une suppression qui
/// laisserait une référence pendante). Tolère l'absence du registre N2.
fn is_referenced(con: &duckdb::Connection, code: &str) -> bool {
    con.query_row(
        "SELECT COUNT(*) > 0 FROM dim_characteristic_attribute WHERE target_dimension = ?",
        [code],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Crée une liste de valeurs : registre + table `lst_<code>`.
pub fn create_list(con: &duckdb::Connection, code: &str, libelle: &str) -> Result<(), AppError> {
    ensure_valid_ident("code de liste", code)?;
    if collides_with_dimension(con, code) {
        return Err(AppError::bad_request(format!(
            "'{code}' est déjà le nom d'une dimension : choisir un autre code"
        )));
    }
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_value_list WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!("liste déjà existante : {code}")));
    }
    // INSERT en premier pour récupérer l'id technique (séquence auto).
    con.execute(
        "INSERT INTO dim_value_list (code, libelle) VALUES (?, ?)",
        &[&code, &libelle],
    )
    .map_err(db_err)?;
    let id: i64 = con
        .query_row(
            "SELECT id FROM dim_value_list WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let vtable = value_table(id);
    con.execute(
        &format!("CREATE TABLE {vtable} (code TEXT PRIMARY KEY, libelle TEXT)"),
        [],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une liste de valeurs : table `lst_<id>` + registre. Refusée si un
/// attribut N2 la cible encore.
pub fn delete_list(con: &duckdb::Connection, code: &str) -> Result<(), AppError> {
    ensure_valid_ident("code de liste", code)?;
    let list_id: Option<i64> = con
        .query_row(
            "SELECT id FROM dim_value_list WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .ok();
    let list_id = list_id.ok_or_else(|| AppError::not_found(format!("liste inexistante : {code}")))?;
    if is_referenced(con, code) {
        return Err(AppError::conflict(format!(
            "liste utilisée par un attribut de caractéristique : {code}"
        )));
    }
    con.execute(&format!("DROP TABLE IF EXISTS {}", value_table(list_id)), [])
        .map_err(db_err)?;
    con.execute("DELETE FROM dim_value_list WHERE code = ?", [code])
        .map_err(db_err)?;
    Ok(())
}

// ──────────────────────────── Valeurs (lignes de lst_<code>) ─────────────────────

/// Confirme l'existence de la liste.
fn require_list(con: &duckdb::Connection, code: &str) -> Result<(), AppError> {
    if !list_exists(con, code) {
        return Err(AppError::not_found(format!("liste inexistante : {code}")));
    }
    Ok(())
}

/// Rejette les champs JSON hors `code` / `libelle`.
fn reject_unknown_value_fields(obj: &Map<String, JsonValue>) -> Result<(), AppError> {
    if let Some(k) = obj
        .keys()
        .find(|k| k.as_str() != "code" && k.as_str() != "libelle")
    {
        return Err(AppError::bad_request(format!(
            "champ inconnu pour une valeur de liste : {k} (attendus : code, libelle)"
        )));
    }
    Ok(())
}

/// Liste les valeurs (lignes) d'une liste.
pub fn list_values(con: &duckdb::Connection, code: &str) -> Result<Vec<JsonValue>, AppError> {
    require_list(con, code)?;
    let vtable = vtable_for(con, code).map_err(db_err)?;
    masterdata::run_query(
        con,
        &format!("SELECT code, libelle FROM {vtable} ORDER BY code"),
        Vec::new(),
    )
}

/// Crée une valeur (ligne de `lst_<code>`).
pub fn create_value(
    con: &duckdb::Connection,
    list_code: &str,
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    require_list(con, list_code)?;
    reject_unknown_value_fields(obj)?;
    let code_val = obj
        .get("code")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::bad_request("code de valeur requis"))?;
    let vtable = vtable_for(con, list_code).map_err(db_err)?;
    let exists: bool = con
        .query_row(
            &format!("SELECT COUNT(*) > 0 FROM {vtable} WHERE code = ?"),
            [code_val],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if exists {
        return Err(AppError::conflict(format!(
            "valeur déjà existante : {code_val}"
        )));
    }
    let libelle = obj.get("libelle").and_then(JsonValue::as_str).unwrap_or("");
    con.execute(
        &format!("INSERT INTO {vtable} (code, libelle) VALUES (?, ?)"),
        &[&code_val, &libelle],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Met à jour le libellé d'une valeur (le `code` est immuable).
pub fn update_value(
    con: &duckdb::Connection,
    list_code: &str,
    value_code: &str,
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    require_list(con, list_code)?;
    reject_unknown_value_fields(obj)?;
    let vtable = vtable_for(con, list_code).map_err(db_err)?;
    let exists: bool = con
        .query_row(
            &format!("SELECT COUNT(*) > 0 FROM {vtable} WHERE code = ?"),
            [value_code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::not_found(format!(
            "valeur inexistante : {value_code}"
        )));
    }
    let libelle = match obj.get("libelle") {
        Some(JsonValue::String(s)) => DbValue::Text(s.clone()),
        Some(JsonValue::Null) | None => return Ok(()), // rien à mettre à jour
        Some(_) => return Err(AppError::bad_request("libelle doit être une chaîne")),
    };
    con.execute(
        &format!("UPDATE {vtable} SET libelle = ? WHERE code = ?"),
        &[&libelle, &DbValue::Text(value_code.to_string())],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une valeur.
pub fn delete_value(
    con: &duckdb::Connection,
    list_code: &str,
    value_code: &str,
) -> Result<(), AppError> {
    require_list(con, list_code)?;
    let vtable = vtable_for(con, list_code).map_err(db_err)?;
    let n = con
        .execute(
            &format!("DELETE FROM {vtable} WHERE code = ?"),
            [value_code],
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!(
            "valeur inexistante : {value_code}"
        )));
    }
    Ok(())
}

// ───────────────────────────────── HTTP ─────────────────────────────────────────

#[derive(Deserialize)]
struct CreateListBody {
    code: String,
    #[serde(default)]
    libelle: String,
}

/// GET /api/meta/value-lists — liste les listes de valeurs.
async fn list(State(state): State<Arc<AppState>>) -> Result<Json<Vec<ValueListDef>>, AppError> {
    let con = lock_con(&state)?;
    Ok(Json(load_all(&con).map_err(db_err)?))
}

/// POST /api/meta/value-lists — crée une liste de valeurs.
async fn create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateListBody>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let con = lock_con(&state)?;
    create_list(&con, &body.code, &body.libelle)?;
    Ok((StatusCode::CREATED, Json(json!({ "code": body.code }))))
}

/// DELETE /api/meta/value-lists/{code} — supprime une liste.
async fn remove(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    delete_list(&con, &code)?;
    Ok(Json(json!({ "deleted": code })))
}

/// GET /api/meta/value-lists/{code}/values — liste les valeurs.
async fn values_list(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let con = lock_con(&state)?;
    Ok(Json(list_values(&con, &code)?))
}

/// POST /api/meta/value-lists/{code}/values — crée une valeur.
async fn values_create(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<JsonValue>,
) -> Result<(StatusCode, Json<JsonValue>), AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?;
    let con = lock_con(&state)?;
    create_value(&con, &code, obj)?;
    Ok((StatusCode::CREATED, Json(body)))
}

/// PUT /api/meta/value-lists/{code}/values/{value} — met à jour une valeur.
async fn values_update(
    State(state): State<Arc<AppState>>,
    Path((code, value)): Path<(String, String)>,
    Json(body): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let obj = body
        .as_object()
        .ok_or_else(|| AppError::bad_request("body doit être un objet JSON"))?;
    let con = lock_con(&state)?;
    update_value(&con, &code, &value, obj)?;
    Ok(Json(body))
}

/// DELETE /api/meta/value-lists/{code}/values/{value} — supprime une valeur.
async fn values_delete(
    State(state): State<Arc<AppState>>,
    Path((code, value)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    delete_value(&con, &code, &value)?;
    Ok(Json(json!({ "deleted": value })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/meta/value-lists", get(list).post(create))
        .route("/api/meta/value-lists/{code}", delete(remove))
        .route(
            "/api/meta/value-lists/{code}/values",
            get(values_list).post(values_create),
        )
        .route(
            "/api/meta/value-lists/{code}/values/{value}",
            put(values_update).delete(values_delete),
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

    fn table_exists(con: &Connection, table: &str) -> bool {
        con.query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables WHERE table_name = ?",
            [table],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn cree_liste_avec_table_de_valeurs() {
        let con = setup();
        create_list(&con, "incoterm", "Incoterms").unwrap();
        let lid = id_of(&con, "incoterm").unwrap();
        let vtable = value_table(lid);
        assert!(table_exists(&con, &vtable), "table de valeurs créée");
        let all = load_all(&con).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].code, "incoterm");
        assert_eq!(all[0].value_table, vtable);
    }

    #[test]
    fn refuse_collision_avec_dimension() {
        let con = setup();
        assert!(
            create_list(&con, "account", "X").is_err(),
            "account est une dimension"
        );
        crate::dimensions::create_custom(&con, "secteur", "Secteur").unwrap();
        assert!(
            create_list(&con, "secteur", "X").is_err(),
            "secteur est une dimension custom"
        );
    }

    #[test]
    fn crud_valeurs() {
        let con = setup();
        create_list(&con, "incoterm", "Incoterms").unwrap();

        let mut obj = Map::new();
        obj.insert("code".into(), json!("FOB"));
        obj.insert("libelle".into(), json!("Free On Board"));
        create_value(&con, "incoterm", &obj).unwrap();

        // Doublon rejeté.
        assert!(create_value(&con, "incoterm", &obj).is_err());

        // Champ inconnu rejeté.
        let mut bad = Map::new();
        bad.insert("code".into(), json!("X"));
        bad.insert("foo".into(), json!("bar"));
        assert!(create_value(&con, "incoterm", &bad).is_err());

        // Mise à jour du libellé.
        let mut upd = Map::new();
        upd.insert("libelle".into(), json!("Franco à bord"));
        update_value(&con, "incoterm", "FOB", &upd).unwrap();
        let lid = id_of(&con, "incoterm").unwrap();
        let lib: String = con
            .query_row(
                &format!("SELECT libelle FROM lst_{lid} WHERE code = 'FOB'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lib, "Franco à bord");

        let vals = list_values(&con, "incoterm").unwrap();
        assert_eq!(vals.len(), 1);

        delete_value(&con, "incoterm", "FOB").unwrap();
        assert_eq!(list_values(&con, "incoterm").unwrap().len(), 0);
    }

    #[test]
    fn liste_cible_un_attribut_n2_et_apparait_dans_le_graphe() {
        let con = setup();
        crate::characteristics::create_characteristic(&con, "comportement", "C", "account")
            .unwrap();
        create_list(&con, "incoterm", "Incoterms").unwrap();

        // Attribut N2 ciblant la liste (et non une dimension).
        crate::characteristics::add_attribute(&con, "comportement", "inco", "Incoterm", "incoterm")
            .unwrap();

        let char_id = crate::characteristics::id_of(&con, "comportement").unwrap();
        let list_id = id_of(&con, "incoterm").unwrap();
        let refs = crate::references::dynamic_references(&con);
        assert!(
            refs.iter().any(|r| r.table == format!("car_{char_id}")
                && r.column == "inco"
                && r.target_table == format!("lst_{list_id}")
                && r.target_column == "code"),
            "l'attribut N2 → liste apparaît dans le graphe de références"
        );

        // La liste est désormais référencée : suppression refusée.
        assert!(delete_list(&con, "incoterm").is_err());
    }

    #[test]
    fn valeur_n2_validee_contre_la_liste() {
        let con = setup();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('700', 'Ventes', 'resultat')",
            [],
        )
        .unwrap();
        crate::characteristics::create_characteristic(&con, "comportement", "C", "account")
            .unwrap();
        create_list(&con, "incoterm", "Incoterms").unwrap();
        let mut v = Map::new();
        v.insert("code".into(), json!("FOB"));
        create_value(&con, "incoterm", &v).unwrap();
        crate::characteristics::add_attribute(&con, "comportement", "inco", "Incoterm", "incoterm")
            .unwrap();

        // Valeur N1 dont l'attribut pointe une valeur de liste valide.
        let mut ok = Map::new();
        ok.insert("code".into(), json!("VAL1"));
        ok.insert("inco".into(), json!("FOB"));
        crate::characteristics::create_value(&con, "comportement", &ok).unwrap();

        // Valeur de liste inexistante → rejetée.
        let mut ko = Map::new();
        ko.insert("code".into(), json!("VAL2"));
        ko.insert("inco".into(), json!("NOPE"));
        assert!(crate::characteristics::create_value(&con, "comportement", &ko).is_err());
    }

    #[test]
    fn survit_au_reset() {
        let con = setup();
        create_list(&con, "incoterm", "Incoterms").unwrap();
        let mut v = Map::new();
        v.insert("code".into(), json!("FOB"));
        create_value(&con, "incoterm", &v).unwrap();

        let lid = id_of(&con, "incoterm").unwrap();
        crate::schema::create_schema(&con).expect("re-create_schema");

        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_value_list", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "registre survit au reset");
        assert!(
            table_exists(&con, &value_table(lid)),
            "table de valeurs survit"
        );
        assert_eq!(
            list_values(&con, "incoterm").unwrap().len(),
            1,
            "valeurs préservées"
        );
    }
}
