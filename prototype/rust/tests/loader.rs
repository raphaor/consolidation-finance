//! Test d'intégration du chargeur CSV (`loader::load_all`).
//!
//! Ce chemin — celui du **fresh-init du serveur** (`CONSO_FORCE_RESEED=1` ou base
//! neuve) — n'était pas couvert par `cargo test` auparavant. Il valide notamment
//! la résolution **code→id** des FK migrées en clé technique (chantier B1), en
//! premier lieu `sat_exchange_rate.rate_set`.

use conso_engine::{create_schema, loader::load_all, resolve::resolve_id};
use duckdb::Connection;
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// `load_all` charge `rates.csv` (où `rate_set` est le code `'RATES'`) en
/// résolvant le code vers l'id de `dim_rate_set` : la colonne `sat_exchange_rate.
/// rate_set` contient l'entier, pas le code.
#[test]
fn load_all_resout_rate_set_code_vers_id() {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    load_all(&con, &data_dir()).expect("load_all");

    let rate_id = resolve_id(&con, "dim_rate_set", "RATES")
        .expect("resolve_id")
        .expect("'RATES' chargé depuis rate_sets.csv");

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
