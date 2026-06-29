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
use crate::{dimensions, references};
use duckdb::{params, Connection};

/// Insère dans `fact_entry` au niveau `level` les écritures de `stg_entry`
/// dont le préfixe de nature correspond à `prefix` (un seul caractère '2'/'3'/'4').
///
/// Ne reprend que la **remontée du run** (`stg_entry.phase = p.phase AND
/// stg_entry.entry_period = p.exercice`) et tague les lignes avec
/// `p.consolidation_id` (isolation du run dans `fact_entry`).
///
/// L'agrégation se fait par le grain complet des dimensions propagées
/// (built-in + customs), généré dynamiquement depuis le registre. Les codes
/// TEXT de `stg_entry` sont résolus en INTEGER ids pour les 10 colonnes
/// id-typées de `fact_entry` via JOIN sur la master data.
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

    // Résolution code→id pour les dims id-typées (stg_entry TEXT → fact_entry INTEGER).
    // La résolution est isolée dans une sous-requête : l'agrégation (GROUP BY +
    // SUM) opère ensuite sur des noms de colonnes propres, sans collision entre
    // la colonne brute `stg_entry.phase` (code) et l'alias `_dphase.id AS phase`
    // — collision qui, sur un GROUP BY par alias, laisse l'`id` ni groupé ni
    // agrégé (cf. même schéma que `aggregate.rs`).
    let mut id_joins = String::new();
    let mut inner_exprs: Vec<String> = Vec::new();
    for dim in &dims {
        let name = &dim.name;
        if let Some((table, code_col)) = references::dimension_master(name) {
            let alias = format!("_d{name}");
            let join_type = if dim.nullable() { "LEFT JOIN" } else { "JOIN" };
            id_joins.push_str(&format!(
                "\n{join_type} {table} {alias} ON {alias}.{code_col} = s.{name}"
            ));
            inner_exprs.push(format!("{alias}.id AS {name}"));
        } else {
            inner_exprs.push(format!("s.{name}"));
        }
    }
    let inner_list = inner_exprs.join(",\n        ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, consolidation_id, level, amount)\n\
         SELECT\n\
             {col_list},\n\
             ? AS consolidation_id,\n\
             '{level}' AS level,\n\
             SUM(amount) AS amount\n\
         FROM (\n\
           SELECT\n\
             {inner_list},\n\
             s.amount AS amount\n\
           FROM stg_entry s\n\
           {id_joins}\n\
           WHERE substr(s.nature, 1, 1) = '{prefix}'\n\
             AND s.phase = ?\n\
             AND s.entry_period = ?\n\
         ) t\n\
         GROUP BY {col_list}"
    );
    con.execute(&sql, params![p.consolidation_id, p.phase, p.exercice])?;
    // Compte par JOIN sur dim_nature (nature est INTEGER dans fact_entry).
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry f \
         JOIN dim_nature n ON n.id = f.nature \
         WHERE f.level = ? AND substr(n.code, 1, 1) = ?",
        [level, prefix],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
