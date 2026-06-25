//! Benchmark de performance du pipeline de consolidation sur gros volumes.
//!
//! Génère un jeu de données réaliste (≈ 60 entités, ≈ 200 comptes, plusieurs
//! devises, variations de périmètre) puis mesure la durée de chaque étape du
//! pipeline (A→B→C→D) sur une DuckDB **fichier** (le cas réel, où le disque est
//! le bottleneck).
//!
//! # Arguments
//!
//! - `--rows <N>`   : nombre d'écritures brutes à générer dans `stg_entry`
//!                    (défaut : 1 000 000).
//! - `--db <path>`  : chemin du fichier DuckDB (défaut : `$TEMP/conso_bench.duckdb`).
//!
//! La génération du volume utilise `range()` en SQL natif DuckDB (très rapide,
//! rien n'est matérialisé côté Rust), ce qui mesure honnêtement le pipeline.
//!
//! # Lancement
//!
//! ```bash
//! cargo run --release --bin conso-bench -- --rows 1000000
//! ```

use conso_engine::{
    create_schema, pipeline::run_pipeline, validate::validate_consolidated, ConvertParams,
};
use duckdb::Connection;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
//  Paramètres du jeu de données généré
// ─────────────────────────────────────────────────────────────────────────────
const N_ENTITIES: i64 = 60;
const N_ACCOUNTS: i64 = 200;
const SCENARIO: &str = "REEL";
const ENTRY_PERIOD: &str = "2024";

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        std::process::exit(0);
    }
    if let Err(msg) = validate_args(&args[1..]) {
        eprintln!("conso-bench: {msg}");
        eprintln!();
        eprintln!("Usage: conso-bench [--rows <N>] [--db <path>]");
        eprintln!("Essayez 'conso-bench --help' pour plus d'informations.");
        std::process::exit(2);
    }

    let rows: usize = arg_value(&args, "--rows")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let db_path = arg_value(&args, "--db").unwrap_or_else(default_db_path);

    println!();
    println!("╔══ CONSO-BENCH — Pipeline de consolidation (gros volumes) ══╗");
    println!("║  fichier DuckDB : {:<41}║", truncate(&db_path, 41));
    println!("║  écritures cible : {:<40} ║", format!("{rows} lignes"));
    println!("╚════════════════════════════════════════════════════════════╝");

    // --- Nettoyage du fichier existant (run propre) ---
    clean_db_file(&db_path);

    // --- Ouverture de la connexion fichier ---
    let con = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\n✗ Impossible d'ouvrir DuckDB ({db_path}) : {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = create_schema(&con) {
        eprintln!("\n✗ ERREUR DDL : {e}");
        std::process::exit(1);
    }

    // --- 1. Génération des dimensions + satellites ---
    let t = Instant::now();
    if let Err(e) = gen_dimensions(&con) {
        eprintln!("\n✗ ERREUR génération dimensions : {e}");
        std::process::exit(1);
    }
    if let Err(e) = gen_satellites(&con) {
        eprintln!("\n✗ ERREUR génération satellites : {e}");
        std::process::exit(1);
    }
    println!(
        "\n▶ Dimensions générées ({} entités, {} comptes, 5 devises) en {:.0} ms",
        N_ENTITIES,
        N_ACCOUNTS,
        ms(t.elapsed())
    );

    // --- 2. Génération des écritures brutes (gros volume, en SQL natif) ---
    let t = Instant::now();
    if let Err(e) = gen_staging(&con, rows) {
        eprintln!("\n✗ ERREUR génération stg_entry : {e}");
        std::process::exit(1);
    }
    let gen_ms = ms(t.elapsed());

    let n_stg: i64 = con
        .query_row("SELECT COUNT(*) FROM stg_entry", [], |row| row.get(0))
        .expect("COUNT stg_entry");
    println!(
        "▶ stg_entry généré : {n_stg} lignes en {:.0} ms ({:.0} k lignes/s généré)",
        gen_ms,
        (n_stg as f64 / gen_ms.max(1.0)) * 1000.0 / 1000.0
    );

    // --- 3. Exécution du pipeline mesuré ---
    println!("\n▶ Exécution du pipeline A→C→D…");
    let params = match ConvertParams::load_params(&con, 1) {
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

    // --- 4. Validation clôtures + invariants ---
    let closures_ok = match check_identity(&con) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("\n✗ ERREUR validation : {e}");
            std::process::exit(1);
        }
    };

    // --- 5. Rapport ---
    print_report(&report, n_stg, closures_ok);

    // Nettoyage du fichier de bench (optionnel : on garde le fichier pour
    // inspection, mais on libère la connexion à la fermeture).
    drop(con);

    println!("\n  (fichier conservé : {db_path})");
    std::process::exit(if closures_ok { 0 } else { 1 });
}

