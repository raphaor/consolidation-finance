//! Étape B — Reclassification de périmètre (→ niveau `reclassified`).
//!
//! Miroir de `conso/pipeline.py::step_b_reclassify`.
//!
//! Travail en devise fonctionnelle (pas de conversion ici) :
//!   - Entité entrante  : F00 → F01 (l'ouverture de l'entrant est isolée en F01)
//!   - Entité sortante  : tous les flux passent à l'identique (clôtures incluses)
//!                        + chaque CONSTITUANT X génère un miroir −X sur F98,
//!                        donc F98 = −Σ(constituants) → F99 = 0 par identité de
//!                        reconstruction (le solde de la sortante ne fuit pas
//!                        dans F99 consolidé).
//!   - Entité continue  : copie à l'identique
//!
//! Le miroir F98 (branche 3b) cible les **constituants** (flux non-clôture de
//! `dim_flow`, i.e. `code <> flux_de_report`) : une clôture étant la somme des
//! constituants, la refléter sur F98 la compterait deux fois. Le passthrough
//! (3a), lui, ne filtre rien — les clôtures transitent puis sont écrasées par
//! la reconstruction au niveau reclassified. Cf. docs/FLUX_CONSO.md §9.

use super::count_level;
use duckdb::Connection;

/// SQL de la reclassification de périmètre.
const SQL_STEP_B: &str = "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2, level, amount)
SELECT
    scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2,
    'reclassified' AS level,
    SUM(amount)    AS amount
FROM (
    -- 1) Entités continues : copie à l'identique
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           f.flow, f.currency, f.nature, f.partner, f.share, f.analysis, f.analysis2, f.amount
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
           f.currency, f.nature, f.partner, f.share, f.analysis, f.analysis2, f.amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND COALESCE(p.entree, FALSE)
      AND NOT COALESCE(p.sortie, FALSE)

    UNION ALL

    -- 3a) Entités sortantes — passthrough de TOUS les flux à l'identique
    --     (clôtures incluses : une clôture saisie transite, puis sera écrasée
    --      par la reconstruction au niveau reclassified).
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           f.flow, f.currency, f.nature, f.partner, f.share, f.analysis, f.analysis2, f.amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND COALESCE(p.sortie, FALSE)

    UNION ALL

    -- 3b) Entités sortantes — miroir négatif sur F98 : chaque CONSTITUANT X
    --      (flux non-clôture) génère −X. On cible les constituants seulement :
    --      une clôture est la somme des constituants, donc la refléter sur F98
    --      la compterait deux fois. Agrégé par compte, F98 = −Σ(constituants) ;
    --      comme F98 reporte à F99 (flux_de_report = 'F99'), l'identité
    --      F99 = F00 + F20 + … + F98 se referme à 0.
    SELECT f.scenario, f.entity, f.entry_period, f.period, f.account,
           'F98' AS flow, f.currency, f.nature, f.partner, f.share, f.analysis, f.analysis2, -f.amount AS amount
    FROM fact_entry f
    JOIN sat_perimeter p
      ON p.entity = f.entity
     AND p.scenario = f.scenario
     AND p.period = f.entry_period
    WHERE f.level = 'corporate'
      AND COALESCE(p.sortie, FALSE)
      AND f.flow IN (SELECT code FROM dim_flow WHERE code <> flux_de_report)
) rec
GROUP BY scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2;";

/// Exécute l'étape B : reclassifie les flux selon les variations de périmètre.
///
/// Renvoie le nombre de lignes produites au niveau `reclassified`.
pub fn step_b(con: &Connection) -> duckdb::Result<usize> {
    con.execute(SQL_STEP_B, [])?;
    count_level(con, "reclassified")
}
