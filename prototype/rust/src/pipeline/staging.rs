//! Injection des écritures par préfixe de nature (staging).
//!
//! Les écritures dont le préfixe de nature est `3` ou `4` sont injectées
//! directement au niveau correspondant du pipeline, en sautant les étapes
//! précédentes. Voir `docs/FLUX_CONSO.md` « Staging — Injection par nature ».
//!
//! - Préfixe `3` → injecté au niveau `converted` (saute A + C)
//! - Préfixe `4` → injecté au niveau `consolidated` (saute A + C + D)
//!
//! Le préfixe `2` n'est **plus routé** depuis la suppression du niveau
//! `reclassified` (cf. docs/A_NOUVEAU.md §4). Sa redéfinition cible (injection au
//! `converted`, en devise fonctionnelle, avant écarts) est prévue en Phase 4
//! (cf. docs/A_NOUVEAU.md §4 bis) — non implémentée ici.
//!
//! Les écritures injectées sont supposées déjà traitées pour les étapes
//! qu'elles sautent. L'agrégation se fait par le grain standard.

use crate::dimensions;
use duckdb::Connection;

/// Insère dans `fact_entry` au niveau `level` les écritures de `stg_entry`
/// dont le préfixe de nature correspond à `prefix` (un seul caractère '2'/'3'/'4').
///
/// L'agrégation se fait par le grain complet des dimensions propagées
/// (built-in + customs), généré dynamiquement depuis le registre.
///
/// Renvoie le nombre de lignes produites à ce niveau pour ce préfixe.
pub fn inject_by_prefix(con: &Connection, level: &str, prefix: &str) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, level, amount)\n\
         SELECT {col_list},\n\
                '{level}' AS level,\n\
                SUM(amount) AS amount\n\
         FROM stg_entry\n\
         WHERE substr(nature, 1, 1) = '{prefix}'\n\
         GROUP BY {col_list}"
    );
    con.execute(&sql, [])?;
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND substr(nature, 1, 1) = ?",
        [level, prefix],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
