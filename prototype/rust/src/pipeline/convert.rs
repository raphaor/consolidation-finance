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
use crate::{dimensions, references};
use duckdb::params;
use duckdb::Connection;

/// Exécute l'étape C : convertit les écritures en devise de présentation et
/// génère les écarts (F80 / F81).
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Après
/// le flip B1 étape 4, `fact_entry` stocke les dims en INTEGER ids. Les
/// codes TEXT de `stg_entry` (UNION préfixe 2) sont résolus en ids via JOIN.
/// `v_flow_behavior` expose désormais `flux_ecart_id` et `flux_de_report_id`
/// (INTEGER) à la place des codes TEXT.
///
/// # Paramètres `?` (ordre, 11 au total)
///
/// | #  | Valeur                | Rôle                                            |
/// |----|-----------------------|-------------------------------------------------|
/// | 1  | `presentation_currency` | CTE `params.presentation` (TEXT, lookup taux) |
/// | 2  | `pivot_currency`      | CTE `params.pivot` (TEXT, check cross-rate)     |
/// | 3  | `rate_set`            | CTE `params.rate_set` (subquery → id)           |
/// | 4  | `rate_period`         | CTE `params.cur_period`                         |
/// | 5  | `presentation_currency` | CTE `params.presentation_id` (subquery → id)  |
/// | 6  | `pivot_currency`      | CTE `params.pivot_id` (subquery → id)           |
/// | 7  | `consolidation_id`    | conv : fact_entry `WHERE consolidation_id = ?`  |
/// | 8  | `phase`               | conv : stg_entry préfixe 2 `WHERE phase = ?`    |
/// | 9  | `exercice`            | conv : stg_entry `AND entry_period = ?`         |
/// | 10 | `consolidation_id`    | branche converti : `? AS consolidation_id`      |
/// | 11 | `consolidation_id`    | branche écart : `? AS consolidation_id`         |
///
/// Renvoie le nombre de lignes produites au niveau `converted`.
pub fn step_c(con: &Connection, p: &ConvertParams) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);

    let f_cols = cols
        .iter()
        .map(|c| format!("f.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let insert_col_list = cols.join(", ");

    // Branche converti : currency → `presentation_id` (colonne exposée par conv).
    // Note : dans l'outer SELECT `FROM conv`, `p` n'est pas en scope — conv expose
    // déjà `presentation_id` via `SELECT p.presentation_id` dans la CTE.
    let final_cols_convert = cols
        .iter()
        .map(|c| {
            if *c == "currency" {
                "presentation_id".to_string()
            } else {
                (*c).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    // Branche écart : currency → presentation_id, flow → flux_ecart_id (INTEGER).
    let final_cols_ecart = cols
        .iter()
        .map(|c| {
            if *c == "currency" {
                "presentation_id".to_string()
            } else if *c == "flow" {
                "flux_ecart_id".to_string()
            } else {
                (*c).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    // Résolution code→id pour le UNION stg_entry (préfixe 2).
    // fact_entry (première branche du UNION) stocke déjà des INTEGER ids.
    let mut stg_id_joins = String::new();
    let mut stg_select_exprs: Vec<String> = Vec::new();
    for dim in &dims {
        let name = &dim.name;
        if let Some((table, code_col)) = references::dimension_master(name) {
            let alias = format!("_ds{name}");
            let join_type = if dim.nullable() { "LEFT JOIN" } else { "JOIN" };
            stg_id_joins.push_str(&format!(
                "\n        {join_type} {table} {alias} ON {alias}.{code_col} = s.{name}"
            ));
            stg_select_exprs.push(format!("{alias}.id"));
        } else {
            stg_select_exprs.push(format!("s.{name}"));
        }
    }
    let stg_select_list = stg_select_exprs.join(", ");

    let sql = format!(
        "\
WITH params AS (\n\
    SELECT\n\
        ?::TEXT AS presentation,\n\
        ?::TEXT AS pivot,\n\
        -- rate_set stocké en id (B1) : résolution code→id une fois dans la CTE.\n\
        (SELECT id FROM dim_rate_set WHERE code = ?::TEXT) AS rate_set,\n\
        ?::TEXT AS cur_period,\n\
        -- presentation_id / pivot_id : ids INTEGER pour comparaisons avec\n\
        -- fact_entry.currency (INTEGER après étape 4 B1).\n\
        (SELECT id FROM dim_currency WHERE code_iso = ?::TEXT) AS presentation_id,\n\
        (SELECT id FROM dim_currency WHERE code_iso = ?::TEXT) AS pivot_id\n\
),\n\
conv AS (\n\
    SELECT\n\
        {f_cols}, f.amount,\n\
        vfb.taux_conversion,\n\
        vfb.flux_ecart_id,\n\
        p.presentation_id,\n\
        CASE\n\
            WHEN f.currency = p.presentation_id THEN 1.0\n\
            ELSE\n\
                (CASE\n\
                    WHEN f.currency = p.pivot_id THEN 1.0\n\
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
        CASE\n\
            WHEN f.currency = p.presentation_id THEN 1.0\n\
            ELSE\n\
                (CASE\n\
                    WHEN f.currency = p.pivot_id THEN 1.0\n\
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
        -- Staging préfixe 2 : stg_entry en devise fonctionnelle → résolution\n\
        -- code→id pour correspondre aux colonnes INTEGER de fact_entry.\n\
        SELECT {stg_select_list}, amount\n\
        FROM stg_entry s\n\
        {stg_id_joins}\n\
        WHERE substr(s.nature, 1, 1) = '2' AND s.phase = ? AND s.entry_period = ?\n\
    ) f\n\
    JOIN v_flow_behavior vfb ON vfb.account = f.account AND vfb.flow = f.flow\n\
    LEFT JOIN v_flow_behavior vfb_rep\n\
           ON vfb_rep.account = f.account AND vfb_rep.flow = vfb.flux_de_report_id\n\
    CROSS JOIN params p\n\
    -- Résolution de la devise fonctionnelle (id → code_iso) pour la jointure\n\
    -- sur sat_exchange_rate.currency_source (TEXT code_iso).\n\
    LEFT JOIN dim_currency f_cu ON f_cu.id = f.currency\n\
    LEFT JOIN sat_exchange_rate r_n\n\
           ON r_n.rate_set        = p.rate_set\n\
          AND r_n.currency_source = f_cu.code_iso\n\
          AND r_n.period          = p.cur_period\n\
    LEFT JOIN sat_exchange_rate r_pres_n\n\
           ON r_pres_n.rate_set        = p.rate_set\n\
          AND r_pres_n.currency_source = p.presentation\n\
          AND r_pres_n.period          = p.cur_period\n\
)\n\
INSERT INTO fact_entry\n\
    ({insert_col_list}, consolidation_id, level, amount)\n\
SELECT {final_cols_convert},\n\
       ? AS consolidation_id,\n\
       'converted' AS level,\n\
       amount * taux_flux AS amount\n\
FROM conv\n\
UNION ALL\n\
SELECT {final_cols_ecart},\n\
       ? AS consolidation_id,\n\
       'converted' AS level,\n\
       amount * (taux_report - taux_flux) AS amount\n\
FROM conv\n\
WHERE currency <> presentation_id\n\
  AND flux_ecart_id IS NOT NULL\n\
  AND ABS(amount * (taux_report - taux_flux)) >= 0.005;"
    );
    con.execute(
        &sql,
        params![
            p.presentation_currency,      // 1 : params.presentation
            p.pivot_currency,             // 2 : params.pivot
            p.rate_set,                   // 3 : params.rate_set (subquery)
            p.rate_period,                // 4 : params.cur_period
            p.presentation_currency,      // 5 : params.presentation_id (subquery)
            p.pivot_currency,             // 6 : params.pivot_id (subquery)
            p.consolidation_id,           // 7 : conv fact_entry WHERE consolidation_id
            p.phase,                      // 8 : conv stg_entry WHERE phase
            p.exercice,                   // 9 : conv stg_entry AND entry_period
            p.consolidation_id,           // 10: branche converti ? AS consolidation_id
            p.consolidation_id,           // 11: branche écart ? AS consolidation_id
        ],
    )?;
    count_level(con, "converted")
}
