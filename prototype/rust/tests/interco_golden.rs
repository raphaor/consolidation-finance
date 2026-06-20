//! Test golden — élimination interco **bilan** multi-devises, flux à flux.
//!
//! Spec validée avec l'utilisateur (cf. discussion #13). Modélise une interco
//! bilan sur une filiale en USD, éliminée par une règle au niveau **converti**
//! qui reclasse **chaque flux** (F20 + son écart de conversion F81 + la clôture
//! F99) du compte créditeur 468 vers le compte de liaison 471L.
//!
//! Points démontrés :
//! - la règle sélectionne et génère **tous les flux** présents, F99 compris
//!   (R2 — reconstruction post-règle — étant débranchée) ;
//! - l'**écart de conversion F81** de l'interco suit l'élimination vers 471L ;
//! - l'**identité F99 tient sans reconstruction** : la règle élimine F99 de la
//!   somme exacte des constitutifs éliminés.
//!
//! Taux du seed (RATES, EUR pivot=présentation) : USD close N (2024)=0,90,
//! moyen=0,95, close N-1 (2023)=0,92. F00→close_n1 (écart F80), F20→avg (écart F81).

use conso_engine::rules::run_ruleset_at_level;
use conso_engine::{create_schema, run_pipeline_with_hook, seed_all, ConvertParams};
use duckdb::Connection;

const TOL: f64 = 0.01;

/// Définition de la règle d'élimination interco bilan : 468 → 471L, en nature
/// 2ELI, niveau converti. 4 opérations = {extourne 468, contrepartie 471L} ×
/// {principal (partner null), dont (partner hérité)}. Sélection = compte 468
/// avec partenaire renseigné (toutes les lignes interco, tous flux confondus).
/// Coefficient constant 1 (l'entité étant globale, l'intégration ×1,0 de
/// l'étape D ne modifie rien).
fn elim_468_json() -> &'static str {
    r#"{
        "scope": [],
        "operations": [
            {
                "seq": 1, "level": "converted",
                "selection": [
                    {"dim": "account", "op": "=", "val": "468"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "constant", "value": 1},
                "multiplicateur": -1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "inherit"}
                }
            },
            {
                "seq": 2, "level": "converted",
                "selection": [
                    {"dim": "account", "op": "=", "val": "468"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "constant", "value": 1},
                "multiplicateur": -1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "null"}
                }
            },
            {
                "seq": 3, "level": "converted",
                "selection": [
                    {"dim": "account", "op": "=", "val": "468"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "constant", "value": 1},
                "multiplicateur": 1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "account": {"mode": "override", "value": "471L"},
                    "partner": {"mode": "inherit"}
                }
            },
            {
                "seq": 4, "level": "converted",
                "selection": [
                    {"dim": "account", "op": "=", "val": "468"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "constant", "value": 1},
                "multiplicateur": 1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "account": {"mode": "override", "value": "471L"},
                    "partner": {"mode": "null"}
                }
            }
        ]
    }"#
}

fn setup() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");

    // A en périmètre **continu** (entree=false) : isole conversion + élimination
    // de l'effet d'entrée de périmètre (sinon le F00 serait reclassé en F01).
    con.execute(
        "UPDATE sat_perimeter SET entree = false WHERE entity = 'A' AND scenario = 'REEL'",
        [],
    )
    .expect("perimeter A continu");

    // Interco bilan sur A (USD) : ouverture 468 = 1000 (non-interco) + variation
    // 468 = 100 **entièrement interco avec M** (ligne principale partner=∅ + dont
    // partner=M).
    con.execute_batch(
        "INSERT INTO stg_entry \
            (scenario, entity, entry_period, period, account, flow, currency, \
             nature, partner, share, analysis, analysis2, amount) \
         VALUES \
            ('REEL','A','2024','2024','468','F00','USD','0LIASS',NULL,NULL,NULL,'S-A-468-1',1000.00), \
            ('REEL','A','2024','2024','468','F20','USD','0LIASS',NULL,NULL,NULL,'S-A-468-2', 100.00), \
            ('REEL','A','2024','2024','468','F20','USD','0LIASS','M', NULL,NULL,'S-A-468-3', 100.00);",
    )
    .expect("seed interco 468");

    // Règle + jeu.
    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
        duckdb::params!["ELI_468", "Élim interco 468->471L", elim_468_json()],
    )
    .expect("create rule");
    con.execute(
        "INSERT INTO dim_ruleset (code, libelle) VALUES ('RS_BILAN', 'Élim bilan')",
        [],
    )
    .expect("create ruleset");
    con.execute(
        "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) VALUES ('RS_BILAN', 1, 'ELI_468')",
        [],
    )
    .expect("create ruleset item");

    con
}

