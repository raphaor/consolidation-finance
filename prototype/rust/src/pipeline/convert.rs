//! Étape C — Conversion multi-devises (→ niveau `converted`).
//!
//! Miroir de `conso/pipeline.py::step_c_convert`.
//!
//! # Principe du cross-rate (SPEC_SCENARIO_V2.md §1)
//!
//! Les taux stockés dans `sat_exchange_rate` convertissent toute devise vers la
//! **devise pivot** applicative (`app_config.pivot_currency`). Pour passer en
//! devise de présentation, on calcule un cross-rate :
//!
//! ```text
//! taux_cross(fonctionnelle → présentation)
//!     = taux(fonctionnelle → pivot) / taux(présentation → pivot)
//! ```
//!
//! Cas particuliers (court-circuités dans le SQL) :
//! - `fonctionnelle = présentation` → taux = 1.0
//! - `présentation = pivot`         → taux_pres = 1.0, cross = taux_func
//!   (comportement historique EUR — invariant de nos tests golden)
//! - `fonctionnelle = pivot`        → taux_func = 1.0, cross = 1 / taux_pres
//!
//! Pour chaque ligne corporate en devise ≠ présentation :
//!   1. taux du flux via `dim_flow.taux_conversion` :
//!        - `close_n1` → taux_close N-1 (cross-rate)
//!        - `avg`      → taux_moyen N  (cross-rate)
//!        - `close_n`  → taux_close N  (cross-rate)
//!        - `terminal` → taux_close N  (écart propre = 0)
//!   2. montant converti = `amount × taux_flux`
//!   3. écart = `amount × (taux_close_n − taux_flux)`, posté sur `flux_ecart`
//!   4. lignes en devise de présentation : copie directe, aucun écart
//!
//! Le niveau *converted* est exprimé en devise de présentation. Tous les flux
//! sont convertis (clôtures F99 comprises) ; la clôture convertie (portée) est
//! ensuite écrasée par `materialize_closures` au niveau converted.

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
/// # Paramètres `?` (ordre, 9 au total)
///
/// | # | Valeur                       | Rôle                                       |
/// |---|------------------------------|--------------------------------------------|
/// | 1 | `presentation_currency`      | CTE `params.presentation`                  |
/// | 2 | `pivot_currency`             | CTE `params.pivot`                         |
/// | 3 | `rate_set`                   | CTE `params.rate_set`                      |
/// | 4 | `current_period`             | CTE `params.cur_period`                    |
/// | 5 | `prev_period`                | CTE `params.prev_period`                   |
/// | 6 | `scenario_code`              | CTE `conv` : `WHERE f.scenario = ?` (isolation) |
/// | 7 | `presentation_currency`      | `final_cols_convert` (colonne `currency`)  |
/// | 8 | `presentation_currency`      | `final_cols_ecart` (colonne `currency`)    |
/// | 9 | `presentation_currency`      | `WHERE currency <> ?` (filtre écart)       |
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
WITH params AS (\n\
    SELECT\n\
        ?::TEXT AS presentation,\n\
        ?::TEXT AS pivot,\n\
        ?::TEXT AS rate_set,\n\
        ?::TEXT AS cur_period,\n\
        ?::TEXT AS prev_period\n\
),\n\
conv AS (\n\
    SELECT\n\
        {f_cols}, f.amount,\n\
        fl.taux_conversion,\n\
        fl.flux_ecart,\n\
        -- Taux applicable au flux (cross-rate fonctionnelle -> présentation).\n\
        -- Court-circuit à 1.0 si la devise est déjà la présentation ; sinon\n\
        -- taux(fonctionnelle -> pivot) / taux(présentation -> pivot).\n\
        CASE\n\
            WHEN f.currency = p.presentation THEN 1.0\n\
            ELSE\n\
                (CASE\n\
                    WHEN f.currency = p.pivot THEN 1.0\n\
                    WHEN fl.taux_conversion = 'close_n1' THEN r_n1.taux_close\n\
                    WHEN fl.taux_conversion = 'avg'      THEN r_n.taux_moyen\n\
                    WHEN fl.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_n.taux_close\n\
                END)\n\
                /\n\
                (CASE\n\
                    WHEN p.presentation = p.pivot THEN 1.0\n\
                    WHEN fl.taux_conversion = 'close_n1' THEN r_pres_n1.taux_close\n\
                    WHEN fl.taux_conversion = 'avg'      THEN r_pres_n.taux_moyen\n\
                    WHEN fl.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_pres_n.taux_close\n\
                END)\n\
        END AS taux_flux,\n\
        -- Taux de clôture N (référence pour le calcul d'écart, cross-rate).\n\
        CASE\n\
            WHEN f.currency = p.presentation THEN 1.0\n\
            ELSE\n\
                (CASE WHEN f.currency = p.pivot THEN 1.0 ELSE r_n.taux_close END)\n\
                /\n\
                (CASE WHEN p.presentation = p.pivot THEN 1.0 ELSE r_pres_n.taux_close END)\n\
        END AS taux_close_n\n\
    FROM fact_entry f\n\
    JOIN dim_flow fl ON fl.code = f.flow\n\
    CROSS JOIN params p\n\
    -- Taux de la devise fonctionnelle vers le pivot (N et N-1)\n\
    LEFT JOIN sat_exchange_rate r_n\n\
           ON r_n.rate_set        = p.rate_set\n\
          AND r_n.currency_source = f.currency\n\
          AND r_n.period          = p.cur_period\n\
    LEFT JOIN sat_exchange_rate r_n1\n\
           ON r_n1.rate_set        = p.rate_set\n\
          AND r_n1.currency_source = f.currency\n\
          AND r_n1.period          = p.prev_period\n\
    -- Taux de la devise de présentation vers le pivot (N et N-1)\n\
    LEFT JOIN sat_exchange_rate r_pres_n\n\
           ON r_pres_n.rate_set        = p.rate_set\n\
          AND r_pres_n.currency_source = p.presentation\n\
          AND r_pres_n.period          = p.cur_period\n\
    LEFT JOIN sat_exchange_rate r_pres_n1\n\
           ON r_pres_n1.rate_set        = p.rate_set\n\
          AND r_pres_n1.currency_source = p.presentation\n\
          AND r_pres_n1.period          = p.prev_period\n\
    WHERE f.level = 'corporate' AND f.scenario = ?\n\
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
            p.pivot_currency,
            p.rate_set,
            p.current_period,
            p.prev_period,
            p.scenario_code, // conv CTE : WHERE f.scenario = ? (isolation)
            p.presentation_currency,
            p.presentation_currency,
            p.presentation_currency,
        ],
    )?;
    count_level(con, "converted")
}
