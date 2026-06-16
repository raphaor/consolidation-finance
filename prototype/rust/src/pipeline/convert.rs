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
use duckdb::params;
use duckdb::Connection;

/// SQL de la conversion multi-devises.
///
/// Les paramètres `$presentation_currency`, `$current_period`, `$prev_period`
/// sont liés positionnellement (duckdb-rs ne gère pas les paramètres nommés
/// de la même manière que l'API Python `$name`). On utilise donc des `?`.
const SQL_STEP_C: &str = "\
WITH conv AS (
    SELECT
        f.scenario, f.entity, f.entry_period, f.period, f.account,
        f.flow, f.currency, f.amount,
        fl.taux_conversion,
        fl.flux_ecart,
        -- Taux applicable au flux (1.0 si déjà en devise de présentation)
        CASE
            WHEN f.currency = ? THEN 1.0
            WHEN fl.taux_conversion = 'close_n1' THEN r_n1.taux_close
            WHEN fl.taux_conversion = 'avg'      THEN r_n.taux_moyen
            WHEN fl.taux_conversion IN ('close_n', 'terminal')
                THEN r_n.taux_close
        END AS taux_flux,
        -- Taux de clôture N (référence pour le calcul d'écart)
        CASE
            WHEN f.currency = ? THEN 1.0
            ELSE r_n.taux_close
        END AS taux_close_n
    FROM fact_entry f
    JOIN dim_flow fl ON fl.code = f.flow
    LEFT JOIN sat_exchange_rate r_n
           ON r_n.currency_source = f.currency
          AND r_n.period = ?
    LEFT JOIN sat_exchange_rate r_n1
           ON r_n1.currency_source = f.currency
          AND r_n1.period = ?
    WHERE f.level = 'reclassified'
)
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, level, amount)
-- Montants convertis (tous flux constitutifs, exprimés en devise de présentation)
SELECT scenario, entity, entry_period, period, account, flow,
       ? AS currency,
       'converted' AS level,
       amount * taux_flux AS amount
FROM conv
UNION ALL
-- Lignes d'écart (devise ≠ présentation, flux porteur d'un flux_ecart, écart ≠ 0)
SELECT scenario, entity, entry_period, period, account, flux_ecart AS flow,
       ? AS currency,
       'converted' AS level,
       amount * (taux_close_n - taux_flux) AS amount
FROM conv
WHERE currency <> ?
  AND flux_ecart IS NOT NULL
  AND ABS(amount * (taux_close_n - taux_flux)) >= 0.005;";

/// Exécute l'étape C : convertit les écritures en devise de présentation et
/// génère les écarts (F80 / F81).
///
/// Renvoie le nombre de lignes produites au niveau `converted`.
pub fn step_c(con: &Connection, p: &ConvertParams) -> duckdb::Result<usize> {
    // L'ordre des `?` dans SQL_STEP_C :
    //   1. presentation_currency (taux_flux)
    //   2. presentation_currency (taux_close_n)
    //   3. current_period   (join r_n)
    //   4. prev_period      (join r_n1)
    //   5. presentation_currency (currency convertie)
    //   6. presentation_currency (currency écart)
    //   7. presentation_currency (filtre écart)
    con.execute(
        SQL_STEP_C,
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
