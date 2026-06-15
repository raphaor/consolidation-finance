//! Étape B — Reclassification de périmètre (→ niveau `reclassified`).
//!
//! Miroir de `conso/pipeline.py::step_b_reclassify`.
//!
//! Travail en devise fonctionnelle (pas de conversion ici) :
//!   - Entité entrante  : F00 → F01 (l'ouverture de l'entrant est isolée en F01)
//!   - Entité sortante  : collapse F00 + F20 → F98 (solde isolé en F98)
//!   - Entité continue  : copie à l'identique

use super::count_level;
use duckdb::Connection;

/// SQL de la reclassification de périmètre.
const SQL_STEP_B: &str = "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, level, amount)
SELECT
    scenario, entity, entry_period, period, account, flow, currency,
    'reclassified' AS level,
    SUM(amount)    AS amount
FROM (
    -- 1) Entités continues : copie à l'identique
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           f.flow, f.currency, f.amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND NOT COALESCE(p.entree, FALSE)
      AND NOT COALESCE(p.sortie, FALSE)

    UNION ALL

    -- 2) Entités entrantes : F00 → F01, autres flux inchangés
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           CASE WHEN f.flow = 'F00' THEN 'F01' ELSE f.flow END AS flow,
           f.currency, f.amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND COALESCE(p.entree, FALSE)
      AND NOT COALESCE(p.sortie, FALSE)

    UNION ALL

    -- 3) Entités sortantes : collapse F00 + F20 → F98 (par compte)
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           'F98' AS flow, f.currency, f.amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND COALESCE(p.sortie, FALSE)
      AND f.flow IN ('F00', 'F20')
) rec
GROUP BY scenario, entity, entry_period, period, account, flow, currency;";

/// Exécute l'étape B : reclassifie les flux selon les variations de périmètre.
///
/// Renvoie le nombre de lignes produites au niveau `reclassified`.
pub fn step_b(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_B, [])?;
    count_level(con, "reclassified")
}
