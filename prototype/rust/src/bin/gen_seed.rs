//! Binaire utilitaire : génère un paquet JSON de seed depuis les CSV + seed_demo_*.
//!
//! Usage :
//!     conso-gen-seed --csv-dir data --out tests/fixtures/seed.json
//!
//! Pratique pour (re)générer `tests/fixtures/seed.json` après évolution du
//! schéma ou des seed_demo_*. Le JSON produit est consommé par `conso-server`
//! via `CONSO_SEED_JSON=tests/fixtures/seed.json`.
//!
//! Ce binaire n'est PAS un chemin de production : il matérialise l'état
//! applicatif de référence pour le boot/reset. Il sera supprimé en T5 (une fois
//! les CSV retirés du repo, le JSON de référence vivra dans tests/fixtures/).

use std::collections::HashSet;

use conso_engine::{
    create_schema, export, load_all, seed_demo_attributes, seed_demo_controls, seed_demo_rules,
};
use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        std::process::exit(0);
    }
    if let Err(msg) = validate_args(&args[1..]) {
        eprintln!("conso-gen-seed: {msg}");
        eprintln!();
        eprintln!("Usage: conso-gen-seed --csv-dir <dir> --out <path>");
        eprintln!("Essayez 'conso-gen-seed --help' pour plus d'informations.");
        std::process::exit(2);
    }

    let csv_dir = args
        .iter()
        .position(|a| a == "--csv-dir")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "data".to_string());
    let out_path = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "tests/fixtures/seed.json".to_string());

    println!("▶ Génération du paquet JSON depuis {csv_dir}/ + seed_demo_*…");

    // Base en mémoire : on ne persite pas la base, juste le JSON.
    let con = duckdb::Connection::open_in_memory().expect("✗ DuckDB in-memory");
    create_schema(&con).expect("✗ create_schema");
    load_all(&con, std::path::Path::new(&csv_dir)).expect("✗ load_all");
    seed_demo_rules(&con).expect("✗ seed_demo_rules");
    seed_demo_controls(&con).expect("✗ seed_demo_controls");
    seed_demo_attributes(&con, std::path::Path::new(&csv_dir))
        .expect("✗ seed_demo_attributes");

    // Construit le paquet via la logique partagée d'`export`. On reproduit le
    // parcours de `export::export_all` sans passer par Axum : TABLES + coefficients
    // user + tables dynamiques.
    let bundle = build_bundle(&con);
    let total: usize = bundle
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(|v| v.as_array().map(|a| a.len()))
                .sum()
        })
        .unwrap_or(0);

    if let Some(parent) = std::path::Path::new(&out_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    let json = serde_json::to_string_pretty(&bundle).expect("✗ sérialisation JSON");
    std::fs::write(&out_path, json).expect("✗ écriture fichier");
    println!("   {total} lignes au total → {out_path}");
}

/// Construit le paquet JSON en réutilisant la logique de `export::export_all`
/// (TABLES + coefficients user + tables dynamiques car_*/lst_*).
fn build_bundle(con: &duckdb::Connection) -> Value {
    // On délègue à un wrapper pub sur la logique d'export — pour l'instant on
    // reconstitue l'objet ici pour ne pas ajouter de fonction pub à export.rs.
    use serde_json::{Map, Value as JsonValue};

    let excluded: HashSet<&str> = HashSet::new();
    // Astuce : import_bundle sur la même base (no-op + CHECKPOINT), c'est inutile.
    // On lit directement les tables.
    let _ = excluded;

    let mut obj = Map::new();
    for t in export::TABLES {
        let rows = conso_engine::masterdata::run_query(
            con,
            &format!("SELECT * FROM {t}"),
            Vec::new(),
        )
        .expect("SELECT *");
        obj.insert((*t).to_string(), JsonValue::Array(rows));
    }

    let coef_rows = conso_engine::masterdata::run_query(
        con,
        "SELECT code, libelle, expression, kind FROM dim_coefficient WHERE kind='user'",
        Vec::new(),
    )
    .expect("SELECT coefficients user");
    obj.insert("dim_coefficient".to_string(), JsonValue::Array(coef_rows));

    // Tables dynamiques car_<id> / lst_<id>.
    let char_rows = conso_engine::masterdata::run_query(
        con,
        "SELECT id FROM dim_characteristic ORDER BY id",
        Vec::new(),
    )
    .expect("SELECT dim_characteristic");
    for r in &char_rows {
        if let Some(id) = r.get("id").and_then(JsonValue::as_i64) {
            let rows = conso_engine::masterdata::run_query(
                con,
                &format!("SELECT * FROM car_{id}"),
                Vec::new(),
            )
            .expect("SELECT car_<id>");
            obj.insert(format!("_car:{id}"), JsonValue::Array(rows));
        }
    }
    let list_rows = conso_engine::masterdata::run_query(
        con,
        "SELECT id FROM dim_value_list ORDER BY id",
        Vec::new(),
    )
    .expect("SELECT dim_value_list");
    for r in &list_rows {
        if let Some(id) = r.get("id").and_then(JsonValue::as_i64) {
            let rows = conso_engine::masterdata::run_query(
                con,
                &format!("SELECT * FROM lst_{id}"),
                Vec::new(),
            )
            .expect("SELECT lst_<id>");
            obj.insert(format!("_lst:{id}"), JsonValue::Array(rows));
        }
    }

    let mut meta = Map::new();
    meta.insert(
        "format".to_string(),
        JsonValue::String(export::FORMAT.to_string()),
    );
    obj.insert("_meta".to_string(), JsonValue::Object(meta));

    JsonValue::Object(obj)
}

fn print_help() {
    println!(
        "conso-gen-seed — Génère un paquet JSON de seed depuis les CSV + seed_demo_*.

Materialise l'état applicatif de référence (master data + règles + caractéristiques
+ contrôles + …) en un fichier JSON consommable par conso-server via
CONSO_SEED_JSON=<chemin>.

USAGE
    conso-gen-seed [--csv-dir <dir>] [--out <path>]

ARGUMENTS
    --csv-dir <dir>  Répertoire des CSV sources (défaut : data)
    --out <path>     Fichier JSON à produire (défaut : tests/fixtures/seed.json)

EXEMPLE
    conso-gen-seed --csv-dir data --out tests/fixtures/seed.json"
    );
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let value_flags = ["--csv-dir", "--out"];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-h" || a == "--help" {
            // déjà traité
        } else if value_flags.contains(&a.as_str()) {
            if i + 1 >= args.len() || args[i + 1].starts_with("--") {
                return Err(format!("l'argument '{a}' requiert une valeur"));
            }
            i += 1;
        } else {
            return Err(format!("argument inconnu : '{a}'"));
        }
        i += 1;
    }
    Ok(())
}