// ─────────────────────────────────────────────────────────────────────────────
//  Génération des données (SQL natif DuckDB)
// ─────────────────────────────────────────────────────────────────────────────

/// Dimensions : scénarios, entités, périodes, comptes, flux, devises.
fn gen_dimensions(con: &Connection) -> duckdb::Result<()> {
    con.execute_batch(&format!(
        "
        -- Config applicative + nouvelles dimensions référentielles (SPEC v2).
        INSERT INTO app_config VALUES ('pivot_currency', 'EUR');

        INSERT INTO dim_scenario_category (code, libelle) VALUES
            ('REEL', 'Réel'),
            ('BUDGET', 'Budget'),
            ('PREV', 'Prévision');

        INSERT INTO dim_variant (code, libelle) VALUES ('BASE', 'Base');

        INSERT INTO dim_rate_set (code, libelle) VALUES ('RATES', 'Taux réels');

        INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PERIM_REEL', 'Périmètre réel 2024');

        -- dim_consolidation : objet composite (PK technique auto `id`). L'id est
        -- alloué par nextval (déterministe 1 sur base fraîche) — `load_params(&con, 1)`
        -- pointe donc sur cette consolidation. Les colonnes `perimeter_period` et
        -- `rate_period` valent l'exercice ('2024').
        INSERT INTO dim_consolidation
            (id, libelle, phase, exercice, perimeter_set, variant, presentation_currency,
             perimeter_period, rate_set, rate_period, ruleset_code, a_nouveau_consolidation_id, statut)
        VALUES
            (nextval('seq_consolidation'), 'Réel', 'REEL', '2024', 'PERIM_REEL',
             (SELECT id FROM dim_variant WHERE code = 'BASE'), 'EUR',
             '2024', 'RATES', '2024', NULL, NULL, 'ouvert');

        INSERT INTO dim_period (code, libelle, type, date_debut, date_fin, statut) VALUES
            ('2023','Exercice 2023','exercice','2023-01-01','2023-12-31','clôturé'),
            ('2024','Exercice 2024','exercice','2024-01-01','2024-12-31','ouvert');

        -- Entités : M (mère, EUR) + filiales réparties sur 5 devises.
        INSERT INTO dim_entity (code, libelle, devise_fonctionnelle, entite_parent, statut)
        SELECT
            CASE WHEN i = 0 THEN 'M' ELSE 'E' || LPAD(CAST(i AS VARCHAR), 02, '0') END,
            'Entite ' || CAST(i AS VARCHAR),
            CASE (i % 5)
                WHEN 0 THEN 'EUR'
                WHEN 1 THEN 'USD'
                WHEN 2 THEN 'GBP'
                WHEN 3 THEN 'CHF'
                ELSE        'JPY'
            END,
            CASE WHEN i = 0 THEN NULL ELSE 'M' END,
            'actif'
        FROM range(0, {N_ENTITIES}) t(i);

        -- Plan de compte : mix bilan / resultat / flux (sous_classe NULLABLE,
        -- laissée à NULL pour les comptes synthétiques).
        INSERT INTO dim_account (code, libelle, classe, sous_classe)
        SELECT
            'ACC_' || LPAD(CAST(i AS VARCHAR), 04, '0'),
            'Compte ' || CAST(i AS VARCHAR),
            CASE
                WHEN i % 5 IN (0,1) THEN 'bilan'
                WHEN i % 5 IN (2,3) THEN 'resultat'
                ELSE                     'flux'
            END,
            NULL
        FROM range(0, {N_ACCOUNTS}) t(i);

        INSERT INTO dim_flow (code, libelle) VALUES
            ('F00','Ouverture'),
            ('F01','Entrée périmètre'),
            ('F20','Variation'),
            ('F80','Écart conv. ouverture'),
            ('F81','Écart conv. variation'),
            ('F98','Sortie périmètre'),
            ('F99','Clôture');

        INSERT INTO dim_flow_scheme (code, libelle) VALUES
            ('BILAN','Schéma bilan'),
            ('RESULTAT','Schéma résultat');

        -- Articulation complète par schéma (cf. Q32 / v_flow_behavior).
        INSERT INTO sat_flow_scheme_item
            (scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau) VALUES
            ('BILAN','F00','close_n1','F80','F99',NULL),
            ('BILAN','F01','close_n1','F80','F99',NULL),
            ('BILAN','F20','avg','F81','F99',NULL),
            ('BILAN','F80','close_n',NULL,'F99',NULL),
            ('BILAN','F81','close_n',NULL,'F99',NULL),
            ('BILAN','F98','close_n',NULL,'F99',NULL),
            ('BILAN','F99','close_n',NULL,'F99',NULL),
            ('RESULTAT','F00','avg',NULL,'F99',NULL),
            ('RESULTAT','F01','avg',NULL,'F99',NULL),
            ('RESULTAT','F20','avg',NULL,'F99',NULL),
            ('RESULTAT','F80','close_n',NULL,'F99',NULL),
            ('RESULTAT','F81','close_n',NULL,'F99',NULL),
            ('RESULTAT','F98','avg',NULL,'F99',NULL),
            ('RESULTAT','F99','avg',NULL,'F99',NULL);

        INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES
            ('EUR','Euro',2),
            ('USD','Dollar US',2),
            ('GBP','Livre sterling',2),
            ('CHF','Franc suisse',2),
            ('JPY','Yen',0);

        INSERT INTO dim_nature (code, libelle, rules) VALUES
            ('0LIASS','Liasse',NULL),
            ('1AJUST','Ajustement',NULL);
        "
    ))?;
    Ok(())
}

