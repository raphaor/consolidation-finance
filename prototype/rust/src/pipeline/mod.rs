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
pub mod materialize_f99;
pub mod reclassify;

use duckdb::Connection;
use std::time::Instant;

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
///
/// Après les étapes B et D, on materialise le flux de clôture F99 (= somme des
/// autres flux) afin que le validateur [`crate::validate`] puisse comparer le
/// F99 stocké à la somme des flux constitutifs.
pub fn run_pipeline(
    con: &Connection,
    params: &ConvertParams,
) -> duckdb::Result<LevelCounts> {
    let corporate = aggregate::step_a(con)?;
    let reclassified = {
        let n = reclassify::step_b(con)?;
        materialize_f99::materialize_f99(con, "reclassified")?;
        n + count_f99(con, "reclassified")?
    };
    let converted = convert::step_c(con, params)?;
    let consolidated = {
        let n = consolidate::step_d(con)?;
        materialize_f99::materialize_f99(con, "consolidated")?;
        n + count_f99(con, "consolidated")?
    };
    Ok([corporate, reclassified, converted, consolidated])
}

/// Temps d'exécution mesuré pour une étape du pipeline.
#[derive(Debug, Clone)]
pub struct StepTiming {
    /// Niveau de stockage produit (`corporate`, `reclassified`, …).
    pub level: &'static str,
    /// Nombre de lignes produites à ce niveau.
    pub rows: usize,
    /// Durée de l'étape, en millisecondes.
    pub ms: f64,
}

/// Rapport d'exécution du pipeline avec timings par étape.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    /// Une entrée par étape, dans l'ordre A→B→C→D.
    pub steps: [StepTiming; 4],
    /// Durée totale A→D (wall-clock), en millisecondes.
    pub total_ms: f64,
}

impl PipelineReport {
    /// Nombre de lignes par niveau `[corporate, reclassified, converted, consolidated]`.
    pub fn counts(&self) -> LevelCounts {
        [
            self.steps[0].rows,
            self.steps[1].rows,
            self.steps[2].rows,
            self.steps[3].rows,
        ]
    }

    /// Durée totale en secondes.
    pub fn total_sec(&self) -> f64 {
        self.total_ms / 1000.0
    }
}

/// Variante de [`run_pipeline`] instrumentée : mêmes effets, renvoie en plus
/// la durée (wall-clock) de chaque étape.
///
/// Pensée pour le benchmark gros volumes — ne change rien à la logique.
pub fn run_pipeline_timed(
    con: &Connection,
    params: &ConvertParams,
) -> duckdb::Result<PipelineReport> {
    let wall = Instant::now();

    let t = Instant::now();
    let corporate = aggregate::step_a(con)?;
    let ms_a = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let reclassified = {
        let n = reclassify::step_b(con)?;
        materialize_f99::materialize_f99(con, "reclassified")?;
        n + count_f99(con, "reclassified")?
    };
    let ms_b = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let converted = convert::step_c(con, params)?;
    let ms_c = t.elapsed().as_secs_f64() * 1000.0;

    let t = Instant::now();
    let consolidated = {
        let n = consolidate::step_d(con)?;
        materialize_f99::materialize_f99(con, "consolidated")?;
        n + count_f99(con, "consolidated")?
    };
    let ms_d = t.elapsed().as_secs_f64() * 1000.0;

    let total_ms = wall.elapsed().as_secs_f64() * 1000.0;

    Ok(PipelineReport {
        steps: [
            StepTiming { level: "corporate", rows: corporate, ms: ms_a },
            StepTiming { level: "reclassified", rows: reclassified, ms: ms_b },
            StepTiming { level: "converted", rows: converted, ms: ms_c },
            StepTiming { level: "consolidated", rows: consolidated, ms: ms_d },
        ],
        total_ms,
    })
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

/// Nombre de lignes F99 materialisées à un niveau donné.
fn count_f99(con: &Connection, level: &str) -> duckdb::Result<usize> {
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND flow = 'F99'",
        [level],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
