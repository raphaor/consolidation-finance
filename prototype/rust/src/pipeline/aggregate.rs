//! Étape A — Agrégation (→ niveau `corporate`).
//!
//! Miroir de `conso/pipeline.py::step_a_aggregate`.
//!
//! Cumul des écritures source par entité. Lit la saisie brute (`stg_entry`),
//! agrège par (scenario, entity, entry_period, period, account, flow, currency)
//! et stocke au niveau *corporate* (en devise fonctionnelle).
//!
//! Aucun filtre sur les flux : la saisie (mode écriture ou formulaire bilan)
//! est agrégée telle quelle. En mode écriture, les liasses ne contiennent que
//! F00/F20 ; en mode bilan, le F99 (clôture) saisi sera agrégé ici puis
//! reconstruit/écrasé plus loin par `materialize_closures` à chaque niveau de
//! stockage. La validité des flux saisis relève du formulaire d'entrée, pas de
//! cette étape.

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
GROUP BY scenario, entity, entry_period, period, account, flow, currency;";

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Renvoie le nombre de lignes produites au niveau `corporate`.
pub fn step_a(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_A, [])?;
    count_level(con, "corporate")
}
