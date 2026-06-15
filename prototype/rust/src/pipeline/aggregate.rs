//! Étape A — Agrégation (→ niveau `corporate`).
//!
//! Miroir de `conso/pipeline.py::step_a_aggregate`.
//!
//! Cumul des écritures source par entité. Lit la saisie brute (`stg_entry`),
//! ne conserve que les flux sociaux d'origine (F00 et F20), agrège par
//! (scenario, entity, entry_period, period, account, flow, currency) et stocke
//! au niveau *corporate* (en devise fonctionnelle).

use super::count_level;
use duckdb::Connection;

/// SQL de l'agrégation corporate.
const SQL_STEP_A: &str = "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, level, amount)
SELECT
    scenario, entity, entry_period, period, account, flow, currency,
    'corporate' AS level,
    SUM(amount) AS amount
FROM stg_entry
WHERE flow IN ('F00', 'F20')
GROUP BY scenario, entity, entry_period, period, account, flow, currency;";

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Renvoie le nombre de lignes produites au niveau `corporate`.
pub fn step_a(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_A, [])?;
    count_level(con, "corporate")
}
