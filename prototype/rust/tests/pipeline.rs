//! Tests d'intégration du pipeline de consolidation.
//!
//! Chaque test ouvre une DuckDB **en mémoire** (isolation totale, pas de fichier
//! à nettoyer), crée le schéma, charge le jeu de seed (groupe M/A/B), exécute le
//! pipeline A→B→C→D puis vérifie :
//!   - les comptes par niveau (corporate/reclassified/converted/consolidated) ;
//!   - les montants F99 attendus par compte au niveau consolidated ;
//!   - l'identité de reconstruction des clôtures via `validate` ;
//!   - la présence/absence des écarts F80/F81 selon la devise ;
//!   - la reproductibilité (re-run après `DELETE FROM fact_entry`).
//!
//! Les montants attendus sont dérivés à la main du seed (cf. `src/seed.rs`) et
//! des taux de change. Toute régression dans une étape (agrégation, conversion,
//! reclassification, consolidation) fera échouer au moins un de ces tests.

use conso_engine::{
    create_schema,
    pipeline::run_pipeline,
    seed_all,
    validate::{validate_consolidated, validate_functional},
    ConvertParams,
};
use duckdb::Connection;

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers locaux (SQL) — propres au fichier de test.
// ─────────────────────────────────────────────────────────────────────────────

/// Ouvre une connexion en mémoire, crée le schéma, charge le seed, lance le
/// pipeline. Renvoie la connexion prête à être interrogée.
fn setup() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    let params = ConvertParams::load_params(&con, "REEL").expect("load_params");
    run_pipeline(&con, &params).expect("run_pipeline");
    con
}

/// Nombre de lignes stockées à un niveau donné.
fn level_count(con: &Connection, level: &str) -> i64 {
    con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ?",
        [level],
        |row| row.get(0),
    )
    .unwrap_or_else(|e| panic!("level_count({level}) : {e}"))
}

/// Somme des montants d'un ensemble de flux pour un compte à un niveau.
/// Utilisée pour reconstruire F99 = Σ des flux constitutifs.
fn flow_sum(con: &Connection, level: &str, account: &str, flows: &[&str]) -> f64 {
    // Liste de placeholders (?, ?, …) pour la clause IN.
    let placeholders: Vec<&str> = flows.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0) FROM fact_entry \
         WHERE level = ? AND account = ? AND flow IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = con.prepare(&sql).expect("prepare flow_sum");
    let mut params: Vec<&dyn duckdb::ToSql> = vec![&level, &account];
    for f in flows {
        params.push(f);
    }
    let sum: f64 = stmt
        .query_row(params.as_slice(), |row| row.get(0))
        .unwrap_or_else(|e| panic!("flow_sum({level}, {account}) : {e}"));
    sum
}

/// Reconstruit F99 (somme des 6 flux constitutifs) pour un compte au niveau
/// consolidated. C'est exactement ce que fait `validate::check_level`.
fn f99_consolidated(con: &Connection, account: &str) -> f64 {
    flow_sum(
        con,
        "consolidated",
        account,
        &["F00", "F01", "F20", "F80", "F81", "F98"],
    )
}

/// Nombre de lignes pour un ensemble de flux à un niveau, optionnellement
/// filtré sur un sous-ensemble d'entités.
fn flow_rows(con: &Connection, level: &str, flows: &[&str], entities: Option<&[&str]>) -> i64 {
    let placeholders: Vec<&str> = flows.iter().map(|_| "?").collect();
    let (sql, bind_entities): (String, bool) = match entities {
        Some(ents) if !ents.is_empty() => {
            let e_ph: Vec<&str> = ents.iter().map(|_| "?").collect();
            (
                format!(
                    "SELECT COUNT(*) FROM fact_entry \
                     WHERE level = ? AND flow IN ({}) AND entity IN ({})",
                    placeholders.join(", "),
                    e_ph.join(", ")
                ),
                true,
            )
        }
        _ => (
            format!(
                "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND flow IN ({})",
                placeholders.join(", ")
            ),
            false,
        ),
    };

    let mut stmt = con.prepare(&sql).expect("prepare flow_rows");
    let mut params: Vec<&dyn duckdb::ToSql> = vec![&level];
    for f in flows {
        params.push(f);
    }
    if bind_entities {
        if let Some(ents) = entities {
            for e in ents {
                params.push(e);
            }
        }
    }
    let n: i64 = stmt
        .query_row(params.as_slice(), |row| row.get(0))
        .unwrap_or_else(|e| panic!("flow_rows : {e}"));
    n
}

