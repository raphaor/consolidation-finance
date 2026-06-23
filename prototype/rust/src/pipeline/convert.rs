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
//!   1. taux du flux via le schéma de flux (`taux_conversion`) :
//!        - `close_n1` → taux_ouverture N (cross-rate) — clôture N-1 portée par N
//!        - `avg`      → taux_moyen N     (cross-rate)
//!        - `close_n`  → taux_close N     (cross-rate)
//!   2. montant converti = `amount × taux_flux`
//!   3. écart = `amount × (taux_report − taux_flux)`, posté sur `flux_ecart`.
//!      `taux_report` = taux du **flux de report** du flux (la clôture où il se
//!      solde), résolu par compte. Générique : la référence suit le schéma de
//!      flux (`flux_de_report`) au lieu d'être figée sur le taux de clôture.
//!      Cas usuel : report = F99 au taux de clôture ⇒ référence = clôture N.
//!   4. lignes en devise de présentation : copie directe, aucun écart
//!
//! `terminal` est conservé comme **alias déprécié** de `close_n` (même taux,
//! écart nul puisque ces flux n'ont pas de `flux_ecart`) ; le seed et l'UI ne le
//! produisent plus.
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
/// # Paramètres `?` (ordre, 12 au total)
///
/// Les `?` sont liés dans l'ordre d'apparition **dans le texte SQL**. NB : dans
/// chaque branche finale, le `?` de la devise (issu de `final_cols_*`, qui remplace
/// `currency`) précède le `? AS consolidation_id` (la liste des colonnes propagées
/// est énumérée avant la consolidation technique).
///
/// | #  | Valeur                       | Rôle                                       |
/// |----|------------------------------|--------------------------------------------|
/// | 1  | `presentation_currency`      | CTE `params.presentation`                  |
/// | 2  | `pivot_currency`             | CTE `params.pivot`                         |
/// | 3  | `rate_set`                   | CTE `params.rate_set`                      |
/// | 4  | `rate_period`                | CTE `params.cur_period`                    |
/// | 5  | `consolidation_id`           | `conv` : fact_entry corporate `WHERE consolidation_id = ?` |
/// | 6  | `phase`                      | `conv` : stg_entry préfixe 2 `WHERE phase = ?` |
/// | 7  | `exercice`                   | `conv` : stg_entry préfixe 2 `AND entry_period = ?` |
/// | 8  | `presentation_currency`      | branche converti : `final_cols_convert` (colonne `currency`) |
/// | 9  | `consolidation_id`           | branche converti : `? AS consolidation_id` |
/// | 10 | `presentation_currency`      | branche écart : `final_cols_ecart` (colonne `currency`) |
/// | 11 | `consolidation_id`           | branche écart : `? AS consolidation_id`    |
/// | 12 | `presentation_currency`      | `WHERE currency <> ?` (filtre écart)       |
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
        .map(|c| {
            if *c == "currency" {
                "?".to_string()
            } else {
                (*c).to_string()
            }
        })
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
        ?::TEXT AS cur_period\n\
),\n\
conv AS (\n\
    SELECT\n\
        {f_cols}, f.amount,\n\
        vfb.taux_conversion,\n\
        vfb.flux_ecart,\n\
        -- Taux applicable au flux (cross-rate fonctionnelle -> présentation).\n\
        -- Court-circuit à 1.0 si la devise est déjà la présentation ; sinon\n\
        -- taux(fonctionnelle -> pivot) / taux(présentation -> pivot).\n\
        CASE\n\
            WHEN f.currency = p.presentation THEN 1.0\n\
            ELSE\n\
                (CASE\n\
                    WHEN f.currency = p.pivot THEN 1.0\n\
                    WHEN vfb.taux_conversion = 'close_n1' THEN r_n.taux_ouverture\n\
                    WHEN vfb.taux_conversion = 'avg'      THEN r_n.taux_moyen\n\
                    WHEN vfb.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_n.taux_close\n\
                END)\n\
                /\n\
                (CASE\n\
                    WHEN p.presentation = p.pivot THEN 1.0\n\
                    WHEN vfb.taux_conversion = 'close_n1' THEN r_pres_n.taux_ouverture\n\
                    WHEN vfb.taux_conversion = 'avg'      THEN r_pres_n.taux_moyen\n\
                    WHEN vfb.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_pres_n.taux_close\n\
                END)\n\
        END AS taux_flux,\n\
        -- Taux de référence de l'écart = taux du FLUX DE REPORT (la clôture où ce\n\
        -- flux se solde), résolu par compte via vfb_rep. Générique : la référence\n\
        -- suit le schéma de flux au lieu d'être codée en dur sur la clôture N.\n\
        -- Cas usuel (report = F99 en `close_n`) ⇒ retombe sur le taux de clôture.\n\
        CASE\n\
            WHEN f.currency = p.presentation THEN 1.0\n\
            ELSE\n\
                (CASE\n\
                    WHEN f.currency = p.pivot THEN 1.0\n\
                    WHEN vfb_rep.taux_conversion = 'close_n1' THEN r_n.taux_ouverture\n\
                    WHEN vfb_rep.taux_conversion = 'avg'      THEN r_n.taux_moyen\n\
                    WHEN vfb_rep.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_n.taux_close\n\
                END)\n\
                /\n\
                (CASE\n\
                    WHEN p.presentation = p.pivot THEN 1.0\n\
                    WHEN vfb_rep.taux_conversion = 'close_n1' THEN r_pres_n.taux_ouverture\n\
                    WHEN vfb_rep.taux_conversion = 'avg'      THEN r_pres_n.taux_moyen\n\
                    WHEN vfb_rep.taux_conversion IN ('close_n', 'terminal')\n\
                        THEN r_pres_n.taux_close\n\
                END)\n\
        END AS taux_report\n\
    FROM (\n\
        SELECT {insert_col_list}, amount FROM fact_entry\n\
        WHERE level = 'corporate' AND consolidation_id = ?\n\
        UNION ALL\n\
        -- Staging préfixe 2 : écritures en devise FONCTIONNELLE injectées au\n\
        -- converti, qui subissent la conversion (montant + écarts F80/F81) sans\n\
        -- apparaître au corporate. Cf. docs/A_NOUVEAU.md §4 bis.\n\
        SELECT {insert_col_list}, amount FROM stg_entry\n\
        WHERE substr(nature, 1, 1) = '2' AND phase = ? AND entry_period = ?\n\
    ) f\n\
    -- Comportement du flux résolu PAR COMPTE (taux de conversion + flux d'écart)\n\
    -- via le schéma de flux du compte. Cf. v_flow_behavior (Q32).\n\
    JOIN v_flow_behavior vfb ON vfb.account = f.account AND vfb.flow = f.flow\n\
    -- Comportement du FLUX DE REPORT du flux (sur le même compte), pour le taux\n\
    -- de référence de l'écart. LEFT JOIN : sans flux de report, taux_report NULL\n\
    -- → la ligne d'écart est filtrée (pas d'écart).\n\
    LEFT JOIN v_flow_behavior vfb_rep\n\
           ON vfb_rep.account = f.account AND vfb_rep.flow = vfb.flux_de_report\n\
    CROSS JOIN params p\n\
    -- Taux de la devise fonctionnelle vers le pivot (N : close, moyen, ouverture)\n\
    LEFT JOIN sat_exchange_rate r_n\n\
           ON r_n.rate_set        = p.rate_set\n\
          AND r_n.currency_source = f.currency\n\
          AND r_n.period          = p.cur_period\n\
    -- Taux de la devise de présentation vers le pivot (N : close, moyen, ouverture)\n\
    LEFT JOIN sat_exchange_rate r_pres_n\n\
           ON r_pres_n.rate_set        = p.rate_set\n\
          AND r_pres_n.currency_source = p.presentation\n\
          AND r_pres_n.period          = p.cur_period\n\
)\n\
INSERT INTO fact_entry\n\
    ({insert_col_list}, consolidation_id, level, amount)\n\
