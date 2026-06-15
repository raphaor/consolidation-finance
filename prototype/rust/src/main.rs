#!/usr/bin/env cargo
//! Point d'entrée du moteur de consolidation financière par les flux.
//!
//! Miroir de `prototype/python/run.py`.
//!
//! Enchaîne : création du schéma → seed des données → pipeline 4 étapes →
//! validation → restitution.
//!
//! Valide que la stack Rust + DuckDB compile et s'exécute sur ARM64 (Raspberry Pi).

use conso_engine::{
    create_schema,
    pipeline::run_pipeline,
    report::{bilan_par_flux, compare_levels, print_level_counts, print_validation},
    seed_all, ConvertParams,
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

    // DuckDB en mémoire : base éphémère, idéale pour un prototype.
    let con = match Connection::open_in_memory() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\n✗ ERREUR : impossible d'ouvrir DuckDB in-memory : {e}");
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

    // 2. Chargement des données de test
    println!("\n▶ Chargement des données de test (seed)…");
    if let Err(e) = seed_all(&con) {
        eprintln!("\n✗ ERREUR seed : {e}");
        std::process::exit(1);
    }
    let n_stg: i64 = con
        .query_row("SELECT COUNT(*) FROM stg_entry", [], |row| row.get(0))
        .expect("COUNT stg_entry");
    println!("   {n_stg} écritures brutes chargées dans stg_entry.");

    // 3. Pipeline 4 étapes
    println!("\n▶ Exécution du pipeline (A→B→C→D)…");
    let params = ConvertParams::default();
    let counts = match run_pipeline(&con, &params) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\n✗ ERREUR pipeline : {e}");
            std::process::exit(1);
        }
    };
    let labels = ["corporate", "reclassified", "converted", "consolidated"];
    for (label, n) in labels.iter().zip(counts.iter()) {
        println!("   étape → {label:<13} {n:>4} lignes produites");
    }

    // 4. Restitutions
    let _ = print_level_counts(&con);
    let _ = bilan_par_flux(&con, "consolidated");
    let _ = compare_levels(&con, "400_Resultat");
    let _ = compare_levels(&con, "100_Capital");

    // 5. Validation des identités F99 = F00 + F01 + F20 + F80 + F81 + F98
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