/// Tolérance f64 (le moteur stocke en DECIMAL(18,2), la lecture se fait en f64).
const TOL: f64 = 0.01;

// ─────────────────────────────────────────────────────────────────────────────
//  1. Comptes par étape (structure du pipeline)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pipeline_produit_les_bons_comptes_par_niveau() {
    let con = setup();
    // Comptages justifiés par le seed (cf. src/seed.rs). Rappel du périmètre :
    //   M = continu (EUR), A = entrante (USD), B = sortante (GBP) ; globale 100%.
    //
    //   Le grain d'agrégation/clôture inclut les dimensions analytiques
    //   (partner/share/analysis/analysis2), mais le seed ne les renseigne PAS :
    //   la réf. source (`S-M-001`…) vit dans `stg_entry.source`, **non-
    //   dimensionnelle**. Les écritures se ré-agrègent donc par compte, et les
    //   clôtures F99 sont reconstruites par compte (pas par ligne).
    //
    //   - corporate = 31 : M=11 (4 F00 + 7 F20), A=10 (3 F00 + 7 F20), B=10.
    //
    //   - reclassified = 64 : constitutifs 39 (M 11 ; A 10 = F00→F01 + F20 ;
    //     B 18 = 3 F00 + 7 F20 + 8 miroirs −X sur F98 de la sortante) + 25
    //     clôtures F99 reconstruites (M 9 comptes, A 8, B 8).
    //
    //   - converted = 84 : 64 lignes converties (clôtures F99 comprises) + 20
    //     écarts de conversion (A/USD et B/GBP : 3 F80 + 7 F81 chacun ; M/EUR
    //     n'en génère pas). materialize(converted) écrase ensuite le F99 porté.
    //
    //   - consolidated = 84 : consolidation (pct 100 %) des 84 lignes converted.
    //
    //   (Détail ligne à ligne : `cargo run --release --bin dump_pipeline`.)
    let corp = level_count(&con, "corporate");
    let recl = level_count(&con, "reclassified");
    let conv = level_count(&con, "converted");
    let cons = level_count(&con, "consolidated");
    assert_eq!(corp, 31, "niveau corporate");
    assert_eq!(recl, 64, "niveau reclassified");
    assert_eq!(conv, 84, "niveau converted");
    assert_eq!(cons, 84, "niveau consolidated");
}

