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
//! proportionnelle). La clôture consolidée (portée) est ensuite écrasée par
//! [`super::materialize_closures`] au niveau consolidated, qui la reconstruit
//! depuis les constituants consolidés (même valeur, mais autoritaire).

use super::count_level;
use crate::dimensions;
use duckdb::Connection;

/// Exécute l'étape D : applique la méthode d'intégration de chaque entité.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Pour
/// les 12 colonnes built-in, le SQL produit reste structurellement identique
/// au SQL statique historique (test golden inchangé).
///
/// Renvoie le nombre de lignes produites au niveau `consolidated`.
pub fn step_d(con: &Connection) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let f_cols = cols
        .iter()
        .map(|c| format!("f.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let col_list = cols.join(", ");

    let sql = format!(
        "\
INSERT INTO fact_entry\n\
    ({col_list}, level, amount)\n\
SELECT\n\
    {f_cols},\n\
    'consolidated' AS level,\n\
    f.amount * COALESCE(p.pct_integration, 1.0) AS amount\n\
FROM fact_entry f\n\
JOIN sat_perimeter p\n\
  ON p.entity = f.entity\n\
 AND p.scenario = f.scenario\n\
 AND p.period = f.entry_period\n\
WHERE f.level = 'converted'\n\
  AND p.methode IN ('globale', 'proportionnelle');  -- équivalence hors MVP"
    );
    con.execute(&sql, [])?;
    count_level(con, "consolidated")
}
