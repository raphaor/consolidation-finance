#!/usr/bin/env cargo
//! Point d'entrée du moteur de consolidation financière par les flux.
//!
//! Miroir de `prototype/python/run.py`.
//!
//! Enchaîne : création du schéma → chargement CSV → pipeline 4 étapes →
//! validation → restitution.
//!
//! # Arguments CLI
//!
//! - `--db <path>`    : chemin du fichier DuckDB (défaut : `conso.duckdb`).
//! - `--csv-dir <dir>`: répertoire contenant les CSV (défaut : `data`).
//!
//! Valide que la stack Rust + DuckDB compile et s'exécute sur ARM64 (Raspberry Pi).

use conso_engine::{
    create_schema,
    load_all,
    pipeline::run_pipeline,
    report::{bilan_par_flux, compare_levels, print_level_counts, print_validation},
    ConvertParams,
};
use duckdb::Connection;

fn main() {
    let title = "  SCAFFOLD RUST — Moteur de consolidation (Rust + DuckDB sur ARM64)";
    let pad = 86usize.saturating_sub(title.len());
    let centered = format!("{}{}", title, " ".repeat(pad));
    println!();
    println!("╔{}╗", "═".repeat(86));
    println!("║{}║", centered);
    println!("╚{}╝", "═".repeat(86));

    // --- Parsing manuel des arguments (pas de clap pour un prototype) ---
    let args: Vec<String> = std::env::args().collect();
    let db_path = args
        .iter()
        .position(|a| a == "--db")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "conso.duckdb".to_string());
    let csv_dir = args
        .iter()
        .position(|a| a == "--csv-dir")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "data".to_string());

    // DuckDB en fichier : base persistante, supprimable d'un run sur l'autre.
    let con = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\n✗ ERREUR : impossible d'ouvrir DuckDB ({db_path}) : {e}");
            std::process::exit(1);
        }
    };

    // 1. Schéma + affichage des tables créées
    println!("\n▶ Création du schéma DuckDB…");
    if let Err(e) = create_schema(&con) {
        eprintln!("\n✗ ERREUR DDL : {e}");
        std::process::exit(1);
    }

    // Liste des tables créées (validation du DDL sur DuckDB)
    match con
        .prepare("SELECT table_name FROM information_schema.tables WHERE table_schema = 'main' ORDER BY table_name")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<duckdb::Result<Vec<_>>>()
        }) {
        Ok(tables) => {
            println!("   Tables créées ({}):", tables.len());
            for t in &tables {
                println!("     • {t}");
            }
        }
        Err(e) => {
            eprintln!("\n⚠ Impossible de lister les tables : {e}");
        }
    }

    // 2. Chargement des données depuis CSV
    println!("\n▶ Chargement des données depuis CSV ({csv_dir})…");
    if let Err(e) = load_all(&con, std::path::Path::new(&csv_dir)) {
        eprintln!("\n✗ ERREUR chargement CSV : {e}");
        std::process::exit(1);
    }
    let n_stg: i64 = con
        .query_row("SELECT COUNT(*) FROM stg_entry", [], |row| row.get(0))
        .expect("COUNT stg_entry");
    println!("   {n_stg} écritures brutes chargées dans stg_entry.");

    // 3. Pipeline 4 étapes
    println!("\n▶ Exécution du pipeline (A→B→C→D)…");
    let params = match ConvertParams::load_params(&con, "REEL") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("\n✗ ERREUR load_params : {e}");
            std::process::exit(1);
        }
    };
    let report = match run_pipeline(&con, &params) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\n✗ ERREUR pipeline : {e}");
            std::process::exit(1);
        }
    };
    let counts = report.counts();
    let labels = ["corporate", "converted", "consolidated"];
    for (label, n) in labels.iter().zip(counts.iter()) {
        println!("   étape → {label:<13} {n:>4} lignes produites");
    }

    // 4. Restitutions
    let _ = print_level_counts(&con);
    let _ = bilan_par_flux(&con, "consolidated");
    let _ = compare_levels(&con, "12");
    let _ = compare_levels(&con, "101");

    // 5. Validation des identités de reconstruction des clôtures (flux_de_report)
    let ok = match print_validation(&con) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("\n✗ ERREUR validation : {e}");
            std::process::exit(1);
        }
    };

    println!("\n{}", "═".repeat(88));
    if ok {
        println!("  OK — Stack Rust + DuckDB validée sur ARM64 (aarch64).");
    } else {
        println!("  ✗ Identité(s) en échec — voir ci-dessus.");
    }
    println!("{}", "═".repeat(88));

    std::process::exit(if ok { 0 } else { 1 });
}
