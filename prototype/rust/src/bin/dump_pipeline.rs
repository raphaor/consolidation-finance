//! Outil de diagnostic : rejoue `seed_all` + le pipeline (scénario REEL) et
//! exporte `stg_entry` (saisie) + `fact_entry` à chaque niveau dans un CSV
//! lisible (Excel), pour analyser le décompte de lignes par niveau.
//!
//! Lancer depuis `prototype/rust/` :
//!   cargo run --release --bin dump_pipeline
//! Produit `dump_pipeline.csv` dans le répertoire courant. Affiche aussi un
//! résumé des décomptes (total par niveau, et au reclassified : par entité et
//! par flux) sur la sortie standard.

use conso_engine::{create_schema, run_pipeline, seed_all, ConvertParams};
use duckdb::Connection;
use std::fs::File;
use std::io::{BufWriter, Write};

fn main() {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    let params = ConvertParams::load_params(&con, "REEL").expect("load_params");
    run_pipeline(&con, &params).expect("run_pipeline");

    // --- Export CSV (un onglet plat : colonne `level` = stg / corporate / … ) ---
    let path = "dump_pipeline.csv";
    let f = File::create(path).expect("create csv");
    let mut w = BufWriter::new(f);
    writeln!(
        w,
        "level,entity,account,flow,nature,partner,share,analysis,analysis2,currency,amount"
    )
    .unwrap();

    // Saisie (stg_entry) en premier, taguée 'stg'.
    dump_query(
        &con,
        &mut w,
        "SELECT 'stg' AS level, entity, account, flow, nature, partner, share, \
                analysis, analysis2, currency, amount \
         FROM stg_entry \
         ORDER BY entity, account, flow, nature",
    );
    // fact_entry à tous les niveaux.
    dump_query(
        &con,
        &mut w,
        "SELECT level, entity, account, flow, nature, partner, share, \
                analysis, analysis2, currency, amount \
         FROM fact_entry \
         ORDER BY CASE level \
             WHEN 'corporate' THEN 1 WHEN 'reclassified' THEN 2 \
             WHEN 'converted' THEN 3 WHEN 'consolidated' THEN 4 END, \
             entity, account, flow, nature, partner",
    );
    w.flush().unwrap();
    println!("CSV écrit : {path}\n");

    // --- Résumé console ---
    println!("Lignes par niveau (fact_entry) :");
    print_counts(
        &con,
        "SELECT level, COUNT(*) FROM fact_entry GROUP BY level \
         ORDER BY CASE level WHEN 'corporate' THEN 1 WHEN 'reclassified' THEN 2 \
             WHEN 'converted' THEN 3 WHEN 'consolidated' THEN 4 END",
    );

    println!("\nReclassified — par entité :");
    print_counts(
        &con,
        "SELECT entity, COUNT(*) FROM fact_entry WHERE level='reclassified' \
         GROUP BY entity ORDER BY entity",
    );

    println!("\nReclassified — par flux (F99 = clôtures reconstruites) :");
    print_counts(
        &con,
        "SELECT flow, COUNT(*) FROM fact_entry WHERE level='reclassified' \
         GROUP BY flow ORDER BY flow",
    );

    println!("\nReclassified — par (entité, flux) :");
    print_counts(
        &con,
        "SELECT entity || ' / ' || flow, COUNT(*) FROM fact_entry \
         WHERE level='reclassified' GROUP BY entity, flow ORDER BY entity, flow",
    );
}

fn dump_query<W: Write>(con: &Connection, w: &mut W, sql: &str) {
    let mut stmt = con.prepare(sql).expect("prepare");
    let mut rows = stmt.query([]).expect("query");
    while let Some(row) = rows.next().expect("row") {
        let cell = |i: usize| -> String {
            // Tout en texte ; NULL -> vide. amount (col 10) lue en f64.
            match row.get_ref(i).unwrap() {
                duckdb::types::ValueRef::Null => String::new(),
                duckdb::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                _ => {
                    // numérique / décimal : passe par f64 pour amount, sinon to-string brut
                    if let Ok(s) = row.get::<_, String>(i) {
                        s
                    } else if let Ok(x) = row.get::<_, f64>(i) {
                        format!("{x}")
                    } else {
                        String::new()
                    }
                }
            }
        };
        let amount: f64 = row.get(10).unwrap_or(0.0);
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{:.2}",
            cell(0), cell(1), cell(2), cell(3), cell(4),
            cell(5), cell(6), cell(7), cell(8), cell(9), amount
        )
        .unwrap();
    }
}

fn print_counts(con: &Connection, sql: &str) {
    let mut stmt = con.prepare(sql).expect("prepare");
    let mut rows = stmt.query([]).expect("query");
    let mut total = 0i64;
    while let Some(row) = rows.next().expect("row") {
        let label: String = row.get(0).unwrap();
        let n: i64 = row.get(1).unwrap();
        total += n;
        println!("  {label:<22} {n:>4}");
    }
    println!("  {:<22} {:>4}", "TOTAL", total);
}
