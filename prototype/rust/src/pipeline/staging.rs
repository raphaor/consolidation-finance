//! Injection des écritures par préfixe de nature (staging).
//!
//! Staging cible (cf. docs/A_NOUVEAU.md §4 bis) — 3 niveaux :
//!
//! - Préfixe `2` → **converti**, en devise fonctionnelle, **avant** écarts :
//!   consommé directement dans `convert::step_c` (UNION), pour subir conversion +
//!   écarts. **Pas** via ce module.
//! - Préfixe `3` → **consolidé**, **avant** le × pct : consommé dans
//!   `consolidate::step_d` (UNION), pour subir le × pct_integration. **Pas** via
//!   ce module.
//! - Préfixe `4` → **consolidé**, **après** le × pct : injecté **tel quel** par
//!   [`inject_by_prefix`] après l'étape D (seul préfixe routé par ce module).
//!
//! Les écritures injectées sont supposées déjà traitées pour les étapes
//! qu'elles sautent. L'agrégation se fait par le grain standard.

use super::ConvertParams;
use crate::dimensions;
use duckdb::{params, Connection};

/// Insère dans `fact_entry` au niveau `level` les écritures de `stg_entry`
/// dont le préfixe de nature correspond à `prefix` (un seul caractère '2'/'3'/'4').
///
/// Ne reprend que la **remontée du run** (`stg_entry.phase = p.phase AND
/// stg_entry.entry_period = p.exercice`) et tague les lignes avec
/// `p.consolidation_id` (isolation du run dans `fact_entry`).
///
/// L'agrégation se fait par le grain complet des dimensions propagées
/// (built-in + customs), généré dynamiquement depuis le registre.
///
/// Renvoie le nombre de lignes produites à ce niveau pour ce préfixe.
pub fn inject_by_prefix(
    con: &Connection,
    p: &ConvertParams,
    level: &str,
    prefix: &str,
) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, consolidation_id, level, amount)\n\
         SELECT {col_list},\n\
                ? AS consolidation_id,\n\
                '{level}' AS level,\n\
                SUM(amount) AS amount\n\
         FROM stg_entry\n\
         WHERE substr(nature, 1, 1) = '{prefix}'\n\
           AND phase = ?\n\
           AND entry_period = ?\n\
         GROUP BY {col_list}"
    );
    con.execute(&sql, params![p.consolidation_id, p.phase, p.exercice])?;
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND substr(nature, 1, 1) = ?",
        [level, prefix],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
