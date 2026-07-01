/// Test de régression pour la conversion cross-currency triangulaire.
///
/// Valide : USD → GBP via pivot EUR
/// - Flux F20 (taux moyen) : 1000 × (taux USD→EUR / taux GBP→EUR)
/// - Flux F00 (taux clôture) : 1000 × (taux USD→EUR / taux GBP→EUR)
///
/// Spécif : ETAT_AVANCEMENT.md §299-313

use conso_engine::{create_schema, pipeline::run_pipeline, seed_all, ConvertParams};
use duckdb::Connection;

// Taux de change 2024 depuis seed.json :
// - USD→EUR : moyen=0.95, close=0.90
// - GBP→EUR : moyen=1.18, close=1.12
//
// USD→GBP via EUR = USD/EUR ÷ GBP/EUR
// - F20 (moyen) : 1000 × (0.95 / 1.18) = 805.0847... ≈ 805.08
// - F00 (close) : 1000 × (0.90 / 1.12) = 803.5714... ≈ 803.57

/// Crée une DB en mémoire, crée le schéma, charge le seed, ajoute une
/// consolidation GBP avec USD→entries (entité TEST), lance le pipeline.
fn setup_cross_currency() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");

    // Créer une entité TEST en USD (n'existe pas dans le seed)
    con.execute(
        "INSERT INTO dim_entity (code, libelle, devise_fonctionnelle, entite_parent, statut)
         VALUES ('TEST', 'Test entity',
                 (SELECT id FROM dim_currency WHERE code_iso='USD'),
                 NULL, 'actif')",
        [],
    )
    .expect("insert entity TEST");

    // Ajouter TEST au périmètre PERIM_REEL pour 2024
    con.execute(
        "INSERT INTO sat_perimeter
         (perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie)
         VALUES
         ((SELECT id FROM dim_perimeter_set WHERE code='PERIM_REEL'),
          'TEST',
          '2024',
          (SELECT id FROM dim_method WHERE code='globale'),
          1.0,
          1.0,
          FALSE,
          FALSE)",
        [],
    )
    .expect("insert TEST into perimeter");

    // Créer une consolidation avec presentation_currency=GBP
    con.execute(
        "INSERT INTO dim_consolidation
         (libelle, phase, exercice, perimeter_set, variant, presentation_currency,
          rate_set, rate_period, perimeter_period, statut)
         VALUES
         ('Test cross-currency',
          (SELECT id FROM dim_scenario_category WHERE code='REEL'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_perimeter_set WHERE code='PERIM_REEL'),
          (SELECT id FROM dim_variant WHERE code='BASE'),
          (SELECT id FROM dim_currency WHERE code_iso='GBP'),
          (SELECT id FROM dim_rate_set WHERE code='RATES'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_period WHERE code='2024'),
          'ouvert')",
        [],
    )
    .expect("insert consolidation GBP");

    // Récupérer l'id de la consolidation créée
    let conso_id: i64 = con
        .query_row(
            "SELECT id FROM dim_consolidation
             WHERE presentation_currency = (SELECT id FROM dim_currency WHERE code_iso='GBP')
               AND phase = (SELECT id FROM dim_scenario_category WHERE code='REEL')
               AND exercice = (SELECT id FROM dim_period WHERE code='2024')",
            [],
            |r| r.get(0),
        )
        .expect("get consolidation GBP id");

    // Insérer des écritures en USD (entité TEST)
    // F20 (variation, taux moyen) et F00 (clôture, taux clôture)
    con.execute(
        "INSERT INTO stg_entry
         (phase, entity, entry_period, period, account, flow, currency, nature, amount)
         VALUES
         ((SELECT id FROM dim_scenario_category WHERE code='REEL'),
          (SELECT id FROM dim_entity WHERE code='TEST'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_account WHERE code='60'),
          (SELECT id FROM dim_flow WHERE code='F20'),
          (SELECT code_iso FROM dim_currency WHERE code_iso='USD'),
          (SELECT id FROM dim_nature WHERE code='0LIASS'),
          1000.0),
         ((SELECT id FROM dim_scenario_category WHERE code='REEL'),
          (SELECT id FROM dim_entity WHERE code='TEST'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_period WHERE code='2024'),
          (SELECT id FROM dim_account WHERE code='60'),
          (SELECT id FROM dim_flow WHERE code='F00'),
          (SELECT code_iso FROM dim_currency WHERE code_iso='USD'),
          (SELECT id FROM dim_nature WHERE code='0LIASS'),
          1000.0)",
        [],
    )
    .expect("insert USD entries");

    // Exécuter le pipeline
    let params = ConvertParams::load_params(&con, conso_id).expect("load_params");
    run_pipeline(&con, &params).expect("run_pipeline");

    con
}

#[test]
fn test_cross_currency_triangulaire() {
    let con = setup_cross_currency();

    // Debug : voir ce qu'il y a dans fact_entry converted pour TEST
    println!("\n=== Debug : fact_entry converted (entité TEST) ===");
    let mut stmt = con
        .prepare(
            "SELECT
                f.code as flow,
                e.amount as amount
             FROM fact_entry e
             JOIN dim_flow f ON e.flow = f.id
             JOIN dim_entity ent ON e.entity = ent.id
             WHERE e.level = 'converted'
               AND ent.code = 'TEST'
             ORDER BY f.code",
        )
        .expect("prepare debug");

    let rows = stmt
        .query_map([], |r| {
            let flow: String = r.get(0)?;
            let amount: f64 = r.get(1)?;
            Ok((flow, amount))
        })
        .expect("query debug");

    for row in rows {
        let (flow, amount) = row.unwrap();
        println!("  {}: {}", flow, amount);
    }

    // Lire les montants au niveau converted pour l'entité TEST
    let (f20_amount, f00_amount): (f64, f64) = con
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN f.code = 'F20' THEN e.amount ELSE 0 END), 0) as f20,
                COALESCE(SUM(CASE WHEN f.code = 'F00' THEN e.amount ELSE 0 END), 0) as f00
             FROM fact_entry e
             JOIN dim_flow f ON e.flow = f.id
             JOIN dim_entity ent ON e.entity = ent.id
             WHERE e.level = 'converted'
               AND ent.code = 'TEST'",
            [],
            |r| {
                let f20: f64 = r.get(0)?;
                let f00: f64 = r.get(1)?;
                Ok((f20, f00))
            },
        )
        .expect("query converted amounts");

    // Valeurs attendues (USD 1000 → GBP via EUR)
    // F20 (taux moyen) : 1000 × (0.95 / 1.18) ≈ 805.08
    // F00 (taux clôture) : 1000 × (0.90 / 1.12) ≈ 803.57
    let expected_f20 = 1000.0 * 0.95 / 1.18; // ≈ 805.0847
    let expected_f00 = 1000.0 * 0.90 / 1.12; // ≈ 803.5714

    // Tolérance 0.01 (le moteur stocke en DECIMAL(18,2))
    let tol = 0.01;

    let diff_f20 = (f20_amount - expected_f20).abs();
    let diff_f00 = (f00_amount - expected_f00).abs();

    assert!(
        diff_f20 < tol,
        "Conversion F20 incorrecte: expected ≈ {}, got {} (diff {})",
        expected_f20,
        f20_amount,
        diff_f20
    );

    assert!(
        diff_f00 < tol,
        "Conversion F00 incorrecte: expected ≈ {}, got {} (diff {})",
        expected_f00,
        f00_amount,
        diff_f00
    );
}