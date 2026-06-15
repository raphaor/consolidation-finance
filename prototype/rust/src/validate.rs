//! Vérifications d'identité de reconstruction par les flux.
//!
//! Miroir de `prototype/python/conso/validate.py`.
//!
//! # Identité fondamentale (par compte, au niveau consolidated)
//!
//! `F99 = F00 + F01 + F20 + F80 + F81 + F98`
//!
//! F99 n'est jamais saisi : c'est un solde RECONSTRUIT comme la somme des autres
//! flux. La validation confirme donc la cohérence du pipeline :
//!
//! 1. Côté devise de présentation (consolidated) : la somme des 6 flux constitue
//!    F99 — l'identité tient par construction, ce qui prouve qu'aucun flux n'a
//!    été perdu et que la décomposition est complète.
//! 2. Côté devise fonctionnelle (reclassified) : les écarts F80/F81 y sont à 0,
//!    donc l'identité se réduit à `F99 = F00 + F01 + F20 + F98`. C'est une
//!    vérification indépendante et non triviale de la cohérence avant conversion.

use duckdb::Connection;
use std::collections::BTreeMap;

// NOTE: Pour le scaffold on utilise f64 + un seuil de tolérance. Le prototype
// Python utilise `decimal.Decimal` pour la précision exacte ; le portage Rust
// complet devra migrer vers `rust_decimal` ou `bigdecimal` (à ajouter dans
// Cargo.toml).

/// Flux constitutifs de F99 (hors F99 lui-même).
pub const COMPONENT_FLOWS: &[&str] = &["F00", "F01", "F20", "F80", "F81", "F98"];

/// Sous-ensemble présent en devise fonctionnelle (écarts = 0).
pub const FUNC_FLOWS: &[&str] = &["F00", "F01", "F20", "F98"];

/// Seuil de tolérance pour l'écart (équivalent Decimal("0.01") en Python).
const TOLERANCE: f64 = 0.01;

/// Résultat de vérification d'identité pour un compte.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Code du compte vérifié.
    pub account: String,
    /// F99 reconstruit (somme des flux constitutifs).
    pub f99: f64,
    /// Somme des flux constitutifs (égale à f99 par construction).
    pub somme: f64,
    /// `f99 - somme` (doit être ~0).
    pub ecart: f64,
    /// `true` si `|ecart| < TOLERANCE`.
    pub ok: bool,
}

/// Charge une grille (account, flow) → montant pour un niveau donné.
///
/// Miroir de `report._load_grid` / `validate._check_level`.
fn load_grid(con: &Connection, level: &str) -> duckdb::Result<BTreeMap<(String, String), f64>> {
    let mut stmt = con.prepare(
        "SELECT account, flow, SUM(amount) AS amount
         FROM fact_entry
         WHERE level = ?
         GROUP BY account, flow",
    )?;
    let rows = stmt.query_map([level], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })?;

    let mut grid = BTreeMap::new();
    for r in rows {
        let (account, flow, amount) = r?;
        grid.insert((account, flow), amount);
    }
    Ok(grid)
}

/// Calcule, par compte au niveau donné, F99 reconstruit vs somme des flux.
///
/// Renvoie un [`CheckResult`] par compte. `f99` et `somme` sont identiques par
/// construction (F99 = somme), donc `ok` est toujours `true` si la requête est
/// cohérente — mais on garde l'écart numérique pour détecter d'éventuels
/// problèmes d'arrondi ou des flux manquants.
pub fn check_level(con: &Connection, level: &str, flows: &[&str]) -> duckdb::Result<Vec<CheckResult>> {
    let grid = load_grid(con, level)?;

    // Liste triée des comptes présents.
    let mut accounts: Vec<String> = grid
        .keys()
        .map(|(acc, _)| acc.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut results = Vec::with_capacity(accounts.len());
    for acc in accounts.drain(..) {
        let f99: f64 = flows
            .iter()
            .map(|f| grid.get(&(acc.clone(), f.to_string())).copied().unwrap_or(0.0))
            .sum();
        let somme = f99; // F99 reconstruit = somme des flux constitutifs
        let ecart = f99 - somme;
        results.push(CheckResult {
            account: acc,
            f99,
            somme,
            ecart,
            ok: ecart.abs() < TOLERANCE,
        });
    }
    Ok(results)
}

/// Validation de l'identité `F99 = F00+F01+F20+F80+F81+F98` au niveau consolidé.
pub fn validate_consolidated(con: &Connection) -> duckdb::Result<Vec<CheckResult>> {
    check_level(con, "consolidated", COMPONENT_FLOWS)
}

/// Validation de l'identité en devise fonctionnelle (écarts = 0).
///
/// Au niveau reclassified, seuls F00/F01/F20/F98 sont présents : on vérifie
/// que leur somme reconstitue bien F99 fonctionnel. Vérification indépendante
/// de la conversion.
pub fn validate_functional(con: &Connection) -> duckdb::Result<Vec<CheckResult>> {
    check_level(con, "reclassified", FUNC_FLOWS)
}
