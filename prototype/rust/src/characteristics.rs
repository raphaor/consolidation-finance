//! Caractéristiques de regroupement (**N1**) et leurs attributs typés (**N2**).
//!
//! Une **caractéristique N1** classe les membres d'une dimension de base (ex.
//! `comportement` sur les comptes). Chaque valeur N1 (ligne de `car_<code>`)
//! porte des **attributs N2**, chacun étant une référence vers une dimension
//! (`compte_destination → dim_account`, `nature → dim_nature`…). Une règle
//! pourra (incrément ultérieur) router une écriture en traversant ces attributs.
//! La cible d'un attribut N2 peut aussi être une **liste de valeurs**
//! réutilisable (`lst_<code>`, cf. [`crate::value_lists`]) plutôt qu'une dimension.
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

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use duckdb::{params_from_iter, types::Value as DbValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as JsonValue};

use crate::dimensions::is_valid_custom_name;
use crate::state::{db_err, lock_con, AppError, AppState};
use crate::{masterdata, references};

/// Nom physique de la table de valeurs d'une caractéristique N1 (B1 étape 5 :
/// nommée par `id` technique, pas par `code`).
pub fn value_table(id: i64) -> String {
    format!("car_{id}")
}

/// `id` technique d'une caractéristique N1, ou `None` si elle n'existe pas.
pub fn id_of(con: &duckdb::Connection, code: &str) -> Option<i64> {
    con.query_row(
        "SELECT id FROM dim_characteristic WHERE code = ?",
        [code],
        |r| r.get(0),
    )
    .ok()
}

/// Nom physique de la table de valeurs pour un code N1 (lookup id).
/// Retourne une erreur si la caractéristique n'existe pas.
fn vtable_for(con: &duckdb::Connection, char_code: &str) -> duckdb::Result<String> {
    let id: i64 = con.query_row(
        "SELECT id FROM dim_characteristic WHERE code = ?",
        [char_code],
        |r| r.get(0),
    )?;
    Ok(value_table(id))
}

// ───────────────────────────── Modèle / chargement ─────────────────────────────

/// Un attribut N2 (colonne typée sur la table de valeurs d'une N1).
#[derive(Serialize)]
pub struct AttributeDef {
    pub id: i64,      // clé technique B1 ; nom physique = c{id} sur car_<char_id>
    pub name: String, // code mutable (contrat API/JSON)
    pub libelle: String,
    pub target_dimension: String,
}

/// Nom physique d'une colonne N2 sur `car_<char_id>`.
pub fn attr_col(attr_id: i64) -> String {
    format!("c{attr_id}")
}

