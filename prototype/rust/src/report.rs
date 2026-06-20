//! Sorties du prototype : bilan par flux, comparaison des niveaux, volumes.
//!
//! Miroir de `prototype/python/conso/report.py`.
//!
//! Toutes les restitutions sont calculées par requête SQL (format long) puis
//! mises en forme en Rust, pour rester lisibles et faciles à maintenir.

use crate::dimensions;
use crate::money::Money;
use crate::validate::{validate_consolidated, validate_functional, CheckResult};
use duckdb::Connection;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// Ordre d'affichage des flux, lu depuis `dim_flow` (catalogue F00–F99).
///
/// Retombe sur une liste codée dur si `dim_flow` est vide ou introuvable
/// (ex. base non encore seedée). Tri lexicographique des codes : F00 < F01 <
/// F20 < F80 < F81 < F98 < F99, conforme à l'ordre logique du catalogue.
pub fn flow_order(con: &Connection) -> Vec<String> {
    con.query_row(
        "SELECT string_agg(code, ',' ORDER BY code) FROM dim_flow",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|s| s.split(',').map(String::from).collect())
    .unwrap_or_else(|| {
        vec![
            "F00".into(),
            "F01".into(),
            "F20".into(),
            "F80".into(),
            "F81".into(),
            "F98".into(),
            "F99".into(),
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers de mise en forme
// ─────────────────────────────────────────────────────────────────────────────

/// Formate un montant : 2 décimales, séparateur de milliers ; `-` si nul.
fn fmt_amount(x: Decimal) -> String {
    if x.is_zero() {
        return "-".to_string();
    }
    // Rust n'a pas de séparateur de milliers natif dans format!().
    // On formate à 2 décimales puis on insère les séparateurs manuellement.
    let formatted = format!("{:.2}", x);
    let (sign, int_part, dec_part) = if formatted.starts_with('-') {
        let rest = &formatted[1..];
        match rest.split_once('.') {
            Some((i, d)) => ("-", i, d),
            None => ("-", rest, ""),
        }
    } else {
        match formatted.split_once('.') {
            Some((i, d)) => ("", i, d),
            None => ("", &formatted[..], ""),
        }
    };
    // Insère un séparateur tous les 3 chiffres dans la partie entière.
    let chars: Vec<char> = int_part.chars().collect();
    let mut result = String::with_capacity(int_part.len() + int_part.len() / 3);
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*c);
    }
    format!("{sign}{result}.{dec_part}")
}

/// Charge une grille (account, flow) → montant pour un niveau donné.
fn load_grid(con: &Connection, level: &str) -> duckdb::Result<BTreeMap<(String, String), Decimal>> {
    // Totaux = lignes principales : exclut les « dont » (analytiques renseignées).
    let dims = dimensions::load_all(con)?;
    let of_which: String = dimensions::analytical_cols(&dims)
        .iter()
        .map(|c| format!(" AND {c} IS NULL"))
        .collect();
    let mut stmt = con.prepare(&format!(
        "SELECT account, flow, SUM(amount) AS amount
         FROM fact_entry
         WHERE level = ?{of_which}
         GROUP BY account, flow"
    ))?;
    let rows = stmt.query_map([level], |row| {
        let m: Money = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            m.into_decimal(),
        ))
    })?;
    let mut grid = BTreeMap::new();
    for r in rows {
        let (account, flow, amount) = r?;
        grid.insert((account, flow), amount);
    }
    Ok(grid)
}

// ─────────────────────────────────────────────────────────────────────────────
//  1. Bilan par flux (comptes × flux, F99 stocké + colonne F99 affichée)
// ─────────────────────────────────────────────────────────────────────────────

