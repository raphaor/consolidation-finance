//! Injection des écritures par préfixe de nature (staging).
//!
//! Les écritures dont le préfixe de nature est `2`, `3` ou `4` sont injectées
//! directement au niveau correspondant du pipeline, en sautant les étapes
//! précédentes. Voir `docs/FLUX_CONSO.md` « Staging — Injection par nature ».
//!
//! - Préfixe `2` → injecté au niveau `reclassified` (saute A + B)
//! - Préfixe `3` → injecté au niveau `converted` (saute A + B + C)
//! - Préfixe `4` → injecté au niveau `consolidated` (saute A + B + C + D)
//!
//! Les écritures injectées sont supposées déjà traitées pour les étapes
//! qu'elles sautent. L'agrégation se fait par le grain standard.

use duckdb::Connection;

/// Insère dans `fact_entry` au niveau `level` les écritures de `stg_entry`
/// dont le préfixe de nature correspond à `prefix` (un seul caractère '2'/'3'/'4').
///
/// L'agrégation se fait par le grain standard (scenario, entity, entry_period,
/// period, account, flow, currency, nature, partner).
///
/// Renvoie le nombre de lignes produites à ce niveau pour ce préfixe.
pub fn inject_by_prefix(con: &Connection, level: &str, prefix: &str) -> duckdb::Result<usize> {
    let sql = format!(
        "INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2, level, amount)
         SELECT scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2,
                '{level}' AS level,
                SUM(amount) AS amount
         FROM stg_entry
         WHERE substr(nature, 1, 1) = '{prefix}'
         GROUP BY scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2"
    );
    con.execute(&sql, [])?;
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND substr(nature, 1, 1) = ?",
        [level, prefix],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
