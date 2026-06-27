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
    routing::{delete, get, post, put},
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
    /// Identifiant technique immuable (B1 étape 11). `None` sur bases non migrées.
    pub id: Option<i64>,
    pub host_dimension: String,
    /// Nom API de la référence (mutable, stocké dans `dim_custom_reference.column_name`).
    pub column: String,
    /// Colonne physique sur `dim_<host>` : `r{id}` pour les customs, `column` pour les natives.
    pub col: String,
    pub target_dimension: String,
    /// `true` pour une référence native (FK du DDL statique, peuplée par
    /// [`seed_native`]). Verrouillée contre l'édition/suppression via l'API.
    #[serde(default)]
    pub native: bool,
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
    let has_id = con.query_row(
        "SELECT COUNT(*) > 0 FROM information_schema.columns \
         WHERE table_schema = 'main' AND table_name = 'dim_custom_reference' AND column_name = 'id'",
        [],
        |r| r.get::<_, bool>(0),
    ).unwrap_or(false);
    let sql = if has_id {
        "SELECT host_dimension, column_name, target_dimension, \
                 COALESCE(native, FALSE), id \
         FROM dim_custom_reference ORDER BY host_dimension, column_name"
    } else {
        "SELECT host_dimension, column_name, target_dimension, \
                 COALESCE(native, FALSE), NULL \
         FROM dim_custom_reference ORDER BY host_dimension, column_name"
    };
    let mut stmt = con.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let column: String = row.get(1)?;
        let native: bool = row.get(3)?;
        let id: Option<i64> = row.get(4)?;
        let col = match (native, id) {
            (false, Some(i)) => format!("r{i}"),
            _ => column.clone(), // native ou non migré : col physique = nom API
        };
        Ok(CustomReferenceDef {
            id,
            host_dimension: row.get(0)?,
            column,
            col,
            target_dimension: row.get(2)?,
            native,
        })
    })?;
    rows.collect()
}

/// Retourne la colonne physique d'une référence directe `(host, column_name)`.
/// Pour les custom (non-native) : `r{id}`. Pour les natives ou non migrées : `column_name`.
pub fn col_of_ref(
    con: &duckdb::Connection,
    host: &str,
    column_name: &str,
) -> duckdb::Result<String> {
    if !registry_exists(con) {
        return Ok(column_name.to_string());
    }
    let result: Option<(bool, Option<i64>)> = con.query_row(
        "SELECT COALESCE(native, FALSE), id \
         FROM dim_custom_reference WHERE host_dimension = ? AND column_name = ?",
        [host, column_name],
        |r| Ok((r.get::<_, bool>(0)?, r.get::<_, Option<i64>>(1)?)),
    ).ok();
    Ok(match result {
        Some((false, Some(id))) => format!("r{id}"),
        _ => column_name.to_string(),
    })
}

/// `true` si `(host, column)` est marqué `native=true`. Retourne `false` si la
/// ligne n'existe pas ou si le registre est absent.
fn is_native(con: &duckdb::Connection, host: &str, column: &str) -> duckdb::Result<bool> {
    if !registry_exists(con) {
        return Ok(false);
    }
    con.query_row(
        "SELECT COALESCE(native, FALSE) FROM dim_custom_reference \
         WHERE host_dimension = ? AND column_name = ?",
        [host, column],
        |r| r.get::<_, bool>(0),
    )
    .or_else(|e| match e {
        duckdb::Error::QueryReturnedNoRows => Ok(false),
        e => Err(e),
    })
}

