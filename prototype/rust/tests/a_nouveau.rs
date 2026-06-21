//! Test d'intégration du report d'ouverture (à-nouveau) — cf. docs/A_NOUVEAU.md.
//!
//! Scénario : on prend le run `REEL` (2024) du seed comme **snapshot N-1 figé**,
//! puis on crée un scénario courant `CUR` (2025) qui le référence en à-nouveau
//! (`dim_scenario.a_nouveau_scenario = 'REEL'`). On vérifie que :
//!   - le F00 d'ouverture de `CUR` est **collé** depuis la clôture F99 de `REEL`
//!     (au corporate ET au consolidé), pour l'entité continue M ;
//!   - le F00 de **liasse** (saisi à 9999) est **écrasé** par le report ;
//!   - la clôture F99 de `CUR` se referme : F00 (reporté) + F20 (saisi).
//!
//! L'à-nouveau est piloté par `dim_flow.flux_a_nouveau` (F99 → F00) ; le seed ne
//! le renseigne pas, donc on l'active explicitement ici.

use conso_engine::{create_schema, pipeline::run_pipeline, seed_all, ConvertParams};
use duckdb::Connection;

const TOL: f64 = 0.01;

/// Montant agrégé pour (scenario, level, entity, account, flow).
fn amt(con: &Connection, scenario: &str, level: &str, entity: &str, account: &str, flow: &str) -> f64 {
    con.query_row(
        "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
         WHERE scenario=? AND level=? AND entity=? AND account=? AND flow=?",
        [scenario, level, entity, account, flow],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("amt({scenario},{level},{entity},{account},{flow}) : {e}"))
}

#[test]
fn a_nouveau_reporte_la_cloture_sur_l_ouverture() {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");

    // Active l'à-nouveau (F99 → F00) — le seed laisse le champ NULL.
    con.execute("UPDATE dim_flow SET flux_a_nouveau='F00' WHERE code='F99'", [])
        .expect("activer flux_a_nouveau");

    // 1) Snapshot N-1 = run REEL (2024). REEL n'a pas d'à-nouveau → carry no-op.
    let p_reel = ConvertParams::load_params(&con, "REEL").expect("load_params REEL");
    run_pipeline(&con, &p_reel).expect("run REEL");

    let reel_corp_f99 = amt(&con, "REEL", "corporate", "M", "100", "F99");
    let reel_cons_f99 = amt(&con, "REEL", "consolidated", "M", "100", "F99");
    assert!(reel_corp_f99.abs() > TOL, "le snapshot REEL doit avoir un F99 M/100 non nul");

    // 2) Scénario courant CUR (2025) référençant REEL en à-nouveau.
    con.execute_batch(
        "INSERT INTO dim_period (code, libelle, type, date_debut, date_fin, statut)
         VALUES ('2025','Exercice 2025','exercice','2025-01-01','2025-12-31','ouvert');

         INSERT INTO dim_scenario
            (code, libelle, category, entry_period, presentation_currency,
             variant, ruleset_code, rate_set, statut, a_nouveau_scenario)
         VALUES ('CUR','Réel 2025','REEL','2025','EUR','BASE',NULL,'RATES','ouvert','REEL');

         INSERT INTO sat_perimeter
            (entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie)
         VALUES ('M','CUR','2025','globale',1.0,1.0,FALSE,FALSE);

         -- Liasse de M en 2025 : un F00 (9999, DOIT être écrasé par le report) + un F20.
         INSERT INTO stg_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, amount)
         VALUES
            ('CUR','M','2025','2025','100','F00','EUR','0LIASS',9999.00),
            ('CUR','M','2025','2025','100','F20','EUR','0LIASS',  50.00);",
    )
    .expect("seed scénario CUR");

    // 3) Run CUR : le carry colle F99[REEL] → F00[CUR] (corporate + consolidé).
    let p_cur = ConvertParams::load_params(&con, "CUR").expect("load_params CUR");
    assert_eq!(p_cur.a_nouveau_scenario.as_deref(), Some("REEL"), "CUR doit référencer REEL");
    run_pipeline(&con, &p_cur).expect("run CUR");

    let cur_corp_f00 = amt(&con, "CUR", "corporate", "M", "100", "F00");
    let cur_cons_f00 = amt(&con, "CUR", "consolidated", "M", "100", "F00");
    let cur_cons_f99 = amt(&con, "CUR", "consolidated", "M", "100", "F99");

    // (a) L'ouverture corporate de CUR = la clôture corporate de REEL (reportée).
    assert!(
        (cur_corp_f00 - reel_corp_f99).abs() < TOL,
        "F00[CUR] corporate = {cur_corp_f00} ≠ F99[REEL] corporate = {reel_corp_f99}"
    );
    // (b) Le F00 de liasse (9999) a bien été écrasé par le report.
    assert!(
        (cur_corp_f00 - 9999.0).abs() > TOL,
        "le F00 de liasse (9999) aurait dû être écrasé, eu {cur_corp_f00}"
    );
    // (c) L'ouverture consolidée de CUR = la clôture consolidée de REEL.
    assert!(
        (cur_cons_f00 - reel_cons_f99).abs() < TOL,
        "F00[CUR] consolidé = {cur_cons_f00} ≠ F99[REEL] consolidé = {reel_cons_f99}"
    );
    // (d) La clôture de CUR se referme : F00 reporté + F20 saisi (50, EUR, ×1.0).
    assert!(
        (cur_cons_f99 - (reel_cons_f99 + 50.0)).abs() < TOL,
        "F99[CUR] consolidé = {cur_cons_f99} ≠ F00 reporté ({reel_cons_f99}) + F20 (50)"
    );
}

