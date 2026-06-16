//! Étape transversale — Reconstruction du flux de clôture F99.
//!
//! F99 n'est jamais saisi (cf. docs/FLUX_CONSO.md §3) : c'est un solde
//! reconstruit par l'identité `F99 = F00 + F01 + F20 + F80 + F81 + F98`.
//!
//! On materialise ce solde **en base** (insertion d'une ligne `flow = 'F99'`
//! par combinaison account/entity/devise) à chaque niveau où l'on valide
//! l'identité : `reclassified` (devise fonctionnelle, écarts = 0) et
//! `consolidated` (devise de présentation, écarts inclus).
//!
//! Avoir F99 stocké (et non seulement reconstruit à la lecture) permet au
//! validateur [`crate::validate`] de comparer le F99 stocké à la somme des
//! flux constitutifs et de détecter toute incohérence (pipeline cassé,
//! écriture manuelle abusive, etc.).

use duckdb::Connection;

/// SQL de reconstruction de F99 pour un niveau de stockage.
///
/// `flow <> 'F99'` : on exclut le F99 lui-même (au cas où la table contiendrait
/// déjà des F99 d'un run précédent — bien que `run_pipeline` soit appelé après
/// un `DELETE FROM fact_entry`, l'identité reste valable en ré-exécution).
const SQL_F99: &str = "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, level, amount)
SELECT scenario, entity, entry_period, period, account,
       'F99' AS flow, currency,
       ?     AS level,
       SUM(amount) AS amount
FROM fact_entry
WHERE level = ? AND flow <> 'F99'
GROUP BY scenario, entity, entry_period, period, account, currency;";

/// Materialise F99 = Σ(flux constitutifs) au niveau donné.
///
/// À appeler après chaque étape dont on veut valider l'identité :
/// `reclassified` (après step B) et `consolidated` (après step D).
/// Renvoie le nombre de lignes F99 insérées.
pub fn materialize_f99(con: &Connection, level: &str) -> duckdb::Result<usize> {
    con.execute(SQL_F99, [level, level])?;
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND flow = 'F99'",
        [level],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