-- Montants convertis (tous flux constitutifs, exprimés en devise de présentation)\n\
SELECT {final_cols_convert},\n\
       ? AS consolidation_id,\n\
       'converted' AS level,\n\
       amount * taux_flux AS amount\n\
FROM conv\n\
UNION ALL\n\
-- Lignes d'écart (devise ≠ présentation, flux porteur d'un flux_ecart, écart ≠ 0)\n\
-- L'écart F80/F81 hérite de la nature — et du partner — du flux parent.\n\
SELECT {final_cols_ecart},\n\
       ? AS consolidation_id,\n\
       'converted' AS level,\n\
       amount * (taux_report - taux_flux) AS amount\n\
FROM conv\n\
WHERE currency <> ?\n\
  AND flux_ecart IS NOT NULL\n\
  AND ABS(amount * (taux_report - taux_flux)) >= 0.005;"
    );
    con.execute(
        &sql,
        params![
            p.presentation_currency,
            p.pivot_currency,
            p.rate_set,
            p.rate_period,
            p.consolidation_id, // conv : fact_entry corporate WHERE consolidation_id = ?
            p.phase,            // conv : stg_entry préfixe 2 WHERE phase = ?
            p.exercice,         // conv : stg_entry préfixe 2 AND entry_period = ?
            p.presentation_currency, // branche converti : final_cols_convert (currency)
            p.consolidation_id, // branche converti : ? AS consolidation_id
            p.presentation_currency, // branche écart : final_cols_ecart (currency)
            p.consolidation_id, // branche écart : ? AS consolidation_id
            p.presentation_currency, // WHERE currency <> ?
        ],
    )?;
    count_level(con, "converted")
}
