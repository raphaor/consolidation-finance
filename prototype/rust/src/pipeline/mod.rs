//! Pipeline de consolidation en 4 étapes.
//!
//! Miroir de `prototype/python/conso/pipeline.py`.
//!
//! Chaque étape lit un niveau de stockage et produit le suivant. L'ordre A→B→C→D
//! correspond à la correspondance stockage ↔ traitement décrite dans
//! `docs/FLUX_CONSO.md` :
//!
//! ```text
//! A. Agrégation      stg_entry        → fact_entry [corporate]
//! B. Reclassification corporate       → fact_entry [reclassified]
//! C. Conversion      reclassified     → fact_entry [converted]
//! D. Consolidation   converted        → fact_entry [consolidated]
//! ```
//!
//! Toute la logique est exprimée en SQL déclaratif (portage Rust direct via
//! duckdb-rs : une passe SQL par règle métier).

pub mod aggregate;
pub mod consolidate;
pub mod convert;
pub mod reclassify;

use duckdb::Connection;

/// Comptage des lignes par niveau de stockage après le pipeline.
pub type LevelCounts = [usize; 4];

/// Paramètres de la conversion multi-devises (étape C).
#[derive(Debug, Clone)]
pub struct ConvertParams {
    /// Devise de présentation (cible de la conversion).
    pub presentation_currency: String,
    /// Exercice courant N (pour les taux close_n / avg).
    pub current_period: String,
    /// Exercice précédent N-1 (pour le taux close_n1).
    pub prev_period: String,
}

impl Default for ConvertParams {
    fn default() -> Self {
        Self {
            presentation_currency: "EUR".to_string(),
            current_period: "2024".to_string(),
            prev_period: "2023".to_string(),
        }
    }
}

/// Enchaîne les 4 étapes et renvoie le nombre de lignes par niveau.
///
/// Miroir de `conso/pipeline.py::run_pipeline`.
///
/// Ordre des éléments de `LevelCounts` :
/// `[corporate, reclassified, converted, consolidated]`.
pub fn run_pipeline(
    con: &Connection,
    params: &ConvertParams,
) -> duckdb::Result<LevelCounts> {
    let corporate = aggregate::step_a(con)?;
    let reclassified = reclassify::step_b(con)?;
    let converted = convert::step_c(con, params)?;
    let consolidated = consolidate::step_d(con)?;
    Ok([corporate, reclassified, converted, consolidated])
}

/// Compte les lignes d'un niveau de stockage donné.
fn count_level(con: &Connection, level: &str) -> duckdb::Result<usize> {
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ?",
        [level],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