/// Tables satellites : périmètre de consolidation + taux de change.
fn gen_satellites(con: &Connection) -> duckdb::Result<()> {
    con.execute_batch(
        "
        -- Périmètre (REEL / 2024) : globales par défaut, quelques proportionnelles,
        -- ~10 % entrantes et ~10 % sortantes (pour exercer F01 / F98).
        INSERT INTO sat_perimeter
            (perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie)
        SELECT
            'PERIM_REEL', e.code, '2024',
            CASE WHEN e.rn % 7 = 0 THEN 'proportionnelle' ELSE 'globale' END,
            CASE WHEN e.rn % 7 = 0 THEN 0.5000 ELSE 1.0000 END,
            CASE WHEN e.rn % 7 = 0 THEN 0.5000 ELSE 1.0000 END,
            CASE WHEN e.rn % 10 = 1 THEN TRUE ELSE FALSE END,
            CASE WHEN e.rn % 10 = 2 THEN TRUE ELSE FALSE END
        FROM (
            SELECT ROW_NUMBER() OVER () - 1 AS rn, code
            FROM dim_entity ORDER BY code
        ) e;

        -- Taux de change vers le pivot EUR : 4 devises non-EUR × 2 exercices.
        -- Tous rattachés au jeu 'RATES' (SPEC_SCENARIO_V2.md §2).
        -- `taux_ouverture` (N) = clôture N-1 : résout `close_n1` (F00/F01) sans
        -- période antérieure. Porté par la période N (ici 2024 = clôture 2023).
        INSERT INTO sat_exchange_rate
            (rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture) VALUES
            ('RATES','USD','2023', 0.92000000, NULL,        NULL),
            ('RATES','USD','2024', 0.90000000, 0.95000000, 0.92000000),
            ('RATES','GBP','2023', 1.15000000, NULL,        NULL),
            ('RATES','GBP','2024', 1.12000000, 1.18000000, 1.15000000),
            ('RATES','CHF','2023', 0.98000000, NULL,        NULL),
            ('RATES','CHF','2024', 1.05000000, 1.02000000, 0.98000000),
            ('RATES','JPY','2023', 0.00650000, NULL,        NULL),
            ('RATES','JPY','2024', 0.00620000, 0.00680000, 0.00650000);
        ",
    )?;
    Ok(())
}

