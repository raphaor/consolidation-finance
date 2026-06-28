//! Test d'intégration de l'import JSON (`export::import_bundle`).
//!
//! Ce chemin — celui du **fresh-init du serveur** (`CONSO_SEED_JSON` ou base
//! neuve) — était historiquement couvert via `loader::load_all` sur les CSV.
//! Depuis la migration T1-T5 (cf. `docs/PLAN_MIGRATION_CSV_JSON.md`), le seed
//! initial transite par un paquet JSON : on teste donc la résolution **code→id**
//! des FK migrées en clé technique (chantier B1), en premier lieu
//! `sat_exchange_rate.rate_set`.

use conso_engine::{create_schema, export::import_bundle, resolve::resolve_id};
use duckdb::Connection;
use std::collections::HashSet;
use std::path::PathBuf;

fn seed_json_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/seed.json")
}

/// Charge le paquet JSON de seed via `import_bundle` dans une base en mémoire.
fn load_seed() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    let raw = std::fs::read_to_string(seed_json_path())
        .expect("tests/fixtures/seed.json doit exister (lancer `cargo run --bin conso-gen-seed -- --csv-dir data --out tests/fixtures/seed.json` pour le régénérer)");
    let bundle: serde_json::Value =
        serde_json::from_str(&raw).expect("seed.json doit être un JSON valide");
    let excluded: HashSet<&str> = HashSet::new();
    import_bundle(&con, &bundle, &excluded).expect("import_bundle");
    con
}

/// `import_bundle` réinsère `sat_exchange_rate` en résolvant le code `RATES`
/// vers l'id de `dim_rate_set` : la colonne `sat_exchange_rate.rate_set` contient
/// l'entier, pas le code.
#[test]
fn import_bundle_resout_rate_set_code_vers_id() {
    let con = load_seed();

    let rate_id = resolve_id(&con, "dim_rate_set", "RATES")
        .expect("resolve_id")
        .expect("'RATES' présent dans le paquet");

    // Au moins un taux rattaché à l'id résolu.
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM sat_exchange_rate WHERE rate_set = ?",
            [rate_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n > 0, "des taux chargés avec rate_set = id('RATES')");

    // La colonne est bien entière sur toutes les lignes (jamais le code brut).
    let stray: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM sat_exchange_rate \
             WHERE typeof(rate_set) NOT IN ('INTEGER','BIGINT','HUGEINT')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stray, 0, "rate_set est un entier sur toutes les lignes");
}

/// `import_bundle` réinsère `sat_perimeter` en résolvant le code vers l'id de
/// `dim_perimeter_set` : `sat_perimeter.perimeter_set` contient l'entier.
/// (Chantier B1 — flip `sat_perimeter.perimeter_set`.)
#[test]
fn import_bundle_resout_perimeter_set_code_vers_id() {
    let con = load_seed();

    // Au moins un jeu de périmètre chargé.
    let perim_id = con
        .query_row::<i64, _, _>(
            "SELECT id FROM dim_perimeter_set ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // sat_perimeter.perimeter_set contient l'id résolu, pas le code.
    let n: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM sat_perimeter WHERE perimeter_set = ?",
            [perim_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(n > 0, "des lignes périmètre chargées avec perimeter_set = id");

    let stray: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM sat_perimeter \
             WHERE typeof(perimeter_set) NOT IN ('INTEGER','BIGINT','HUGEINT')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stray, 0, "perimeter_set est un entier sur toutes les lignes");
}
