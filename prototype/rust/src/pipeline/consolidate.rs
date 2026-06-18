//! Étape D — Consolidation (→ niveau `consolidated`).
//!
//! Miroir de `conso/python/pipeline.py::step_d_consolidate`.
//!
//! Application des méthodes de consolidation (natif MVP) :
//!   - globale         : copie à 100 % (`pct_integration = 1.0`)
//!   - proportionnelle : `amount × pct_integration`
//!   - équivalence     : EXCLUE du MVP (non traitée)
//!
//! **NB** : tous les flux sont consolidés, **clôtures (F99) comprises** : le
//! `pct_integration` est appliqué à la clôture (indispensable pour la méthode
//! proportionnelle). La clôtre consolidée (portée) est ensuite écrasée par
//! [`super::materialize_closures`] au niveau consolidated, qui la reconstruit
//! depuis les constituants consolidés (même valeur, mais autoritaire).

use super::count_level;
use duckdb::Connection;

/// SQL de la consolidation (application des méthodes).
const SQL_STEP_D: &str = "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2, level, amount)
SELECT
    f.scenario, f.entity, f.entry_period, f.period, f.account, f.flow, f.currency, f.nature, f.partner, f.share, f.analysis, f.analysis2,
    'consolidated' AS level,
    f.amount * COALESCE(p.pct_integration, 1.0) AS amount
FROM fact_entry f
JOIN sat_perimeter p
  ON p.entity = f.entity
 AND p.scenario = f.scenario
 AND p.period = f.entry_period
WHERE f.level = 'converted'
  AND p.methode IN ('globale', 'proportionnelle');  -- équivalence hors MVP";

/// Exécute l'étape D : applique la méthode d'intégration de chaque entité.
///
/// Renvoie le nombre de lignes produites au niveau `consolidated`.
pub fn step_d(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_D, [])?;
    count_level(con, "consolidated")
}