/// Montant net (toutes natures confondues) pour (level, account, partner, flow).
/// `partner = None` cible la ligne principale (partner NULL).
fn amt(con: &Connection, level: &str, account: &str, partner: Option<&str>, flow: &str) -> f64 {
    let (clause, p): (&str, Vec<String>) = match partner {
        Some(pp) => (
            "partner = ?",
            vec![level.into(), account.into(), pp.into(), flow.into()],
        ),
        None => (
            "partner IS NULL",
            vec![level.into(), account.into(), flow.into()],
        ),
    };
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0) FROM fact_entry \
         WHERE level = ? AND account = ? AND {clause} AND flow = ?"
    );
    con.query_row(&sql, duckdb::params_from_iter(p), |r| r.get::<_, f64>(0))
        .unwrap_or_else(|e| panic!("amt({level},{account},{partner:?},{flow}): {e}"))
}

/// Vérifie le tableau golden à un niveau donné (converti puis consolidé, A étant
/// globale → consolidé = converti × 1,0).
fn assert_golden(con: &Connection, level: &str) {
    // 468 principal : interco (F20) et son écart F81 éliminés ; reste l'ouverture
    // (F00=920) et son écart d'ouverture F80=−20 (non-interco, intouché).
    assert!((amt(con, level, "468", None, "F00") - 920.0).abs() < TOL, "{level} 468/∅ F00");
    assert!(amt(con, level, "468", None, "F20").abs() < TOL, "{level} 468/∅ F20 (éliminé)");
    assert!((amt(con, level, "468", None, "F80") - (-20.0)).abs() < TOL, "{level} 468/∅ F80");
    assert!(amt(con, level, "468", None, "F81").abs() < TOL, "{level} 468/∅ F81 (éliminé)");
    assert!((amt(con, level, "468", None, "F99") - 900.0).abs() < TOL, "{level} 468/∅ F99");

    // 468 « dont » M : entièrement éliminé.
    assert!(amt(con, level, "468", Some("M"), "F20").abs() < TOL, "{level} 468/M F20");
    assert!(amt(con, level, "468", Some("M"), "F81").abs() < TOL, "{level} 468/M F81");
    assert!(amt(con, level, "468", Some("M"), "F99").abs() < TOL, "{level} 468/M F99");

    // 471L : reçoit l'interco reclassée, **F81 = −5 compris**.
    assert!((amt(con, level, "471L", None, "F20") - 95.0).abs() < TOL, "{level} 471L/∅ F20");
    assert!((amt(con, level, "471L", None, "F81") - (-5.0)).abs() < TOL, "{level} 471L/∅ F81");
    assert!((amt(con, level, "471L", None, "F99") - 90.0).abs() < TOL, "{level} 471L/∅ F99");
    assert!((amt(con, level, "471L", Some("M"), "F20") - 95.0).abs() < TOL, "{level} 471L/M F20");
    assert!((amt(con, level, "471L", Some("M"), "F81") - (-5.0)).abs() < TOL, "{level} 471L/M F81");
    assert!((amt(con, level, "471L", Some("M"), "F99") - 90.0).abs() < TOL, "{level} 471L/M F99");
}

#[test]
fn interco_bilan_multidevise_golden() {
    let con = setup();
    let params = ConvertParams::load_params(&con, "REEL").expect("load_params");

    // Pipeline avec hook : la règle (level converted) s'intercale après l'étape C
    // (conversion), puis l'étape D consolide (× taux d'intégration = 1,0).
    let mut hook = |c: &Connection, level: &str| -> duckdb::Result<()> {
        run_ruleset_at_level(c, "RS_BILAN", level)?;
        Ok(())
    };
    run_pipeline_with_hook(&con, &params, &mut hook).expect("pipeline");

    assert_golden(&con, "converted");
    assert_golden(&con, "consolidated");
}
