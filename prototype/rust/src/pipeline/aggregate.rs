//! Étape A — Agrégation (→ niveau `corporate`).
//!
//! Miroir de `conso/pipeline.py::step_a_aggregate`.
//!
//! Cumul des écritures source par entité. Lit la saisie brute (`stg_entry`),
//! agrège par (scenario, entity, entry_period, period, account, flow, currency,
//! nature, partner) et stocke au niveau *corporate* (en devise fonctionnelle).
//! La nature fait partie du grain d'agrégation : deux écritures de natures
//! différentes ne sont jamais agrégées. La dimension `partner` est également
//! préservée au grain : deux écritures interco sur des partenaires distincts
//! restent séparées (nécessaire pour les règles d'élimination interco).
//!
//! **Staging par nature** : seules les écritures de préfixe `0` ou `1` passent
//! par l'étape A. Les préfixes `2`, `3`, `4` sont injectés directement à leur
//! niveau cible par le module `staging`. Voir `docs/FLUX_CONSO.md` « Staging ».
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
    (scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2, level, amount)
SELECT
    scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2,
    'corporate' AS level,
    SUM(amount) AS amount
FROM stg_entry
WHERE substr(nature, 1, 1) IN ('0', '1')
GROUP BY scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2;";

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Renvoie le nombre de lignes produites au niveau `corporate`.
pub fn step_a(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_A, [])?;
    count_level(con, "corporate")
}
