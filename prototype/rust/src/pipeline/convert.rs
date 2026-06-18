//! Étape C — Conversion multi-devises (→ niveau `converted`).
//!
//! Miroir de `conso/pipeline.py::step_c_convert`.
//!
//! Pour chaque ligne reclassifiée en devise ≠ présentation :
//!   1. taux du flux via `dim_flow.taux_conversion` :
//!        - `close_n1` → taux_close N-1
//!        - `avg`      → taux_moyen N
//!        - `close_n`  → taux_close N
//!        - `terminal` → taux_close N (écart propre = 0)
//!   2. montant converti = `amount × taux`
//!   3. écart = `amount × (taux_close_N − taux_du_flux)`, posté sur `flux_ecart`
//!   4. lignes en devise de présentation : copie directe, aucun écart
//!
//! Le niveau *converted* est exprimé en devise de présentation.
//!
//! Tous les flux sont convertis, **clôtures (F99) comprises** : une clôture est
//! convertie à son taux (F99 → `close_n`), sans écart (`flux_ecart` NULL). La
//! clôture convertie (portée) est ensuite écrasée par `materialize_closures`
//! au niveau converted, qui la reconstruit depuis les constituants convertis +
//! écarts (même valeur, mais autoritaire).

use super::{count_level, ConvertParams};
use crate::dimensions;
use duckdb::params;
use duckdb::Connection;

/// Exécute l'étape C : convertit les écritures en devise de présentation et
/// génère les écarts (F80 / F81).
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Pour
/// les 12 colonnes built-in, le SQL produit reste structurellement identique
/// au SQL statique historique (test golden inchangé).
///
/// # Paramètres `?` (ordre)
///
/// Le SQL généré conserve exactement 7 paramètres `?`, dans l'ordre :
/// 1. `presentation_currency` (taux_flux)
/// 2. `presentation_currency` (taux_close_n)
/// 3. `current_period`  (join r_n)
/// 4. `prev_period`     (join r_n1)
/// 5. `presentation_currency` (currency convertie)
/// 6. `presentation_currency` (currency écart)
/// 7. `presentation_currency` (filtre écart)
///
/// Renvoie le nombre de lignes produites au niveau `converted`.
pub fn step_c(con: &Connection, p: &ConvertParams) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);

    // CTE `conv` : sélectionne toutes les colonnes propagées + `amount` et
    // les colonnes techniques (taux_flux, taux_close_n, flux_ecart).
    let f_cols = cols
        .iter()
        .map(|c| format!("f.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    // SELECT final branche 1 (lignes converties) : hérite tout sauf `currency`
    // (forcée à `?` = devise de présentation).
    let final_cols_convert = cols
        .iter()
        .map(|c| if *c == "currency" { "?".to_string() } else { (*c).to_string() })
        .collect::<Vec<_>>()
        .join(", ");
    // SELECT final branche 2 (écarts) : comme branche 1 + `flow` remplacé par
    // `flux_ecart` (flux d'écart associé).
    let final_cols_ecart = cols
        .iter()
        .map(|c| {
            if *c == "currency" {
                "?".to_string()
            } else if *c == "flow" {
                "flux_ecart".to_string()
            } else {
                (*c).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // INSERT INTO fact_entry (toutes les cols propagées, level, amount)
    let insert_col_list = cols.join(", ");

    let sql = format!(
        "\
WITH conv AS (\n\
    SELECT\n\
        {f_cols}, f.amount,\n\
        fl.taux_conversion,\n\
        fl.flux_ecart,\n\
        -- Taux applicable au flux (1.0 si déjà en devise de présentation)\n\
        CASE\n\
            WHEN f.currency = ? THEN 1.0\n\
            WHEN fl.taux_conversion = 'close_n1' THEN r_n1.taux_close\n\
            WHEN fl.taux_conversion = 'avg'      THEN r_n.taux_moyen\n\
            WHEN fl.taux_conversion IN ('close_n', 'terminal')\n\
                THEN r_n.taux_close\n\
        END AS taux_flux,\n\
        -- Taux de clôture N (référence pour le calcul d'écart)\n\
        CASE\n\
            WHEN f.currency = ? THEN 1.0\n\
            ELSE r_n.taux_close\n\
        END AS taux_close_n\n\
    FROM fact_entry f\n\
    JOIN dim_flow fl ON fl.code = f.flow\n\
    LEFT JOIN sat_exchange_rate r_n\n\
           ON r_n.currency_source = f.currency\n\
          AND r_n.period = ?\n\
    LEFT JOIN sat_exchange_rate r_n1\n\
           ON r_n1.currency_source = f.currency\n\
          AND r_n1.period = ?\n\
    WHERE f.level = 'reclassified'\n\
)\n\
INSERT INTO fact_entry\n\
    ({insert_col_list}, level, amount)\n\
-- Montants convertis (tous flux constitutifs, exprimés en devise de présentation)\n\
SELECT {final_cols_convert},\n\
       'converted' AS level,\n\
       amount * taux_flux AS amount\n\
FROM conv\n\
UNION ALL\n\
-- Lignes d'écart (devise ≠ présentation, flux porteur d'un flux_ecart, écart ≠ 0)\n\
-- L'écart F80/F81 hérite de la nature — et du partner — du flux parent.\n\
SELECT {final_cols_ecart},\n\
       'converted' AS level,\n\
       amount * (taux_close_n - taux_flux) AS amount\n\
FROM conv\n\
WHERE currency <> ?\n\
  AND flux_ecart IS NOT NULL\n\
  AND ABS(amount * (taux_close_n - taux_flux)) >= 0.005;"
    );
    con.execute(
        &sql,
        params![
            p.presentation_currency,
            p.presentation_currency,
            p.current_period,
            p.prev_period,
            p.presentation_currency,
            p.presentation_currency,
            p.presentation_currency,
        ],
    )?;
    count_level(con, "converted")
}