/// Résout `(char_code, attr_name)` → nom de colonne physique (`c<attr_id>`).
/// Retourne `None` si l'attribut n'existe pas.
pub fn attr_col_for(
    con: &duckdb::Connection,
    char_code: &str,
    attr_name: &str,
) -> Option<String> {
    con.query_row(
        "SELECT id FROM dim_characteristic_attribute \
         WHERE characteristic_code = ? AND name = ?",
        [char_code, attr_name],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .map(attr_col)
}

/// Une caractéristique N1 avec ses attributs N2.
#[derive(Serialize)]
pub struct CharacteristicDef {
    pub id: i64,
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
    let chars: Vec<(i64, String, String, String)> = {
        let mut stmt = con.prepare(
            "SELECT id, code, libelle, base_dimension FROM dim_characteristic ORDER BY code",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<duckdb::Result<_>>()?
    };
    let mut out = Vec::with_capacity(chars.len());
    for (id, code, libelle, base_dimension) in chars {
        let attributes = load_attributes(con, &code)?;
        out.push(CharacteristicDef {
            id,
            value_table: value_table(id),
            code,
            libelle,
            base_dimension,
            attributes,
        });
    }
    Ok(out)
}

/// Dimension de base d'une caractéristique N1, si elle existe. Utilisé par le
/// moteur de règles pour bâtir la traversée (mode `map`).
pub fn base_dimension_of(con: &duckdb::Connection, code: &str) -> duckdb::Result<Option<String>> {
    match con.query_row(
        "SELECT base_dimension FROM dim_characteristic WHERE code = ?",
        [code],
        |r| r.get::<_, String>(0),
    ) {
        Ok(s) => Ok(Some(s)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Dimension cible d'un attribut N2 (`characteristic_code.name`), si l'attribut
/// existe. Utilisé par le moteur de règles pour valider la compatibilité de type
/// d'un mapping (la dimension écrite doit correspondre à `target_dimension`).
pub fn attribute_target(
    con: &duckdb::Connection,
    char_code: &str,
    attr: &str,
) -> duckdb::Result<Option<String>> {
    match con.query_row(
        "SELECT target_dimension FROM dim_characteristic_attribute \
         WHERE characteristic_code = ? AND name = ?",
        [char_code, attr],
        |r| r.get::<_, String>(0),
    ) {
        Ok(s) => Ok(Some(s)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

fn load_attributes(con: &duckdb::Connection, char_code: &str) -> duckdb::Result<Vec<AttributeDef>> {
    let mut stmt = con.prepare(
        "SELECT id, name, libelle, target_dimension FROM dim_characteristic_attribute \
         WHERE characteristic_code = ? ORDER BY name",
    )?;
    let rows = stmt.query_map([char_code], |row| {
        Ok(AttributeDef {
            id: row.get::<_, i64>(0)?,
            name: row.get::<_, String>(1)?,
            libelle: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            target_dimension: row.get::<_, String>(3)?,
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
    // INSERT en premier pour récupérer l'id technique (séquence auto).
    con.execute(
        "INSERT INTO dim_characteristic (code, libelle, base_dimension) VALUES (?, ?, ?)",
        &[&code, &libelle, &base_dimension],
    )
    .map_err(db_err)?;
    let id: i64 = con
        .query_row(
            "SELECT id FROM dim_characteristic WHERE code = ?",
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
    con.execute(
        &format!("ALTER TABLE {base_table} ADD COLUMN {code} TEXT"),
        [],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Supprime une caractéristique N1 : table de valeurs + colonne de rattachement
/// + attributs N2 + registre.
pub fn delete_characteristic(con: &duckdb::Connection, code: &str) -> Result<(), AppError> {
    ensure_valid_ident("code de caractéristique", code)?;
    let (id, base_dimension): (i64, String) = con
        .query_row(
            "SELECT id, base_dimension FROM dim_characteristic WHERE code = ?",
            [code],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::not_found(format!("caractéristique inexistante : {code}")))?;
    let vtable = value_table(id);
    con.execute(&format!("DROP TABLE IF EXISTS {vtable}"), [])
        .map_err(db_err)?;
    if let Some((base_table, _)) = references::dimension_master(&base_dimension) {
        // Silencieux si la colonne a déjà disparu (ex. après un reset partiel).
        let _ = con.execute(&format!("ALTER TABLE {base_table} DROP COLUMN {code}"), []);
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

/// Ajoute un attribut N2 : colonne sur `car_<char>` + registre. La cible
/// (`target_dimension`) peut être une **dimension** à master data **ou** une
/// **liste de valeurs** (`lst_<code>`, cf. [`crate::value_lists`]) — résolue via
/// [`references::target_master`].
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
    if references::target_master(con, target_dimension).is_none() {
        return Err(AppError::bad_request(format!(
            "cible inconnue : {target_dimension} (ni dimension à master data, ni liste de valeurs)"
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
        return Err(AppError::conflict(format!(
            "attribut déjà existant : {name}"
        )));
    }
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
    // INSERT d'abord pour obtenir l'id technique → le nom physique sera c{id}.
    con.execute(
        "INSERT INTO dim_characteristic_attribute \
         (characteristic_code, name, libelle, target_dimension) VALUES (?, ?, ?, ?)",
        &[&char_code, &name, &libelle, &target_dimension],
    )
    .map_err(db_err)?;
    let attr_id: i64 = con
        .query_row(
            "SELECT id FROM dim_characteristic_attribute \
             WHERE characteristic_code = ? AND name = ?",
            [char_code, name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let col = attr_col(attr_id);
    con.execute(&format!("ALTER TABLE {vtable} ADD COLUMN \"{col}\" TEXT"), [])
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
    let attr_id: i64 = con
        .query_row(
            "SELECT id FROM dim_characteristic_attribute \
             WHERE characteristic_code = ? AND name = ?",
            [char_code, name],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let col = attr_col(attr_id);
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
    con.execute(&format!("ALTER TABLE {vtable} DROP COLUMN \"{col}\""), [])
        .map_err(db_err)?;
    con.execute(
        "DELETE FROM dim_characteristic_attribute WHERE characteristic_code = ? AND name = ?",
        [char_code, name],
    )
    .map_err(db_err)?;
    Ok(())
}

// ─────────────────────── Valeurs N1 (lignes de car_<code>) ──────────────────────

/// Clés API/JSON d'une valeur N1 : `code`, `libelle` + les noms d'attributs.
/// Utilisé pour valider et lire les corps de requête.
fn value_api_keys(attrs: &[AttributeDef]) -> Vec<String> {
    let mut cols = vec!["code".to_string(), "libelle".to_string()];
    cols.extend(attrs.iter().map(|a| a.name.clone()));
    cols
}

/// Expression SELECT pour une valeur N1 : alias physique → clé API.
/// `"c{id}" AS "{name}"` pour les attributs N2, colonnes nues pour code/libelle.
fn select_clause(attrs: &[AttributeDef]) -> String {
    let mut parts = vec!["\"code\"".to_string(), "\"libelle\"".to_string()];
    parts.extend(
        attrs
            .iter()
            .map(|a| format!("\"{}\" AS \"{}\"", attr_col(a.id), a.name)),
    );
    parts.join(", ")
}

/// Traduit une clé JSON d'attribut (nom) en nom de colonne physique (`c{id}`).
/// Retourne le nom physique si trouvé, sinon le nom tel quel (fallback base fraîche).
fn physical_col(attrs: &[AttributeDef], api_key: &str) -> String {
    if api_key == "code" || api_key == "libelle" {
        return api_key.to_string();
    }
    attrs
        .iter()
        .find(|a| a.name == api_key)
        .map(|a| attr_col(a.id))
        .unwrap_or_else(|| api_key.to_string())
}

/// Confirme l'existence de la caractéristique et renvoie ses attributs N2.
fn require_characteristic(
    con: &duckdb::Connection,
    char_code: &str,
) -> Result<Vec<AttributeDef>, AppError> {
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic WHERE code = ?",
            [char_code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if !exists {
        return Err(AppError::not_found(format!(
            "caractéristique inexistante : {char_code}"
        )));
    }
    load_attributes(con, char_code).map_err(db_err)
}

/// Rejette les champs JSON hors `code` / `libelle` / attributs N2.
fn reject_unknown_value_fields(
    attrs: &[AttributeDef],
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    let allowed: HashSet<String> = value_api_keys(attrs).into_iter().collect();
    if let Some(k) = obj.keys().find(|k| !allowed.contains(k.as_str())) {
        return Err(AppError::bad_request(format!(
            "champ inconnu pour une valeur de caractéristique : {k}"
        )));
    }
    Ok(())
}

/// Vérifie que chaque valeur d'attribut N2 fournie existe dans la master data de
/// sa dimension cible (référence dynamique, cf. [`references`]).
fn validate_attribute_values(
    con: &duckdb::Connection,
    attrs: &[AttributeDef],
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    for a in attrs {
        let v = match obj.get(&a.name) {
            Some(JsonValue::String(s)) if !s.is_empty() => s.as_str(),
            _ => continue, // absent / null / vide = non renseigné (attributs nullables)
        };
        let (tt, tc) = references::target_master(con, &a.target_dimension).ok_or_else(|| {
            AppError::bad_request(format!("cible inconnue : {}", a.target_dimension))
        })?;
        if !references::value_exists(con, &tt, &tc, v).map_err(db_err)? {
            return Err(AppError::bad_request(format!(
                "{} = '{}' (absent de {}.{})",
                a.name, v, tt, tc
            )));
        }
    }
    Ok(())
}

/// Liste les valeurs (lignes) d'une caractéristique N1.
pub fn list_values(con: &duckdb::Connection, char_code: &str) -> Result<Vec<JsonValue>, AppError> {
    let attrs = require_characteristic(con, char_code)?;
    let cols = select_clause(&attrs);
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
    masterdata::run_query(
        con,
        &format!("SELECT {cols} FROM {vtable} ORDER BY code"),
        Vec::new(),
    )
}

/// Crée une valeur N1 (ligne de `car_<id>`). `code` requis ; attributs N2
/// validés contre leur dimension cible.
pub fn create_value(
    con: &duckdb::Connection,
    char_code: &str,
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    let attrs = require_characteristic(con, char_code)?;
    reject_unknown_value_fields(&attrs, obj)?;
    let code_val = obj
        .get("code")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::bad_request("code de valeur requis"))?;
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
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
    validate_attribute_values(con, &attrs, obj)?;
    let mut cols = Vec::new();
    let mut vals: Vec<DbValue> = Vec::new();
    for (k, v) in obj {
        // Traduire la clé API (nom) en nom de colonne physique (c{id} pour les N2).
        cols.push(format!("\"{}\"", physical_col(&attrs, k)));
        vals.push(masterdata::json_to_db_value(v));
    }
    let placeholders = vals.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "INSERT INTO {vtable} ({}) VALUES ({})",
        cols.join(", "),
        placeholders
    );
    con.execute(&sql, params_from_iter(vals)).map_err(db_err)?;
    Ok(())
}

/// Met à jour une valeur N1 (le `code` est immuable).
pub fn update_value(
    con: &duckdb::Connection,
    char_code: &str,
    value_code: &str,
    obj: &Map<String, JsonValue>,
) -> Result<(), AppError> {
    let attrs = require_characteristic(con, char_code)?;
    reject_unknown_value_fields(&attrs, obj)?;
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
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
    validate_attribute_values(con, &attrs, obj)?;
    let mut sets = Vec::new();
    let mut vals: Vec<DbValue> = Vec::new();
    for (k, v) in obj {
        if k == "code" {
            continue; // PK immuable
        }
        sets.push(format!("\"{}\" = ?", physical_col(&attrs, k)));
        vals.push(masterdata::json_to_db_value(v));
    }
    if sets.is_empty() {
        return Ok(());
    }
    vals.push(DbValue::Text(value_code.to_string()));
    let sql = format!("UPDATE {vtable} SET {} WHERE code = ?", sets.join(", "));
    con.execute(&sql, params_from_iter(vals)).map_err(db_err)?;
    Ok(())
}

/// Supprime une valeur N1.
pub fn delete_value(
    con: &duckdb::Connection,
    char_code: &str,
    value_code: &str,
) -> Result<(), AppError> {
    require_characteristic(con, char_code)?;
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
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

/// Affecte (ou retire, si `value` est `None`/vide) une valeur N1 à un membre de
/// la dimension de base (ex. classer le compte `700` en `VENTES_IC`).
pub fn assign(
    con: &duckdb::Connection,
    char_code: &str,
    member: &str,
    value: Option<&str>,
) -> Result<(), AppError> {
    let base_dimension: String = con
        .query_row(
            "SELECT base_dimension FROM dim_characteristic WHERE code = ?",
            [char_code],
            |r| r.get(0),
        )
        .map_err(|_| AppError::not_found(format!("caractéristique inexistante : {char_code}")))?;
    let (base_table, base_key) =
        references::dimension_master(&base_dimension).ok_or_else(|| {
            AppError::bad_request(format!(
                "dimension de base sans master data : {base_dimension}"
            ))
        })?;
    if !references::value_exists(con, base_table, base_key, member).map_err(db_err)? {
        return Err(AppError::not_found(format!(
            "membre inexistant : {base_table}.{base_key} = {member}"
        )));
    }
    let vtable = vtable_for(con, char_code).map_err(db_err)?;
    let dbval = match value {
        Some(v) if !v.is_empty() => {
            if !references::value_exists(con, &vtable, "code", v).map_err(db_err)? {
                return Err(AppError::bad_request(format!(
                    "valeur inexistante : {vtable}.code = {v}"
                )));
            }
            DbValue::Text(v.to_string())
        }
        _ => DbValue::Null, // désaffectation
    };
    let sql = format!("UPDATE {base_table} SET \"{char_code}\" = ? WHERE \"{base_key}\" = ?");
    con.execute(
        &sql,
        params_from_iter(vec![dbval, DbValue::Text(member.to_string())]),
    )
    .map_err(db_err)?;
    Ok(())
}

/// Met à jour le libellé d'une caractéristique N1.
pub fn update_characteristic(
    con: &duckdb::Connection,
    char_code: &str,
    libelle: &str,
) -> Result<(), AppError> {
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM dim_characteristic WHERE code = ?",
            [char_code],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if n == 0 {
        return Err(AppError::not_found(format!(
            "caractéristique inconnue : {char_code}"
        )));
    }
    con.execute(
        "UPDATE dim_characteristic SET libelle = ? WHERE code = ?",
        [libelle, char_code],
    )
    .map_err(db_err)?;
    Ok(())
}

/// Renomme le code d'une caractéristique N1.
///
/// Cascade :
/// - colonne `{old}` sur la master data de la dimension de base → renommée `{new}` ;
/// - `dim_characteristic_attribute.characteristic_code` : mise à jour en place.
/// La table physique `car_<id>` n'est **pas** touchée — c'est le gain B1.
pub fn rename_characteristic_code(
    con: &duckdb::Connection,
    old: &str,
    new: &str,
) -> Result<(), AppError> {
    if new.is_empty() {
        return Err(AppError::bad_request("nouveau code vide"));
    }
    if new == old {
        return Ok(());
    }
    if !is_valid_custom_name(new) {
        return Err(AppError::bad_request(format!(
            "code invalide : {new:?} (alphanumérique + underscore, \
             premier caractère lettre ou underscore, non réservé)"
        )));
    }
    // Résoudre l'ancien code : on a besoin de la dimension de base pour renommer la colonne.
    let base_dim: Option<String> = con
        .query_row(
            "SELECT base_dimension FROM dim_characteristic WHERE code = ?",
            [old],
            |r| r.get(0),
        )
        .ok();
    let base_dim = base_dim
        .ok_or_else(|| AppError::not_found(format!("caractéristique inconnue : {old}")))?;
    // Conflit : le nouveau code existe déjà.
    let conflict: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic WHERE code = ?",
            [new],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if conflict {
        return Err(AppError::conflict(format!("code déjà utilisé : {new}")));
    }
    // Renommer la colonne sur la master data de la dimension de base.
    if let Some((base_table, _)) = references::dimension_master(&base_dim) {
        con.execute(
            &format!("ALTER TABLE {base_table} RENAME COLUMN \"{old}\" TO \"{new}\""),
            [],
        )
        .map_err(db_err)?;
    }
    // Mettre à jour le registre.
    con.execute(
        "UPDATE dim_characteristic SET code = ? WHERE code = ?",
        [new, old],
    )
    .map_err(db_err)?;
    // Cascade FK dans dim_characteristic_attribute.
    con.execute(
        "UPDATE dim_characteristic_attribute \
         SET characteristic_code = ? WHERE characteristic_code = ?",
        [new, old],
    )
    .map_err(db_err)?;
    Ok(())
}

// ───────────────────────────────── HTTP ─────────────────────────────────────────

#[derive(Deserialize)]
struct RenameCharBody {
    new_code: String,
}

#[derive(Deserialize)]
struct CreateCharacteristicBody {
    code: String,
    #[serde(default)]
    libelle: String,
    base_dimension: String,
}

#[derive(Deserialize)]
struct UpdateCharacteristicBody {
    libelle: String,
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

/// PUT /api/meta/characteristics/{code} — modifie le libellé d'une N1.
async fn update(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<UpdateCharacteristicBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    update_characteristic(&con, &code, &body.libelle)?;
    Ok(Json(json!({ "code": code, "libelle": body.libelle })))
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
    add_attribute(
        &con,
        &code,
        &body.name,
        &body.libelle,
        &body.target_dimension,
    )?;
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

#[derive(Deserialize)]
struct AssignBody {
    member: String,
    #[serde(default)]
    value: Option<String>,
}

/// GET /api/meta/characteristics/{code}/values — liste les valeurs N1.
async fn values_list(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let con = lock_con(&state)?;
    Ok(Json(list_values(&con, &code)?))
}

/// POST /api/meta/characteristics/{code}/values — crée une valeur N1.
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

/// PUT /api/meta/characteristics/{code}/values/{value} — met à jour une valeur.
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

/// DELETE /api/meta/characteristics/{code}/values/{value} — supprime une valeur.
async fn values_delete(
    State(state): State<Arc<AppState>>,
    Path((code, value)): Path<(String, String)>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    delete_value(&con, &code, &value)?;
    Ok(Json(json!({ "deleted": value })))
}

/// POST /api/meta/characteristics/{code}/rename — renomme le code d'une N1.
async fn rename_char_handler(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<RenameCharBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    rename_characteristic_code(&con, &code, &body.new_code)?;
    let _ = con.execute("CHECKPOINT", []);
    Ok(Json(json!({ "renamed": { "old": code, "new": body.new_code } })))
}

/// PUT /api/meta/characteristics/{code}/assign — classe (ou déclasse) un membre.
async fn assign_handler(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
    Json(body): Json<AssignBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    assign(&con, &code, &body.member, body.value.as_deref())?;
    Ok(Json(json!({ "member": body.member, "value": body.value })))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/meta/characteristics", get(list).post(create))
        .route("/api/meta/characteristics/{code}", put(update).delete(remove))
        .route(
            "/api/meta/characteristics/{code}/attributes",
            post(add_attr),
        )
        .route(
            "/api/meta/characteristics/{code}/attributes/{name}",
            delete(remove_attr),
        )
        .route(
            "/api/meta/characteristics/{code}/values",
            get(values_list).post(values_create),
        )
        .route(
            "/api/meta/characteristics/{code}/values/{value}",
            put(values_update).delete(values_delete),
        )
        .route(
            "/api/meta/characteristics/{code}/assign",
            put(assign_handler),
        )
        .route(
            "/api/meta/characteristics/{code}/rename",
            post(rename_char_handler),
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
        let cid = id_of(&con, "comportement").unwrap();
        let vtable = value_table(cid);
        assert!(table_exists(&con, &vtable), "table de valeurs créée");
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
        // Après étape 9 B1, la colonne physique est c{attr_id}, pas le nom de l'attribut.
        let phys_col = attr_col_for(&con, "comportement", "compte_destination")
            .expect("attr_col_for doit trouver la colonne");
        assert!(
            col_exists(&con, &vtable, &phys_col),
            "colonne d'attribut N2 sur car_<id>"
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

        let cid = id_of(&con, "comportement").unwrap();
        let vtable = value_table(cid); // "car_1"

        let refs = references::dynamic_references(&con);
        // N1 : dim_account.comportement → car_<id>.code
        assert!(refs.iter().any(|r| r.table == "dim_account"
            && r.column == "comportement"
            && r.target_table == vtable
            && r.target_column == "code"));
        // N2 : car_<id>.c{attr_id} → dim_account.code (colonne physique B1 étape 9)
        let col_cd = attr_col_for(&con, "comportement", "compte_destination").unwrap();
        assert!(
            refs.iter().any(|r| r.table == vtable
                && r.column == col_cd
                && r.target_table == "dim_account"
                && r.target_column == "code"),
            "N2 compte_destination ({col_cd}) dans le graphe"
        );
        let col_nat = attr_col_for(&con, "comportement", "nat").unwrap();
        assert!(
            refs.iter().any(|r| r.table == vtable
                && r.column == col_nat
                && r.target_table == "dim_nature"
                && r.target_column == "code"),
            "N2 nat ({col_nat}) dans le graphe"
        );
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
        let cid = id_of(&con, "comportement").unwrap();
        let vtable = value_table(cid);

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
            table_exists(&con, &vtable),
            "table de valeurs survit au reset"
        );
        let phys_col = attr_col_for(&con, "comportement", "compte_destination").unwrap();
        assert!(
            col_exists(&con, &vtable, &phys_col),
            "colonne d'attribut survit au reset"
        );
    }

    #[test]
    fn valeurs_validation_et_affectation() {
        let con = setup();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('471L', 'Liaison', 'bilan')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe) VALUES ('700', 'Ventes', 'resultat')",
            [],
        )
        .unwrap();
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        add_attribute(&con, "comportement", "compte_destination", "L", "account").unwrap();

        // Valeur N1 avec attribut valide.
        let mut obj = Map::new();
        obj.insert("code".into(), json!("VENTES_IC"));
        obj.insert("libelle".into(), json!("Ventes interco"));
        obj.insert("compte_destination".into(), json!("471L"));
        create_value(&con, "comportement", &obj).unwrap();

        // Attribut pointant vers un compte inexistant → rejeté.
        let mut bad = Map::new();
        bad.insert("code".into(), json!("X"));
        bad.insert("compte_destination".into(), json!("INEXISTANT"));
        assert!(
            create_value(&con, "comportement", &bad).is_err(),
            "valeur d'attribut hors master data rejetée"
        );

        // Champ inconnu → rejeté.
        let mut unknown = Map::new();
        unknown.insert("code".into(), json!("Y"));
        unknown.insert("foo".into(), json!("bar"));
        assert!(create_value(&con, "comportement", &unknown).is_err());

        let vals = list_values(&con, "comportement").unwrap();
        assert_eq!(vals.len(), 1, "une seule valeur créée");

        // Affectation d'un compte à la valeur.
        assign(&con, "comportement", "700", Some("VENTES_IC")).unwrap();
        let assigned: Option<String> = con
            .query_row(
                "SELECT comportement FROM dim_account WHERE code = '700'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(assigned.as_deref(), Some("VENTES_IC"));

        // Affecter une valeur inexistante → rejeté.
        assert!(assign(&con, "comportement", "700", Some("NOPE")).is_err());
        // Affecter à un membre inexistant → rejeté.
        assert!(assign(&con, "comportement", "999", Some("VENTES_IC")).is_err());

        // Désaffectation (value = None).
        assign(&con, "comportement", "700", None).unwrap();
        let after: Option<String> = con
            .query_row(
                "SELECT comportement FROM dim_account WHERE code = '700'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, None, "compte déclassé");
    }

    #[test]
    fn suppression_nettoie_artefacts() {
        let con = setup();
        create_characteristic(&con, "comportement", "C", "account").unwrap();
        add_attribute(&con, "comportement", "compte_destination", "L", "account").unwrap();
        let cid = id_of(&con, "comportement").unwrap();
        let vtable = value_table(cid);

        delete_characteristic(&con, "comportement").unwrap();
        assert!(!table_exists(&con, &vtable), "table supprimée");
        assert!(
            !col_exists(&con, "dim_account", "comportement"),
            "colonne de rattachement retirée"
        );
        let n: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM dim_characteristic_attribute",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "attributs N2 supprimés avec la N1");
    }
}
