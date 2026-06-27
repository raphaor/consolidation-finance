//! Report d'ouverture (à-nouveau) — colle le solde de **clôture** d'une conso
//! N-1 **figée** (snapshot) sur le flux d'**ouverture** du run courant, niveau
//! par niveau. Cf. `docs/A_NOUVEAU.md`.
//!
//! # Principe (générique, data-driven)
//!
//! Un flux de clôture C déclare son flux d'ouverture cible O via
//! `dim_flow.flux_a_nouveau` (aujourd'hui F99 → F00 ; jamais en dur). Le carry,
//! pour chaque couple (C, O) :
//!
//! 1. **écrase** le flux d'ouverture O du run courant au niveau visé (sémantique
//!    d'écrasement : DELETE puis INSERT), donc le F00 issu de la liasse est
//!    remplacé ;
//! 2. **colle** le solde de clôture C du snapshot (au **même niveau**),
//!    relabellisé en O, phase/période repointés sur le run courant et tagué
//!    `consolidation_id` du run courant.
//!
//! # Périmètre : entités consolidées en N-1 seulement
//!
//! Le carry ne concerne que les entités **effectivement consolidées** dans le
//! snapshot, c.-à-d. celles qui y portent une clôture C au niveau
//! `consolidated`. Les entités absentes (nouvelles entrées) gardent leur F00 de
//! liasse (reclassé en F01 par règle, hors moteur — cf. A_NOUVEAU.md §5). Cette
//! distinction garantit la non-duplication **à la source**, sans marqueur.
//!
//! # Niveaux
//!
//! - `corporate` : colle le F99 corporate du snapshot → F00 corporate. C'est ce
//!   montant (fonctionnel) qui sert ensuite de base aux écarts de conversion et
//!   au report sur la clôture. La conversion native reproduit alors le F99
//!   converti N-1 (cf. A_NOUVEAU.md §3.1) — pas de carry au converti.
//! - `consolidated` : colle le F99 consolidé du snapshot → F00 consolidé (figé
//!   au % d'intégration N-1). Appelé **après** l'étape D : l'écrasement remplace
//!   le F00 que la consolidation a produit (× pct N), de sorte que l'ouverture
//!   consolidée reste au % N-1. La variation vers le % N est une règle (hors
//!   moteur).

use super::ConvertParams;
use crate::dimensions;
use duckdb::{params, Connection};

/// Couples `(source_id, target_id, target_code)` d'à-nouveau :
/// - `source_id` : id du flux de clôture (ex. id de F99) — pour filtrer fact_entry.flow
/// - `target_id` : id du flux d'ouverture cible (ex. id de F00) — pour INSERT dans fact_entry.flow
/// - `target_code` : code TEXT du flux cible — pour le garde EXISTS `b.flux_a_nouveau = ?`
fn pairs(con: &Connection) -> duckdb::Result<Vec<(i64, i64, String)>> {
    let mut stmt = con.prepare(
        "SELECT DISTINCT flow AS source_id, flux_a_nouveau_id AS target_id, \
                         flux_a_nouveau AS target_code \
         FROM v_flow_behavior \
         WHERE flux_a_nouveau_id IS NOT NULL ORDER BY flow",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
    })?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

/// Exécute le carry d'à-nouveau pour le niveau `level` produit, si la
/// consolidation du run référence une conso d'à-nouveau
/// (`a_nouveau_consolidation_id`). No-op sinon, ou si `level` n'est ni
/// `corporate` ni `consolidated`.
///
/// Appelé par `run_steps` juste après la production du niveau et **avant** la
/// reconstruction des clôtures.
pub fn carry(con: &Connection, params: &ConvertParams, level: &str) -> duckdb::Result<()> {
    let snap = match params.a_nouveau_consolidation_id {
        Some(id) => id,
        None => return Ok(()), // pas d'à-nouveau
    };
    if level != "corporate" && level != "consolidated" {
        return Ok(());
    }

    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");

    // Entités éligibles au carry = consolidées dans le snapshot N-1 (clôture
    // `source` au niveau consolidated) ET présentes dans le scope du run courant
    // (`sat_perimeter`). fact_entry.entity est INTEGER, sat_perimeter.entity est TEXT
    // → bridge via dim_entity pour la comparaison périmètre.
    // `?` : snap(i64), source_id(i64), perimeter_set(i64), perimeter_period(TEXT).
    let eligible =
        "entity IN ( \
             SELECT DISTINCT entity FROM fact_entry \
             WHERE consolidation_id = ? AND level = 'consolidated' AND flow = ? \
         ) \
         AND entity IN ( \
             SELECT de.id FROM dim_entity de \
             WHERE de.code IN ( \
                 SELECT entity FROM sat_perimeter \
                 WHERE perimeter_set = ? AND period = ? \
             ) \
         )";

    for (source_id, target_id, target_code) in pairs(con)? {
        // 1) Écrase le flux d'ouverture `target_id` du run courant à ce niveau.
        con.execute(
            &format!(
                "DELETE FROM fact_entry \
                 WHERE consolidation_id = ? AND level = '{level}' AND flow = ? AND {eligible} \
                   AND EXISTS ( \
                       SELECT 1 FROM v_flow_behavior b \
                       WHERE b.account = fact_entry.account AND b.flow = ? AND b.flux_a_nouveau = ? \
                   )"
            ),
            params![
                params.consolidation_id,
                target_id,
                snap,
                source_id,
                params.perimeter_set,
                params.perimeter_period,
                source_id,
                target_code,
            ],
        )?;

        // 2) Colle le solde de clôture `source_id` du snapshot (même niveau),
        //    relabellisé `target_id`, repointé sur la phase/période du run.
        //    phase, entry_period, period : résolution code→id (params TEXT → id INTEGER).
        //    flow : target_id direct (i64).
        let sel = cols
            .iter()
            .map(|c| match *c {
                "phase" => "(SELECT id FROM dim_scenario_category WHERE code = ?)".to_string(),
                "entry_period" => "(SELECT id FROM dim_period WHERE code = ?)".to_string(),
                "period" => "(SELECT id FROM dim_period WHERE code = ?)".to_string(),
                "flow" => "?".to_string(),
                _ => format!("snap.{c}"),
            })
            .collect::<Vec<_>>()
            .join(", ");

        con.execute(
            &format!(
                "INSERT INTO fact_entry ({col_list}, consolidation_id, level, amount) \
                 SELECT {sel}, ?, '{level}', snap.amount \
                 FROM fact_entry snap \
                 WHERE snap.consolidation_id = ? AND snap.level = '{level}' AND snap.flow = ? \
                   AND snap.{eligible} \
                   AND EXISTS ( \
                       SELECT 1 FROM v_flow_behavior b \
                       WHERE b.account = snap.account AND b.flow = ? AND b.flux_a_nouveau = ? \
                   )"
            ),
            params![
                params.phase,       // phase subquery
                params.exercice,    // entry_period subquery
                params.exercice,    // period subquery
                target_id,          // flow = target_id (i64)
                params.consolidation_id,
                snap,
                source_id,
                snap,
                source_id,
                params.perimeter_set,
                params.perimeter_period,
                source_id,
                target_code,
            ],
        )?;
    }
    Ok(())
}