#[test]
fn sans_a_nouveau_le_f00_de_liasse_est_conserve() {
    // Contrôle : un scénario SANS à-nouveau garde son F00 de liasse (pas de carry).
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    con.execute("UPDATE dim_flow SET flux_a_nouveau='F00' WHERE code='F99'", [])
        .expect("activer flux_a_nouveau");

    let p_reel = ConvertParams::load_params(&con, "REEL").expect("load_params REEL");
    assert!(p_reel.a_nouveau_scenario.is_none(), "REEL ne référence aucun à-nouveau");
    run_pipeline(&con, &p_reel).expect("run REEL");

    // M continue (EUR) : son F00 corporate vient de la liasse, non nul, non écrasé.
    let f00 = amt(&con, "REEL", "corporate", "M", "100", "F00");
    assert!(f00.abs() > TOL, "sans à-nouveau, le F00 de liasse de M/100 doit subsister");
}

/// Prépare un snapshot REEL (2024) consolidé (M, A, B) + un scénario CUR (2025)
/// le référençant en à-nouveau. Le périmètre de CUR est laissé à l'appelant.
fn snapshot_reel_et_cur() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    con.execute("UPDATE dim_flow SET flux_a_nouveau='F00' WHERE code='F99'", [])
        .expect("activer flux_a_nouveau");
    let p = ConvertParams::load_params(&con, "REEL").expect("load_params REEL");
    run_pipeline(&con, &p).expect("run REEL");
    con.execute_batch(
        "INSERT INTO dim_period (code,libelle,type,date_debut,date_fin,statut)
         VALUES ('2025','Exercice 2025','exercice','2025-01-01','2025-12-31','ouvert');
         INSERT INTO dim_scenario
            (code,libelle,category,entry_period,presentation_currency,variant,
             ruleset_code,rate_set,statut,a_nouveau_scenario)
         VALUES ('CUR','Réel 2025','REEL','2025','EUR','BASE',NULL,'RATES','ouvert','REEL');",
    )
    .expect("seed période + scénario CUR");
    con
}

#[test]
fn coherence_signale_divergences_et_orphelins() {
    let con = snapshot_reel_et_cur();
    // Périmètre CUR : M (entree=false, cohérent : M était consolidée en N-1) ;
    // NEW (entree=false → divergence : marquée continue mais absente de N-1).
    // A et B sont consolidées en N-1 mais absentes du périmètre CUR → orphelines.
    con.execute_batch(
        "INSERT INTO sat_perimeter
            (entity,scenario,period,methode,pct_interet,pct_integration,entree,sortie)
         VALUES ('M','CUR','2025','globale',1.0,1.0,FALSE,FALSE),
                ('NEW','CUR','2025','globale',1.0,1.0,FALSE,FALSE);",
    )
    .expect("seed périmètre CUR");

    let anomalies =
        conso_engine::validate::check_a_nouveau_coherence(&con, "CUR", "REEL", "2025")
            .expect("check_a_nouveau_coherence");

    let has = |kind: &str, entity: &str| {
        anomalies.iter().any(|a| a.kind == kind && a.entity == entity)
    };
    assert!(has("entree_divergente", "NEW"), "NEW doit diverger : {anomalies:?}");
    assert!(has("snapshot_orphelin", "A"), "A doit être orpheline : {anomalies:?}");
    assert!(has("snapshot_orphelin", "B"), "B doit être orpheline : {anomalies:?}");
    assert!(
        !anomalies.iter().any(|a| a.entity == "M"),
        "M est cohérente (continue + consolidée N-1) : {anomalies:?}"
    );
}

#[test]
fn coherence_ok_quand_perimetre_aligne() {
    let con = snapshot_reel_et_cur();
    // Périmètre CUR aligné : M, A, B toutes continues (entree=false) et toutes
    // consolidées en N-1 → aucune anomalie.
    con.execute_batch(
        "INSERT INTO sat_perimeter
            (entity,scenario,period,methode,pct_interet,pct_integration,entree,sortie)
         VALUES ('M','CUR','2025','globale',1.0,1.0,FALSE,FALSE),
                ('A','CUR','2025','globale',1.0,1.0,FALSE,FALSE),
                ('B','CUR','2025','globale',1.0,1.0,FALSE,FALSE);",
    )
    .expect("seed périmètre CUR");

    let anomalies =
        conso_engine::validate::check_a_nouveau_coherence(&con, "CUR", "REEL", "2025")
            .expect("check_a_nouveau_coherence");
    assert!(
        anomalies.is_empty(),
        "périmètre aligné → aucune anomalie attendue : {anomalies:?}"
    );
}