/// Dimension cible d'une référence directe `(host, column)`, si elle existe.
/// Utilisé par le moteur de règles pour valider / bâtir la traversée
/// `map_ref` (sélection et destination) sur une référence de patron B.
pub fn target_of(
    con: &duckdb::Connection,
    host: &str,
    column: &str,
) -> duckdb::Result<Option<String>> {
    if !registry_exists(con) {
        return Ok(None);
    }
    match con.query_row(
        "SELECT target_dimension FROM dim_custom_reference \
         WHERE host_dimension = ? AND column_name = ?",
        [host, column],
        |r| r.get::<_, String>(0),
    ) {
        Ok(s) => Ok(Some(s)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
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
                &format!("ALTER TABLE {host_table} ADD COLUMN \"{}\" TEXT", r.col),
                [],
            );
        }
    }
    Ok(())
}

/// Peuple `dim_custom_reference` avec les **FK natives** listées dans
/// `references::NATIVE_MASTER_REFS` (clés étrangères du DDL statique des master
/// data : `account.sous_classe`, `entity.entite_parent`, `scenario.category`,
/// etc.). Les lignes sont marquées `native=TRUE` pour les verrouiller contre
/// édition/suppression (cf. [`create`] / [`remove`]).
///
/// Idempotent : `INSERT OR IGNORE` préserve les lignes déjà présentes (custom ou
/// native). À appeler après `create_schema` (bases fraîches) et au démarrage
/// serveur pour migrer les bases existantes (cf. [`migrate_native`]).
pub fn seed_native(con: &duckdb::Connection) -> duckdb::Result<()> {
    if !registry_exists(con) {
        return Ok(());
    }
    for &(host, column, target) in references::NATIVE_MASTER_REFS {
        con.execute(
            "INSERT OR IGNORE INTO dim_custom_reference \
             (host_dimension, column_name, target_dimension, native) \
             VALUES (?, ?, ?, TRUE)",
            [&host, &column, &target],
        )?;
    }
    Ok(())
}