// ─────────────────────────────────────────────────────────────────────────────
//  1b. F99 présent au niveau converted (les clôtures transitent par la
//      conversion, puis materialize(converted) les écrase par la reconstruction).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn f99_present_au_niveau_converted_et_identite_tient() {
    let con = setup();

    // F99 doit être présent au niveau converted (clôtures portées par l'étape C
    // puis reconstruites autoritairement par materialize(converted)).
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='converted' AND flow='F99'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n > 0, "F99 doit être présent au niveau converted");

    // L'identité de reconstruction y tient (F99 converti = Σ constituants convertis).
    let checks = conso_engine::validate::check_closures(&con, "converted")
        .expect("check_closures converted");
    assert!(!checks.is_empty(), "aucun contrôle renvoyé pour converted");
    for c in &checks {
        assert!(
            c.ok,
            "identité F99 en échec au niveau converted pour {} : écart = {}",
            c.account,
            c.ecart
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  2. Montants F99 attendus au niveau consolidated
//     (détecte toute régression d'agrégation / conversion / consolidation)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn montants_f99_consolidated_attendus() {
    let con = setup();

    let capital = f99_consolidated(&con, "100");
    let immo = f99_consolidated(&con, "200");
    let stocks = f99_consolidated(&con, "300");
    let resultat = f99_consolidated(&con, "400");

    assert!(
        (capital - 14_500.00).abs() < TOL,
        "100 consolidated F99 = {capital} (attendu 14500.00)"
    );
    assert!(
        (immo - 20_060.00).abs() < TOL,
        "200 consolidated F99 = {immo} (attendu 20060.00)"
    );
    assert!(
        (stocks - 3_000.00).abs() < TOL,
        "300 consolidated F99 = {stocks} (attendu 3000.00)"
    );
    assert!(
        (resultat - 7_870.00).abs() < TOL,
        "400 consolidated F99 = {resultat} (attendu 7870.00)"
    );
}

#[test]
fn comptes_attendus_presents_au_niveau_consolidated() {
    let con = setup();
    for acc in &[
        "100",
        "200",
        "300",
        "400",
    ] {
        let n: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM fact_entry WHERE level='consolidated' AND account=?",
                [acc],
                |row| row.get(0),
            )
            .unwrap();
        assert!(n > 0, "compte {acc} absent du niveau consolidated");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  3. Identité de reconstruction des clôtures (validateur du crate)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn validate_f99_consolidated_tient() {
    let con = setup();
    let checks = validate_consolidated(&con).expect("validate_consolidated");
    assert!(!checks.is_empty(), "aucun compte renvoyé par la validation");
    for c in &checks {
        assert!(
            c.ok,
            "identité F99 en échec au niveau consolidated pour {} : écart = {}",
            c.account,
            c.ecart
        );
    }
}

#[test]
fn validate_f99_functional_tient() {
    let con = setup();
    let checks = validate_functional(&con).expect("validate_functional");
    assert!(!checks.is_empty(), "aucun compte renvoyé par la validation");
    for c in &checks {
        assert!(
            c.ok,
            "identité F99 en échec au niveau reclassified pour {} : écart = {}",
            c.account,
            c.ecart
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  4. Écarts de conversion F80/F81
//     - absents en devise fonctionnelle (corporate / reclassified) ;
//     - présents en devise de présentation (converted / consolidated) ;
//     - localisés sur les entités non-EUR (A=USD, B=GBP), absents sur M=EUR.
//
//     NB : depuis la refonte de la sortie de périmètre (miroir −F98 par flux),
//     la sortante B garde ses flux F00/F20 à la conversion → elle génère
//     elle aussi des F80/F81 (absorbés par F98 dans F99 = 0).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn ecarts_f80_f81_absents_des_niveaux_fonctionnels() {
    let con = setup();
    for lvl in &["corporate", "reclassified"] {
        let n = flow_rows(&con, lvl, &["F80", "F81"], None);
        assert_eq!(
            n, 0,
            "F80/F81 ne doivent pas exister au niveau {lvl} (devise fonctionnelle)"
        );
    }
}

#[test]
fn ecarts_f80_f81_presents_aux_niveaux_presentation() {
    let con = setup();
    for lvl in &["converted", "consolidated"] {
        let n = flow_rows(&con, lvl, &["F80", "F81"], None);
        assert!(
            n > 0,
            "F80/F81 doivent être présents au niveau {lvl} (devise de présentation)"
        );
    }
}

#[test]
fn ecarts_f80_f81_localises_sur_entites_non_eur() {
    let con = setup();

    // M (EUR) : aucun écart (taux = 1).
    let n_m = flow_rows(&con, "consolidated", &["F80", "F81"], Some(&["M"]));
    assert_eq!(n_m, 0, "M (EUR) ne doit générer aucun écart F80/F81");

    // A (USD, entrante) : écart F80 sur F01 (close_n1) et F81 sur F20 (avg).
    let n_a = flow_rows(&con, "consolidated", &["F80", "F81"], Some(&["A"]));
    assert!(n_a > 0, "A (USD) doit générer des écarts F80/F81");

    // B (GBP, sortante) : ses flux F00/F20 sont convertis et génèrent des
    // écarts F80/F81 (ensuite absorbés par F98 dans F99 = 0).
    let n_b = flow_rows(&con, "consolidated", &["F80", "F81"], Some(&["B"]));
    assert!(n_b > 0, "B (sortante) doit générer des F80/F81 sur ses constituants");
}

// ─────────────────────────────────────────────────────────────────────────────
//  5. Cohérence de la conversion : le montant converti d'A + son écart
//     reconstitue le montant × taux_close_n. Vérification indépendante du
//     validateur (qui, lui, reconstruit F99 par somme — triviale par construction).
//
//     NB : on filtre sur l'entité A car depuis la refonte de la sortie de
//     périmètre, la sortante B génère elle aussi du F80 sur le compte 100
//     (sur son F00 converti) ; un agrégat au seul compte mélangerait A et B.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn conversion_reconstitue_montant_au_taux_close_n() {
    let con = setup();

    // Pour A (USD), compte 100 (Capital), flux F01 (ex F00 entrant) :
    //   converted(F01 de A) + écart(F80 de A) doit valoir fonctionnel × taux_close_n.
    let f01_pres = amount_for(&con, "consolidated", "A", "100", "F01");
    let f80_pres = amount_for(&con, "consolidated", "A", "100", "F80");

    // Côté fonctionnel (reclassified) : F01 de A = 5000 USD.
    let f01_func: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F01' AND entity='A'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    // Identité de conversion : converted + écart_signé = fonctionnel × taux_close_n.
    // (L'écart F80 est signé : 5000×(0.90−0.92) = −100 ; 4600 + (−100) = 4500.)
    let reconstruit = f01_pres + f80_pres;
    let attendu = f01_func * 0.90; // taux_close_n USD 2024 = 0.90
    assert!(
        (reconstruit - attendu).abs() < TOL,
        "conversion A/100 : reconstruit {reconstruit} ≠ attendu {attendu}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  6. Reproductibilité : un second run (après DELETE de fact_entry) redonne
//     exactement les mêmes montants et comptes.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pipeline_reproductible_apres_reset() {
    let con = setup();

    // Snapshot des montants F99 consolidés.
    let accounts = ["100", "200", "300", "400"];
    let before: Vec<f64> = accounts.iter().map(|a| f99_consolidated(&con, a)).collect();
    let counts_before: [i64; 4] = [
        level_count(&con, "corporate"),
        level_count(&con, "reclassified"),
        level_count(&con, "converted"),
        level_count(&con, "consolidated"),
    ];

    // Reset : on vide fact_entry (stg_entry est conservé).
    con.execute_batch("DELETE FROM fact_entry;")
        .expect("DELETE fact_entry");

    // Re-run.
    let params = ConvertParams::load_params(&con, "REEL").expect("load_params");
    run_pipeline(&con, &params).expect("re-run pipeline");

    let after: Vec<f64> = accounts.iter().map(|a| f99_consolidated(&con, a)).collect();
    let counts_after: [i64; 4] = [
        level_count(&con, "corporate"),
        level_count(&con, "reclassified"),
        level_count(&con, "converted"),
        level_count(&con, "consolidated"),
    ];

    assert_eq!(counts_before, counts_after, "comptes par niveau non reproductibles");
    for (i, acc) in accounts.iter().enumerate() {
        assert!(
            (before[i] - after[i]).abs() < TOL,
            "montant F99 non reproductible pour {acc} : {} puis {}",
            before[i],
            after[i]
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  7. Généricité de la reconstruction (Q28) — `flux_de_report` pilotant les
//     clôtures, et sémantique d'écrasement au grain.
//
//     On vérifie hors seed que :
//       (a) une clôture autre que F99 (ici F88, auto-référentielle) est
//           reconstruite depuis les flux qui y reportent (F10) ;
//       (b) la reconstruction est AUTORITAIRE au grain : une valeur résiduelle
//           sur la clôture au même grain est écrasée (pas additionnée) ;
//       (c) une valeur résiduelle sur un AUTRE grain (ici un autre compte, qui
//           sert de proxy à une autre dimension — ex. Nature à venir) est
//           PRÉSERVÉE (l'écrasement ne déborde pas sur un grain sans composante).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn materialize_closures_reconstruit_plusieurs_clotures_et_ecrase_au_grain() {
    use conso_engine::pipeline::materialize_closures::materialize_closures;

    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");

    // dim_flow : F99 (clôture auto-réf) ← F20 ; F88 (clôture auto-réf) ← F10.
    con.execute_batch(
        "INSERT INTO dim_flow
            (code, libelle, taux_conversion, flux_ecart, flux_de_report) VALUES
            ('F20','Variation','avg',NULL,'F99'),
            ('F99','Clôture','close_n',NULL,'F99'),
            ('F10','Intermédiaire','avg',NULL,'F88'),
            ('F88','Clôture intermédiaire','close_n',NULL,'F88');",
    )
    .expect("seed dim_flow");

    // dim_nature minimale pour le test (2 codes : liasse + ajustement).
    con.execute_batch(
        "INSERT INTO dim_nature VALUES
            ('0LIASS','Liasse',NULL),
            ('1AJUST','Ajustement',NULL);",
    )
    .expect("seed dim_nature");

    // Composantes au niveau reclassified : F20 = 50 et F10 = 30 sur le compte 100
    // (nature 0LIASS).
    con.execute_batch(
        "INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, level, amount)
         VALUES
            ('REEL','M','2024','2024','100','F20','EUR','0LIASS','reclassified',50.00),
            ('REEL','M','2024','2024','100','F10','EUR','0LIASS','reclassified',30.00);",
    )
    .expect("seed composantes");

    materialize_closures(&con, "reclassified").expect("materialize #1");

    // (a) F99 = F20 = 50 ; F88 = F10 = 30.
    let f99: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F99'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let f88: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F88'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((f99 - 50.0).abs() < TOL, "F99 devrait valoir 50 (F20), eu {f99}");
    assert!((f88 - 30.0).abs() < TOL, "F88 devrait valoir 30 (F10), eu {f88}");

    // (b)+(c) On injecte des valeurs résiduelles, puis on re-materialise.
    //   - F99 @ compte 100 (même grain) = 999      → doit être ÉCRASÉ en 50.
    //   - F99 @ compte 200 (grain distinct, proxy d'une autre dimension) = 777
    //                                                  → doit être PRÉSERVÉ.
    con.execute_batch(
        "INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, level, amount)
         VALUES
            ('REEL','M','2024','2024','100','F99','EUR','0LIASS','reclassified',999.00),
            ('REEL','M','2024','2024','200','F99','EUR','0LIASS','reclassified',777.00);",
    )
    .expect("seed résiduel");

    materialize_closures(&con, "reclassified").expect("materialize #2");

    // (b) écrasement au même grain — pas d'addition (50, ni 1049, ni 999).
    let f99_100: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F99'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        con.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F99'",
            [],
            |r| r.get(0),
        )
        .unwrap(),
        1,
        "une seule ligne F99 au grain (pas de doublon)"
    );
    assert!(
        (f99_100 - 50.0).abs() < TOL,
        "F99@100 doit être écrasé à 50 (pas additionné), eu {f99_100}"
    );

    // (c) grain distinct préservé — 777 intact (aucune composante sur le compte 200).
    let f99_200: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='200' AND flow='F99'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (f99_200 - 777.0).abs() < TOL,
        "F99@200 (grain sans composante) doit rester 777, eu {f99_200}"
    );

    // (d) Non-diversion entre codes Nature : la reconstruction sur la nature
    //     0LIASS ne doit pas écraser une clôture résiduelle portée par une
    //     AUTRE nature (1AJUST) sur le même compte. Puisque `nature` entre
    //     dans le grain, F99@100@1AJUST est un grain distinct de F99@100@0LIASS.
    con.execute_batch(
        "INSERT INTO fact_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, level, amount)
         VALUES
            ('REEL','M','2024','2024','100','F99','EUR','1AJUST','reclassified',555.00);",
    )
    .expect("seed résiduel autre nature");

    materialize_closures(&con, "reclassified").expect("materialize #3");

    let f99_ajust: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F99' \
               AND nature='1AJUST'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (f99_ajust - 555.0).abs() < TOL,
        "F99@100@1AJUST doit être préservé à 555 (grain nature distinct), eu {f99_ajust}"
    );
    // Et la clôture 0LIASS reste à 50 (pas de diversion).
    let f99_liasse_apres: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100' AND flow='F99' \
               AND nature='0LIASS'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (f99_liasse_apres - 50.0).abs() < TOL,
        "F99@100@0LIASS reste 50 après la reconstruction sur 1AJUST (non-diversion), eu {f99_liasse_apres}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  9. Validation de la nature (obligatoire + FK sur dim_nature)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn check_natures_ok_pour_le_seed() {
    use conso_engine::validate::check_natures;
    let con = setup();
    let anomalies = check_natures(&con).expect("check_natures");
    assert!(
        anomalies.is_empty(),
        "le seed ne doit contenir aucune écriture sans nature ou avec nature inconnue : {anomalies:?}"
    );
}

#[test]
fn check_natures_detecte_nature_manquante_et_inconnue() {
    use conso_engine::validate::check_natures;
    let con = setup();

    // Une écriture sans nature (NULL via colonne omise → on UPDATE à NULL/Vide).
    // On cible une ligne par sa réf. `source` (les `S-M-xxx` sont désormais dans
    // `stg_entry.source`, plus dans `analysis2`).
    con.execute(
        "UPDATE stg_entry SET nature = '' WHERE source = 'S-M-001'",
        [],
    )
    .expect("update nature vide");

    // Une écriture avec une nature inconnue de dim_nature.
    con.execute(
        "UPDATE stg_entry SET nature = 'XNOPE' WHERE source = 'S-M-002'",
        [],
    )
    .expect("update nature inconnue");

    let anomalies = check_natures(&con).expect("check_natures");
    let has_missing = anomalies.iter().any(|a| a.kind == "missing" && a.count >= 1);
    let has_unknown = anomalies
        .iter()
        .any(|a| a.kind == "unknown" && a.nature.as_deref() == Some("XNOPE"));
    assert!(has_missing, "doit détecter une nature manquante : {anomalies:?}");
    assert!(has_unknown, "doit détecter une nature inconnue : {anomalies:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
//  8. Sortie de périmètre — F98 = −Σ(constituants) et F99 = 0 par identité.
//
//     La sortante B (GBP) ne doit pas fuir dans F99 : ses flux F00/F20 sont
//     conservés à l'identique (donc visibles et convertis), et chaque
//     constituant X génère un miroir −X sur F98. L'identité F99 = F00+F20+F98
//     se referme à 0, en DEVISE FONCTIONNELLE comme en DEVISE DE PRÉSENTATION.
// ─────────────────────────────────────────────────────────────────────────────

/// SOMME des montants pour (level, entity, account, flow) — zéro si absent.
fn amount_for(con: &Connection, level: &str, entity: &str, account: &str, flow: &str) -> f64 {
    con.query_row(
        "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
         WHERE level=? AND entity=? AND account=? AND flow=?",
        [level, entity, account, flow],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("amount_for({level},{entity},{account},{flow}) : {e}"))
}

#[test]
fn sortie_perimetre_donne_f99_zero_et_f98_negatif() {
    let con = setup();

    // B (GBP, sortante) au niveau reclassified (devise fonctionnelle) :
    // par compte, F98 = -(F00+F20) et F99 = 0.
    // (compte, F00, F20, F98_attendu) — montants dérivés du seed.
    let cases: &[(&str, f64, f64, f64)] = &[
        ("100", 4000.0,    0.0, -4000.0),
        ("400", 1500.0,  200.0, -1700.0),
        ("200", 6000.0,  300.0, -6300.0),
        ("700",    0.0,  800.0,  -800.0),
        ("705",    0.0,  400.0,  -400.0),
        ("600",    0.0,  300.0,  -300.0),
        ("610",    0.0,  200.0,  -200.0),
        ("640",    0.0,  100.0,  -100.0),
    ];
    for &(acc, f00, f20, f98_attendu) in cases {
        let got_f00 = amount_for(&con, "reclassified", "B", acc, "F00");
        let got_f20 = amount_for(&con, "reclassified", "B", acc, "F20");
        let got_f98 = amount_for(&con, "reclassified", "B", acc, "F98");
        let got_f99 = amount_for(&con, "reclassified", "B", acc, "F99");
        assert!((got_f00 - f00).abs() < TOL, "B/{acc} F00 = {got_f00} (attendu {f00})");
        assert!((got_f20 - f20).abs() < TOL, "B/{acc} F20 = {got_f20} (attendu {f20})");
        assert!(
            (got_f98 - f98_attendu).abs() < TOL,
            "B/{acc} F98 = {got_f98} (attendu {f98_attendu} = −(F00+F20))"
        );
        assert!(
            got_f99.abs() < TOL,
            "B/{acc} F99 = {got_f99} (attendu 0 — la sortante ne fuit pas dans F99)"
        );
        // L'identité se referme : F00 + F20 + F98 = 0.
        assert!(
            (got_f00 + got_f20 + got_f98).abs() < TOL,
            "B/{acc} identité F00+F20+F98 ≠ 0"
        );
    }

    // À consolidated (devise de présentation EUR), F99 de B doit aussi être 0
    // sur tous ses comptes : les écarts F80/F81 générés à la conversion sont
    // absorbés par F98 (terminal, taux close_n).
    let n_nonzero: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry \
             WHERE level='consolidated' AND entity='B' AND flow='F99' \
               AND ABS(amount) >= 0.01",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n_nonzero, 0,
        "aucun F99 non nul pour B au niveau consolidated (écarts absorbés par F98)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  10. Staging par nature — routing des préfixes 2/3/4 vers leur niveau
//      d'injection (post-MVP, mais le mécanisme est en place).
//
//      Les écritures dont le préfixe de nature est 2/3/4 sont injectées
//      directement au niveau correspondant, en sautant les étapes précédentes.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn staging_route_les_prefixes_vers_le_bon_niveau() {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");

    // Natures de test pour les préfixes 2/3/4
    con.execute_batch(
        "INSERT INTO dim_nature VALUES
            ('2TEST','Test reclass skip',NULL),
            ('3TEST','Test convert skip',NULL),
            ('4TEST','Test cons skip',NULL);",
    )
    .expect("seed natures");

    // Écritures de test dans stg_entry
    con.execute_batch(
        "INSERT INTO stg_entry
            (scenario, entity, entry_period, period, account, flow, currency, nature, amount)
         VALUES
            ('REEL','M','2024','2024','100','F20','EUR','2TEST',999.00),
            ('REEL','M','2024','2024','100','F20','EUR','3TEST',888.00),
            ('REEL','M','2024','2024','100','F20','EUR','4TEST',777.00);",
    )
    .expect("seed stg entries");

    run_pipeline(
        &con,
        &ConvertParams::load_params(&con, "REEL").expect("load_params"),
    )
    .expect("run_pipeline");

    // Préfixe 2 : visible à reclassified, invisible à corporate
    let n2_corp: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='corporate' AND substr(nature,1,1)='2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let n2_reclass: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='reclassified' AND nature='2TEST'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n2_corp, 0, "préfixe 2 ne doit pas apparaître à corporate");
    assert!(n2_reclass > 0, "préfixe 2 doit apparaître à reclassified");

    // Préfixe 3 : visible à converted, invisible à corporate et reclassified
    let n3_reclass: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='reclassified' AND substr(nature,1,1)='3'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let n3_conv: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='converted' AND nature='3TEST'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n3_reclass, 0,
        "préfixe 3 ne doit pas apparaître à reclassified"
    );
    assert!(n3_conv > 0, "préfixe 3 doit apparaître à converted");

    // Préfixe 4 : visible à consolidated, invisible ailleurs
    let n4_conv: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='converted' AND substr(nature,1,1)='4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let n4_cons: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level='consolidated' AND nature='4TEST'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n4_conv, 0, "préfixe 4 ne doit pas apparaître à converted");
    assert!(
        n4_cons > 0,
        "préfixe 4 doit apparaître à consolidated"
    );
}
