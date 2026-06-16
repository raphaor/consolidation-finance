//! Tests d'intégration du pipeline de consolidation.
//!
//! Chaque test ouvre une DuckDB **en mémoire** (isolation totale, pas de fichier
//! à nettoyer), crée le schéma, charge le jeu de seed (groupe M/A/B), exécute le
//! pipeline A→B→C→D puis vérifie :
//!   - les comptes par niveau (corporate/reclassified/converted/consolidated) ;
//!   - les montants F99 attendus par compte au niveau consolidated ;
//!   - l'identité de reconstruction via `validate` ;
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
    run_pipeline(&con, &ConvertParams::default()).expect("run_pipeline");
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
    assert_eq!(level_count(&con, "corporate"), 16, "niveau corporate");
    assert_eq!(level_count(&con, "reclassified"), 14, "niveau reclassified");
    assert_eq!(level_count(&con, "converted"), 19, "niveau converted");
    assert_eq!(level_count(&con, "consolidated"), 19, "niveau consolidated");
}

// ─────────────────────────────────────────────────────────────────────────────
//  2. Montants F99 attendus au niveau consolidated
//     (détecte toute régression d'agrégation / conversion / consolidation)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn montants_f99_consolidated_attendus() {
    let con = setup();

    let capital = f99_consolidated(&con, "100_Capital");
    let immo = f99_consolidated(&con, "200_Immobilisations");
    let stocks = f99_consolidated(&con, "300_Stocks");
    let resultat = f99_consolidated(&con, "400_Resultat");

    assert!(
        (capital - 18_980.00).abs() < TOL,
        "100_Capital consolidated F99 = {capital} (attendu 18980.00)"
    );
    assert!(
        (immo - 27_116.00).abs() < TOL,
        "200_Immobilisations consolidated F99 = {immo} (attendu 27116.00)"
    );
    assert!(
        (stocks - 3_000.00).abs() < TOL,
        "300_Stocks consolidated F99 = {stocks} (attendu 3000.00)"
    );
    assert!(
        (resultat - 9_774.00).abs() < TOL,
        "400_Resultat consolidated F99 = {resultat} (attendu 9774.00)"
    );
}

#[test]
fn comptes_attendus_presents_au_niveau_consolidated() {
    let con = setup();
    for acc in &[
        "100_Capital",
        "200_Immobilisations",
        "300_Stocks",
        "400_Resultat",
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
//  3. Identité de reconstruction F99 (validateur du crate)
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
//     NB : B (GBP) est sortante → tous ses flux sont collapsés en F98 (terminal,
//     sans écart). Seule A (USD, entrante) génère donc des F80/F81 ici.
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

    // B (GBP, sortante) : tout est en F98 (terminal) → aucun écart.
    let n_b = flow_rows(&con, "consolidated", &["F80", "F81"], Some(&["B"]));
    assert_eq!(n_b, 0, "B (sortante, F98 terminal) ne doit générer aucun écart");
}

// ─────────────────────────────────────────────────────────────────────────────
//  5. Cohérence de la conversion : le montant converti d'A + son écart
//     reconstitue le montant × taux_close_n. Vérification indépendante du
//     validateur (qui, lui, reconstruit F99 par somme — triviale par construction).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn conversion_reconstitue_montant_au_taux_close_n() {
    let con = setup();

    // Pour A (USD), compte 100_Capital, flux F01 (ex F00 entrant) :
    //   converted(F01) + écart(F80) doit valoir fonctionnel × taux_close_n (0.90).
    let f01_pres = flow_sum(&con, "consolidated", "100_Capital", &["F01"]);
    let f80_pres = flow_sum(&con, "consolidated", "100_Capital", &["F80"]);

    // Côté fonctionnel (reclassified) : F01 de A = 5000 USD.
    let f01_func: f64 = con
        .query_row(
            "SELECT COALESCE(SUM(amount),0) FROM fact_entry \
             WHERE level='reclassified' AND account='100_Capital' AND flow='F01' AND entity='A'",
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
        "conversion A/100_Capital : reconstruit {reconstruit} ≠ attendu {attendu}"
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
    let accounts = ["100_Capital", "200_Immobilisations", "300_Stocks", "400_Resultat"];
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
    run_pipeline(&con, &ConvertParams::default()).expect("re-run pipeline");

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