/// Migration idempotente : ajoute la colonne `native` au registre si elle
/// manque (bases DuckDB existantes antérieures à l'introduction du flag), puis
/// appelle [`seed_native`]. À appeler au démarrage serveur, inconditionnellement,
/// avant le branchement « base déjà initialisée » — couvre donc les bases
/// existantes sans toucher aux éditions utilisateur.
///
/// DuckDB ne supporte pas `ADD COLUMN ... NOT NULL DEFAULT` : on ADD sans
/// contrainte, puis on UPDATE les NULL existants vers FALSE (cohérent avec le
/// DDL `CREATE TABLE` qui, lui, spécifie `NOT NULL DEFAULT FALSE` sur les bases
/// fraîches — la lecture utilise `COALESCE(native, FALSE)` partout).
pub fn migrate_native(con: &duckdb::Connection) -> duckdb::Result<()> {
    if !registry_exists(con) {
        return Ok(());
    }
    let native_col_exists: bool = con.query_row(
        "SELECT COUNT(*) > 0 FROM information_schema.columns \
         WHERE table_schema = 'main' \
           AND table_name = 'dim_custom_reference' \
           AND column_name = 'native'",
        [],
        |r| r.get(0),
    )?;
    if !native_col_exists {
        con.execute(
            "ALTER TABLE dim_custom_reference ADD COLUMN native BOOLEAN",
            [],
        )?;
        con.execute(
            "UPDATE dim_custom_reference SET native = FALSE WHERE native IS NULL",
            [],
        )?;
    }
    seed_native(con)
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
///
/// Refuse `(host, column)` déjà occupé par une référence **native** (FK du DDL
/// statique, marquée `native=TRUE` par [`seed_native`]). Ces références reflètent
/// le schéma et ne sont pas éditables via l'API.
pub fn create(
    con: &duckdb::Connection,
    host: &str,
    column: &str,
    target: &str,
) -> Result<(), AppError> {
    ensure_valid_column(column)?;
    if is_native(con, host, column).map_err(db_err)? {
        return Err(AppError::conflict(format!(
            "référence native (non éditable) : {host}.{column}"
        )));
    }
    let (host_table, _) = references::dimension_master(host).ok_or_else(|| {
        AppError::bad_request(format!(
            "dimension hôte inconnue ou sans master data : {host}"
        ))
    })?;
    // La cible peut être une dimension d'écriture (`account`, `currency`…) ou
    // une master data secondaire (`sous_classe`, `flow_scheme`…) — résolue par
    // `target_master` qui englobe les deux.
    if references::target_master(con, target).is_none() {
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
    // INSERT d'abord pour obtenir l'id généré par la séquence,
    // puis ADD COLUMN avec le nom physique r{id}.
    con.execute(
        "INSERT INTO dim_custom_reference (host_dimension, column_name, target_dimension, native) \
         VALUES (?, ?, ?, FALSE)",
        &[&host, &column, &target],
    )
    .map_err(db_err)?;
    let new_id: i64 = con
        .query_row(
            "SELECT id FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [&host, &column],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let phys_col = format!("r{new_id}");
    con.execute(
        &format!("ALTER TABLE {host_table} ADD COLUMN \"{phys_col}\" TEXT"),
        [],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une référence directe : colonne sur la master data hôte + registre.
///
/// Refuse les références **natives** (FK du DDL statique) : leur suppression
/// casserait les règles existantes qui s'y appuient et le catalogue système.
pub fn remove(con: &duckdb::Connection, host: &str, column: &str) -> Result<(), AppError> {
    ensure_valid_column(column)?;
    if is_native(con, host, column).map_err(db_err)? {
        return Err(AppError::conflict(format!(
            "référence native (non supprimable) : {host}.{column}"
        )));
    }
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
    let phys_col = col_of_ref(con, host, column).map_err(db_err)?;
    if let Some((host_table, _)) = references::dimension_master(host) {
        // Silencieux si la colonne a déjà disparu (ex. après un reset partiel).
        let _ = con.execute(
            &format!("ALTER TABLE {host_table} DROP COLUMN \"{phys_col}\""),
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
    let phys_col = col_of_ref(con, host, column).map_err(db_err)?;
    let sql = format!("UPDATE {host_table} SET \"{phys_col}\" = ? WHERE \"{host_key}\" = ?");
    con.execute(
        &sql,
        params_from_iter(vec![dbval, DbValue::Text(member.to_string())]),
    )
    .map_err(db_err)?;
    Ok(())
}

/// Scanne les JSON de règles et postes pour détecter les blockers avant un
/// renommage de colonne API `(host, column_name)`. Retourne une liste de codes
/// bloquants, ou `Ok(vec![])` si aucun.
pub fn scan_blockers(
    con: &duckdb::Connection,
    host: &str,
    column_name: &str,
) -> duckdb::Result<Vec<String>> {
    let mut blockers = Vec::new();
    // dim_rule.definition : `"ref": "<column_name>"` sous `selection` ou `destination`
    let mut stmt = con.prepare(
        "SELECT code, definition FROM dim_rule WHERE definition IS NOT NULL",
    )?;
    let rules: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .flatten()
        .collect();
    for (code, def) in &rules {
        if def.contains(column_name) {
            blockers.push(format!("règle {code}"));
        }
    }
    // dim_aggregate.definition : idem
    if let Ok(mut stmt2) = con.prepare(
        "SELECT code, definition FROM dim_aggregate WHERE definition IS NOT NULL",
    ) {
        let aggs: Vec<(String, String)> = stmt2
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();
        for (code, def) in &aggs {
            if def.contains(column_name) {
                blockers.push(format!("poste {code}"));
            }
        }
    }
    // Filtre faux positifs : si le column_name est très court, la recherche
    // naive peut matcher des sous-chaînes. On l'accepte (conservateur = sûr).
    let _ = host; // host non utilisé pour l'instant (scan global sur le nom)
    Ok(blockers)
}

/// Renomme le nom API d'une référence directe custom (UPDATE `column_name`).
/// La colonne physique (`r{id}`) reste inchangée. Refuse les natifs.
pub fn rename_custom_ref(
    con: &duckdb::Connection,
    host: &str,
    old_name: &str,
    new_name: &str,
) -> Result<(), AppError> {
    if !is_valid_custom_name(new_name) {
        return Err(AppError::bad_request(format!(
            "nom invalide : {new_name:?}"
        )));
    }
    if is_native(con, host, old_name).map_err(db_err)? {
        return Err(AppError::conflict(format!(
            "référence native (non renommable) : {host}.{old_name}"
        )));
    }
    // Blocker check
    let blockers = scan_blockers(con, host, old_name).map_err(db_err)?;
    if !blockers.is_empty() {
        return Err(AppError::conflict(format!(
            "référence citée dans : {}",
            blockers.join(", ")
        )));
    }
    // Vérifier que old_name existe
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [host, old_name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::not_found(format!(
            "référence inexistante : {host}.{old_name}"
        )));
    }
    // Vérifier que new_name est libre
    let conflict: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_custom_reference \
             WHERE host_dimension = ? AND column_name = ?",
            [host, new_name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if conflict {
        return Err(AppError::conflict(format!(
            "nom déjà utilisé : {host}.{new_name}"
        )));
    }
    con.execute(
        "UPDATE dim_custom_reference SET column_name = ? \
         WHERE host_dimension = ? AND column_name = ?",
        [new_name, host, old_name],
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

#[derive(Deserialize)]
struct RenameBody {
    new_name: String,
}

/// POST /api/meta/references-custom/{host}/{column}/rename — renomme le nom API.
///
/// La colonne physique (`r{id}`) reste inchangée. Retourne 409 si des règles
/// ou postes citent encore ce nom, 404 si la référence est inconnue.
async fn rename_handler(
    Path((host, column)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RenameBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    rename_custom_ref(&con, &host, &column, &body.new_name)?;
    Ok(Json(json!({ "renamed": true, "column": body.new_name })))
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
        .route(
            "/api/meta/references-custom/{host}/{column}/rename",
            post(rename_handler),
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
        // La référence créée est trouvée et marquée non-native (les natives sont
        // peuplées par `seed_native` au setup).
        let all = load_all(&con).unwrap();
        let ours = all
            .iter()
            .find(|r| r.column == "compte_parent" && r.host_dimension == "account")
            .expect("référence custom présente");
        assert_eq!(ours.target_dimension, "account");
        assert!(!ours.native, "référence utilisateur = native FALSE");
        // B1 étape 11 : la colonne physique est r{id}, pas le nom API.
        assert!(
            col_exists(&con, "dim_account", &ours.col),
            "colonne physique {} ajoutée sur dim_account", ours.col
        );
        assert!(ours.col.starts_with('r'), "col physique commence par r");
    }

    #[test]
    fn exposee_dans_le_graphe_de_references() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();
        // B1 étape 11 : dynamic_references retourne la col physique r{id}.
        let phys = col_of_ref(&con, "account", "compte_parent").unwrap();
        let refs = references::dynamic_references(&con);
        assert!(
            refs.iter().any(|r| r.table == "dim_account"
                && r.column == phys
                && r.target_table == "dim_account"
                && r.target_column == "code"),
            "la référence directe apparaît dans dynamic_references (col physique {phys})"
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
    fn accepte_cible_master_data_secondaire() {
        // Une référence custom peut cibler une master data secondaire (ex.
        // `sous_classe`), résolue via `references::target_master`.
        let con = setup();
        // `account.comportement` → sous_classe (inventé pour le test).
        create(&con, "account", "comportement", "sous_classe").unwrap();
        let all = load_all(&con).unwrap();
        assert!(all.iter().any(|r| r.host_dimension == "account"
            && r.column == "comportement"
            && r.target_dimension == "sous_classe"));
    }

    #[test]
    fn seed_native_peuple_les_fk_natives() {
        let con = setup();
        // Après create_schema, les 12 FK natives du catalogue sont présentes.
        let all = load_all(&con).unwrap();
        let natives: Vec<_> = all.iter().filter(|r| r.native).collect();
        assert_eq!(
            natives.len(),
            references::NATIVE_MASTER_REFS.len(),
            "toutes les FK natives sont seedées"
        );
        // Vérifie deux cas représentatifs.
        assert!(natives.iter().any(|r| r.host_dimension == "account"
            && r.column == "sous_classe"
            && r.target_dimension == "sous_classe"));
        assert!(natives.iter().any(|r| r.host_dimension == "entity"
            && r.column == "entite_parent"
            && r.target_dimension == "entity"));
    }

    #[test]
    fn seed_native_est_idempotent() {
        let con = setup();
        let n1: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| r.get(0))
            .unwrap();
        seed_native(&con).unwrap();
        let n2: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n1, n2, "seed_native ne double pas les insertions");
    }

    #[test]
    fn create_refuse_sur_ligne_native() {
        let con = setup();
        let err = create(&con, "account", "sous_classe", "sous_classe").unwrap_err();
        let msg = err.1;
        assert!(
            msg.contains("native") || msg.contains("déjà existante"),
            "message d'erreur pertinent : {msg}"
        );
    }

    #[test]
    fn remove_refuse_sur_ligne_native() {
        let con = setup();
        let err = remove(&con, "account", "sous_classe").unwrap_err();
        let msg = err.1;
        assert!(msg.contains("native"), "refus suppression native : {msg}");
    }

    #[test]
    fn migrate_native_idempotent_apres_setup() {
        // Après create_schema (qui appelle déjà seed_native), migrate_native
        // doit rester idempotent : il ne double pas les insertions et ne
        // plante pas (ALTER ADD COLUMN IF NOT EXISTS silencieux sur la colonne
        // déjà présente).
        let con = setup();
        let before: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| r.get(0))
            .unwrap();
        migrate_native(&con).unwrap();
        let after: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "migrate_native idempotent");
        // Toutes les lignes attendues du catalogue natif sont marquées native.
        let all = load_all(&con).unwrap();
        for &(host, column, _target) in references::NATIVE_MASTER_REFS {
            assert!(
                all.iter().any(|r| r.host_dimension == host
                    && r.column == column
                    && r.native),
                "référence native présente : {host}.{column}"
            );
        }
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
        // B1 étape 11 : la col physique est r{id}.
        let phys = col_of_ref(&con, "account", "compte_parent").unwrap();

        // Affecte le parent 60 au compte 600.
        assign(&con, "account", "compte_parent", "600", Some("60")).unwrap();
        let parent: Option<String> = con
            .query_row(
                &format!("SELECT \"{phys}\" FROM dim_account WHERE code = '600'"),
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
                &format!("SELECT \"{phys}\" FROM dim_account WHERE code = '600'"),
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
        // B1 étape 11 : la col physique est r{id}.
        let phys = col_of_ref(&con, "account", "compte_parent").unwrap();
        // Combien de références avant reset (natives + notre custom).
        let before: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| r.get(0))
            .unwrap();

        // Reset complet du schéma.
        crate::schema::create_schema(&con).expect("re-create_schema");

        let after: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_custom_reference", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(before, after, "registre survit au reset (natives + custom)");
        assert!(
            col_exists(&con, "dim_account", &phys),
            "colonne physique {phys} réappliquée après reset"
        );
    }

    #[test]
    fn suppression_nettoie() {
        let con = setup();
        create(&con, "account", "compte_parent", "account").unwrap();
        let phys = col_of_ref(&con, "account", "compte_parent").unwrap();
        remove(&con, "account", "compte_parent").unwrap();
        assert!(
            !col_exists(&con, "dim_account", &phys),
            "colonne physique {phys} retirée"
        );
        // Les natives restent (le custom supprimé ne touche pas aux autres).
        let all = load_all(&con).unwrap();
        assert!(all.iter().all(|r| r.native), "il ne reste que des natives");
    }
}