/// Écritures brutes : `rows` lignes dans stg_entry via `range()` DuckDB.
///
/// Chaque ligne `i` est rattachée à une entité (`i % N_ENTITIES`) et un compte
/// (`(i / N_ENTITIES) % N_ACCOUNTS`) — les deux expressions sont **découplées**
/// pour balayer toutes les combinaisons (entité × compte) et éviter qu'une
/// corrélation artefactuelle ne réduise l'agrégation à quelques centaines de
/// lignes. Le flow alterne sur un cycle plus long. La génération est 100 % SQL
/// (rien n'est matérialisé en Rust).
fn gen_staging(con: &Connection, rows: usize) -> duckdb::Result<()> {
    // Index des entités/comptes pour le JOIN.
    con.execute_batch(
        "CREATE TEMP TABLE ent_idx AS
            SELECT ROW_NUMBER() OVER () - 1 AS rn, code, devise_fonctionnelle
            FROM dim_entity ORDER BY code;
         CREATE TEMP TABLE acc_idx AS
            SELECT ROW_NUMBER() OVER () - 1 AS rn, code
            FROM dim_account ORDER BY code;",
    )?;

    // Période du cycle entité×compte = N_ENTITIES × N_ACCOUNTS = 12000.
    // On fait varier le flow sur un cycle double (24000) pour produire F00 et F20.
    // NB : en DuckDB, `/` est une division *flottante* (contrairement à
    // PostgreSQL). On utilise `//` (division entière floor) pour les index.
    let flow_cycle = N_ENTITIES * N_ACCOUNTS * 2;
    // CTE explicite : on matérialise les index (ent_rn / acc_rn) avant les JOIN.
    let sql = format!(
        "
        INSERT INTO stg_entry
            (phase, entity, entry_period, period, account, flow, currency, nature,
             partner, share, analysis, analysis2, amount)
        WITH gen AS (
            SELECT g.i,
                   g.i % {N_ENTITIES}                         AS ent_rn,
                   (g.i // {N_ENTITIES}) % {N_ACCOUNTS}       AS acc_rn,
                   (g.i // {flow_cycle}) % 2                  AS fl
            FROM range(0, {rows}) AS g(i)
        )
        SELECT
            '{SCENARIO}',
            e.code,
            '{ENTRY_PERIOD}', '{ENTRY_PERIOD}',
            a.code,
            CASE WHEN gen.fl = 0 THEN 'F00' ELSE 'F20' END,
            e.devise_fonctionnelle,
            '0LIASS',
            NULL, NULL, NULL,
            'BENCH-' || CAST(gen.i AS VARCHAR),
            CAST(((gen.i % 9000) + 1000) AS DECIMAL(18,2))
        FROM gen
        JOIN ent_idx e ON e.rn = gen.ent_rn
        JOIN acc_idx a ON a.rn = gen.acc_rn;
        "
    );
    con.execute_batch(&sql)?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Validation : identités de clôture + invariants non triviaux
// ─────────────────────────────────────────────────────────────────────────────

/// Vérifie les identités de clôture (via le validateur du crate) et un invariant
/// réel : les écarts F80/F81 doivent être absents du niveau en devise
/// fonctionnelle (corporate). Ces écarts n'existent qu'après l'étape C.
fn check_identity(con: &Connection) -> duckdb::Result<bool> {
    // (a) validateur du crate — tous les comptes doivent passer.
    let checks = validate_consolidated(con)?;
    let closures_ok = !checks.is_empty() && checks.iter().all(|c| c.ok);
    if !closures_ok {
        let failed: Vec<&str> = checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| c.account.as_str())
            .collect();
        eprintln!(
            "\n✗ Identité de clôture en échec pour : {}",
            failed.join(", ")
        );
    }

    // (b) invariant structurel : F80/F81 absents du niveau fonctionnel.
    for lvl in &["corporate"] {
        let n: i64 = con.query_row(
            "SELECT COUNT(*) FROM fact_entry WHERE level = ? AND flow IN ('F80','F81')",
            [lvl],
            |row| row.get(0),
        )?;
        if n != 0 {
            eprintln!("\n✗ Invariant F80/F81 en échec : {n} lignes d'écart au niveau {lvl}");
            return Ok(false);
        }
    }

    // (c) invariant structurel : F80/F81 présents au niveau consolidated (entités non-EUR).
    let n_consol: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = 'consolidated' AND flow IN ('F80','F81')",
        [],
        |row| row.get(0),
    )?;
    if n_consol == 0 {
        eprintln!("\n✗ Aucun écart F80/F81 au niveau consolidated (conversion suspecte)");
        return Ok(false);
    }

    Ok(closures_ok)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Rapport final
// ─────────────────────────────────────────────────────────────────────────────

fn print_report(report: &conso_engine::pipeline::PipelineReport, n_stg: i64, closures_ok: bool) {
    println!();
    println!("{}", "═".repeat(70));
    println!("  RAPPORT DE PERFORMANCE");
    println!("{}", "═".repeat(70));
    println!(
        "  {:<16}{:>14}{:>14}{:>14}",
        "Étape (niveau)", "Lignes", "Durée (ms)", "Débit (k/s)"
    );
    println!("  {}", "─".repeat(58));
    for s in &report.steps {
        let throughput = (s.rows as f64 / s.ms.max(1.0)) * 1000.0 / 1000.0;
        println!(
            "  {:<16}{:>14}{:>14.1}{:>14.0}",
            s.level, s.rows, s.ms, throughput
        );
    }
    println!("  {}", "─".repeat(58));
    let total_throughput = (n_stg as f64 / report.total_ms.max(1.0)) * 1000.0;
    println!(
        "  {:<16}{:>14}{:>14.1}{:>14.0}",
        "TOTAL",
        n_stg,
        report.total_ms,
        total_throughput / 1000.0
    );
    println!();
    println!("  Temps total pipeline : {:.3} s", report.total_sec());
    println!(
        "  Débit global         : {:.0} k lignes stg/s  ({:.0} lignes/s)",
        total_throughput / 1000.0,
        total_throughput
    );
    println!();
    let verdict = if closures_ok {
        "✓ OK — identités de clôture + invariants tenus"
    } else {
        "✗ ÉCHEC"
    };
    println!("  Verdict clôtures : {verdict}");
    println!("{}", "═".repeat(70));
}

// ─────────────────────────────────────────────────────────────────────────────
//  Utilitaires
// ─────────────────────────────────────────────────────────────────────────────

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn default_db_path() -> String {
    let tmp = std::env::var("TEMP")
        .or_else(|_| std::env::var("TMPDIR"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::Path::new(&tmp)
        .join("conso_bench.duckdb")
        .to_string_lossy()
        .into_owned()
}

fn clean_db_file(path: &str) {
    let p = std::path::Path::new(path);
    let _ = std::fs::remove_file(p);
    // DuckDB peut laisser un fichier .wal collé.
    let wal = format!("{}.wal", path);
    let _ = std::fs::remove_file(&wal);
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (n - 1)..])
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Aide (--help / -h) et validation des arguments
// ─────────────────────────────────────────────────────────────────────────────

fn print_help() {
    println!(
        "conso-bench — Benchmark de performance du pipeline sur gros volumes.

Génère un jeu réaliste (≈ 60 entités × 200 comptes, 5 devises, variations de
périmètre), exécute le pipeline A→B→C→D sur une DuckDB fichier, puis valide les
identités de clôture et les invariants F80/F81.

USAGE
    conso-bench [--rows <N>] [--db <path>]

ARGUMENTS
    --rows <N>     Nombre d'écritures brutes générées dans stg_entry (défaut : 1000000)
    --db <path>    Fichier DuckDB (défaut : $TEMP/conso_bench.duckdb)

EXEMPLE
    conso-bench --rows 5000000"
    );
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let value_flags = ["--rows", "--db"];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-h" || a == "--help" {
            // déjà traité avant l'appel
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
