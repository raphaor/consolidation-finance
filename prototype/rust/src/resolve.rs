//! Résolution **code ↔ id** des dimensions — **étape 2** du chantier « codes
//! renommables » (option B1, cf. `docs/PLAN_RENOMMAGE_CODES.md`).
//!
//! Socle commun des étapes 3–4 (bascule des FK puis de `fact_entry` vers les
//! `id`). Deux directions :
//!
//! - **écriture** : un code entrant (API, import CSV, sauvegarde de règle) est
//!   traduit en `id` avant insertion ([`resolve_id`], [`code_to_id_map`]) ;
//! - **lecture** : un `id` stocké est re-projeté en code pour l'affichage
//!   ([`code_of`], [`id_to_code_map`]).
//!
//! # Performance
//!
//! Les versions **batch** ([`code_to_id_map`] / [`id_to_code_map`]) chargent la
//! dimension entière en une requête : à utiliser à l'import et aux reports (jamais
//! une requête par ligne). Les versions unitaires servent les cas ponctuels
//! (validation d'une écriture, résolution d'un paramètre).
//!
//! # Sécurité
//!
//! `table` et `code_col` proviennent **toujours** du registre statique
//! ([`crate::surrogate::SURROGATE_DIMS`]) — jamais de l'entrée utilisateur — donc
//! interpolables sans risque ; les valeurs passent par des `?` paramétrés.
//! [`master_of`] garantit ce contrat en n'acceptant qu'une table connue.

use duckdb::Connection;
use std::collections::HashMap;

/// `(table, colonne_code)` d'une dimension dotée d'un `id`, par **nom de table**.
/// Source : [`crate::surrogate::SURROGATE_DIMS`]. `None` si la table n'est pas une
/// dimension à clé technique (garde-fou contre toute interpolation arbitraire).
pub fn master_of(table: &str) -> Option<(&'static str, &'static str)> {
    crate::surrogate::SURROGATE_DIMS
        .iter()
        .find(|(t, _, _)| *t == table)
        .map(|(t, code_col, _)| (*t, *code_col))
}

/// Résout `code → id` pour une dimension. `Ok(None)` si le code est absent.
///
/// `table` doit être une dimension connue ([`master_of`]) — sinon
/// [`Error::InvalidParameterName`] est renvoyée (refus de toute table inconnue).
pub fn resolve_id(con: &Connection, table: &str, code: &str) -> duckdb::Result<Option<i64>> {
    let (t, code_col) = require_master(table)?;
    opt_row(con.query_row(
        &format!("SELECT id FROM {t} WHERE \"{code_col}\" = ?"),
        [code],
        |r| r.get::<_, i64>(0),
    ))
}

/// Résout `id → code` pour une dimension. `Ok(None)` si l'id est absent.
pub fn code_of(con: &Connection, table: &str, id: i64) -> duckdb::Result<Option<String>> {
    let (t, code_col) = require_master(table)?;
    opt_row(con.query_row(
        &format!("SELECT \"{code_col}\" FROM {t} WHERE id = ?"),
        [id],
        |r| r.get::<_, String>(0),
    ))
}

/// Carte `code → id` de toute la dimension (une seule requête). Pour l'import et
/// toute traduction en masse.
pub fn code_to_id_map(con: &Connection, table: &str) -> duckdb::Result<HashMap<String, i64>> {
    let (t, code_col) = require_master(table)?;
    let mut stmt = con.prepare(&format!("SELECT \"{code_col}\", id FROM {t}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    rows.collect()
}

/// Carte `id → code` de toute la dimension (une seule requête). Pour les reports
/// et la re-projection des `id` en codes.
pub fn id_to_code_map(con: &Connection, table: &str) -> duckdb::Result<HashMap<i64, String>> {
    let (t, code_col) = require_master(table)?;
    let mut stmt = con.prepare(&format!("SELECT id, \"{code_col}\" FROM {t}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

/// Variante de [`master_of`] qui échoue (au lieu de `None`) sur une table
/// inconnue : garantit qu'aucun identifiant non whitelisté n'est interpolé.
fn require_master(table: &str) -> duckdb::Result<(&'static str, &'static str)> {
    master_of(table).ok_or(duckdb::Error::InvalidParameterName(format!(
        "table sans clé technique (hors SURROGATE_DIMS) : {table}"
    )))
}

/// Convertit `QueryReturnedNoRows` en `Ok(None)`, propage les autres erreurs.
fn opt_row<T>(res: duckdb::Result<T>) -> duckdb::Result<Option<T>> {
    match res {
        Ok(v) => Ok(Some(v)),
        Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::create_schema;

    fn setup() -> Connection {
        let con = Connection::open_in_memory().expect("open_in_memory");
        create_schema(&con).expect("create_schema");
        crate::seed_all(&con).expect("seed_all");
        con
    }

    #[test]
    fn resolve_id_et_code_of_sont_reciproques() {
        let con = setup();
        // dim_entity / code 'M' (entité mère seedée).
        let id = resolve_id(&con, "dim_entity", "M").unwrap().expect("M existe");
        let code = code_of(&con, "dim_entity", id).unwrap().expect("id existe");
        assert_eq!(code, "M");
    }

    #[test]
    fn resolve_id_inconnu_renvoie_none() {
        let con = setup();
        assert!(resolve_id(&con, "dim_entity", "ZZZ").unwrap().is_none());
        assert!(code_of(&con, "dim_entity", 999_999).unwrap().is_none());
    }

    #[test]
    fn devise_utilise_sa_colonne_code_iso() {
        let con = setup();
        // dim_currency a pour colonne de code `code_iso` (pas `code`).
        let id = resolve_id(&con, "dim_currency", "EUR")
            .unwrap()
            .expect("EUR existe");
        assert_eq!(code_of(&con, "dim_currency", id).unwrap().as_deref(), Some("EUR"));
    }

    #[test]
    fn cartes_batch_couvrent_toute_la_dimension_et_round_trip() {
        let con = setup();
        let c2i = code_to_id_map(&con, "dim_account").unwrap();
        let i2c = id_to_code_map(&con, "dim_account").unwrap();
        let total: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_account", [], |r| r.get(0))
            .unwrap();
        assert_eq!(c2i.len() as i64, total, "carte code→id complète");
        assert_eq!(i2c.len() as i64, total, "carte id→code complète");
        // Round-trip sur chaque entrée.
        for (code, id) in &c2i {
            assert_eq!(i2c.get(id), Some(code), "round-trip code→id→code pour {code}");
        }
    }

    #[test]
    fn table_inconnue_est_refusee() {
        let con = setup();
        // Refus d'une table hors registre (pas d'interpolation arbitraire).
        assert!(resolve_id(&con, "fact_entry; DROP TABLE", "x").is_err());
        assert!(master_of("table_bidon").is_none());
    }
}