/// Affiche le bilan par flux : comptes en lignes × flux en colonnes.
///
/// La colonne F99 affichée est le F99 **stocké** en base (matérialisé par le
/// pipeline), conformément à l'identité de reconstruction.
pub fn bilan_par_flux(con: &Connection, level: &str) -> duckdb::Result<()> {
    let grid = load_grid(con, level)?;
    let accounts: std::collections::BTreeSet<String> =
        grid.keys().map(|(acc, _)| acc.clone()).collect();

    let devise = if level == "converted" || level == "consolidated" {
        "devise de présentation (EUR)"
    } else {
        "devise fonctionnelle"
    };

    println!("\n{}", "═".repeat(88));
    println!("  BILAN PAR FLUX  —  niveau « {} »  ({})", level, devise);
    println!("{}", "═".repeat(88));

    let col_w = 13;
    let flow_order = flow_order(con);
    print!("  {:<22}", "Compte");
    for fl in &flow_order {
        print!("{:>width$}", fl, width = col_w);
    }
    println!();
    println!("  {}", "─".repeat(22 + col_w * flow_order.len()));

    for acc in accounts {
        print!("  {:<22}", acc);
        for fl in &flow_order {
            let val = grid
                .get(&(acc.clone(), fl.to_string()))
                .copied()
                .unwrap_or(Decimal::ZERO);
            print!("{:>width$}", fmt_amount(val), width = col_w);
        }
        println!();
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  2. Comparaison des 4 niveaux pour un compte donné
// ─────────────────────────────────────────────────────────────────────────────

/// Affiche, pour un compte, le détail par flux aux 4 niveaux de stockage.
///
/// Met en évidence l'effet de chaque étape : agrégation → reclassification
/// (F00→F01 / collapse→F98) → conversion (écarts F80/F81, passage en EUR)
/// → consolidation (% d'intégration).
pub fn compare_levels(con: &Connection, account: &str) -> duckdb::Result<()> {
    let levels = ["corporate", "reclassified", "converted", "consolidated"];
    let level_desc = [
        ("corporate", "Agrégation (fonctionnel)"),
        ("reclassified", "Reclassification (fonctionnel)"),
        ("converted", "Conversion (EUR)"),
        ("consolidated", "Consolidation (EUR)"),
    ];

    println!("\n{}", "═".repeat(88));
    println!("  COMPARAISON DES 4 NIVEAUX  —  compte « {} »", account);
    println!("{}", "═".repeat(88));

    let col_w = 13;
    let flow_order = flow_order(con);
    print!("  {:<28}", "Niveau");
    for fl in &flow_order {
        print!("{:>width$}", fl, width = col_w);
    }
    println!();
    println!("  {}", "─".repeat(28 + col_w * flow_order.len()));

    let dims = dimensions::load_all(con)?;
    let of_which: String = dimensions::analytical_cols(&dims)
        .iter()
        .map(|c| format!(" AND {c} IS NULL"))
        .collect();
    for lvl in levels {
        let mut stmt = con.prepare(&format!(
            "SELECT flow, SUM(amount) AS amount
             FROM fact_entry
             WHERE level = ? AND account = ?{of_which}
             GROUP BY flow"
        ))?;
        let account_str = account.to_string();
        let rows = stmt.query_map([&lvl, account_str.as_str()], |row| {
            let m: Money = row.get(1)?;
            Ok((row.get::<_, String>(0)?, m.into_decimal()))
        })?;
        let mut grid: BTreeMap<String, Decimal> = BTreeMap::new();
        for r in rows {
            let (flow, amount) = r?;
            grid.insert(flow, amount);
        }

        let label = level_desc.iter().find(|(l, _)| *l == lvl).unwrap().1;
        print!("  {:<28}", label);
        for fl in &flow_order {
            let val = grid
                .get(fl)
                .copied()
                .unwrap_or(Decimal::ZERO);
            print!("{:>width$}", fmt_amount(val), width = col_w);
        }
        println!();
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  3. Résultat de validation (✓ / ✗ par compte)
// ─────────────────────────────────────────────────────────────────────────────

/// Affiche une ligne de résultat de validation.
fn print_check_row(r: &CheckResult) {
    let mark = if r.ok { "✓ OK" } else { "✗ ÉCHEC" };
    println!(
        "  {:<18}{:<8}{:>14}{:>18}{:>12}   {}",
        r.account,
        r.closure,
        fmt_amount(r.closure_stored),
        fmt_amount(r.somme),
        fmt_amount(r.ecart),
        mark
    );
}

/// Affiche le résultat des vérifications d'identité et renvoie le statut global.
pub fn print_validation(con: &Connection) -> duckdb::Result<bool> {
    println!("\n{}", "═".repeat(88));
    println!("  VALIDATION — Reconstruction des clôtures (via dim_flow.flux_de_report)");
    println!("{}", "═".repeat(88));

    // --- a) Côté consolidé (devise de présentation, écarts inclus) ---
    println!("\n  (a) Niveau CONSOLIDÉ (devise de présentation, écarts inclus)");
    println!(
        "  {:<18}{:<8}{:>14}{:>18}{:>12}   statut",
        "Compte", "Clôt.", "Clôture", "Σ composantes", "écart"
    );
    println!("  {}", "─".repeat(86));
    let mut all_ok = true;
    for r in validate_consolidated(con)? {
        if !r.ok {
            all_ok = false;
        }
        print_check_row(&r);
    }

    // --- b) Côté fonctionnel (reclassified, écarts = 0) ---
    println!("\n  (b) Niveau RECLASSIFIÉ (devise fonctionnelle, écarts = 0)");
    println!(
        "  {:<18}{:<8}{:>14}{:>18}{:>12}   statut",
        "Compte", "Clôt.", "Clôture", "Σ composantes", "écart"
    );
    println!("  {}", "─".repeat(86));
    for r in validate_functional(con)? {
        if !r.ok {
            all_ok = false;
        }
        print_check_row(&r);
    }

    let verdict = if all_ok {
        "✓ TOUTES LES IDENTITÉS TIENNENT"
    } else {
        "✗ IDENTITÉ(S) EN ÉCHEC"
    };
    println!("\n  Verdict global : {}", verdict);
    Ok(all_ok)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Bonus : synthèse des volumes par niveau
// ─────────────────────────────────────────────────────────────────────────────

/// Affiche le nombre de lignes stockées à chaque niveau.
pub fn print_level_counts(con: &Connection) -> duckdb::Result<()> {
    println!("\n{}", "─".repeat(88));
    println!("  Volumes par niveau de stockage");
    println!("{}", "─".repeat(88));

    let mut stmt = con.prepare(
        "SELECT level, COUNT(*) AS n
         FROM fact_entry
         GROUP BY level
         ORDER BY CASE level
             WHEN 'corporate' THEN 1
             WHEN 'reclassified' THEN 2
             WHEN 'converted' THEN 3
             WHEN 'consolidated' THEN 4
         END",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for r in rows {
        let (level, n) = r?;
        println!("    {:<14} {:>6} lignes", level, n);
    }
    Ok(())
}
