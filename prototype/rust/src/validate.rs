//! Vérifications d'identité de reconstruction par les flux.
//!
//! Miroir de `prototype/python/conso/validate.py`.
//!
//! # Identité fondamentale (par compte, par niveau)
//!
//! `F99 = F00 + F01 + F20 + F80 + F81 + F98`
//!
//! F99 n'est jamais saisi : c'est un solde RECONSTRUIT par le pipeline (cf.
//! [`crate::pipeline::materialize_f99`]) comme la somme des autres flux, puis
//! stocké en base. La validation compare le F99 STOCKÉ à la somme INDEPENDANTE
//! des flux constitutifs lus à ce même niveau — ces deux quantités sont
//! produites par des requêtes SQL distinctes, donc toute incohérence (pipeline
//! cassé, écriture manuelle abusive sur F99, flux perdu) fait dériver l'écart.
//!
//! 1. Côté devise de présentation (consolidated) : les 6 flux constitutifs sont
//!    présents (F00/F01/F20/F80/F81/F98).
//! 2. Côté devise fonctionnelle (reclassified) : les écarts F80/F81 y sont à 0
//!    et ne sont jamais générés à ce niveau — on restreint donc la somme aux
//!    4 flux fonctionnels (F00/F01/F20/F98).

use crate::money::Money;
use duckdb::Connection;
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use std::collections::BTreeMap;

/// Flux constitutifs de F99 au niveau consolidated (écarts de conversion inclus).
pub const COMPONENT_FLOWS: &[&str] = &["F00", "F01", "F20", "F80", "F81", "F98"];

/// Flux constitutifs au niveau reclassified (devise fonctionnelle, écarts = 0).
pub const FUNC_FLOWS: &[&str] = &["F00", "F01", "F20", "F98"];

/// Seuil de tolérance pour l'écart : `Decimal("0.01")` (équivalent Python).
const TOLERANCE: Decimal = dec!(0.01);

/// Résultat de vérification d'identité pour un compte.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Code du compte vérifié.
    pub account: String,
    /// F99 **stocké** lu depuis la base.
    pub f99: Decimal,
    /// Σ des flux constitutifs (calculée indépendamment de `f99`).
    pub somme: Decimal,
    /// `f99 - somme` (doit être ~0).
    pub ecart: Decimal,
    /// `true` si `|ecart| < TOLERANCE`.
    pub ok: bool,
}

/// Charge une grille (account, flow) → montant pour un niveau donné.
///
/// Miroir de `report._load_grid` / `validate._check_level`.
fn load_grid(con: &Connection, level: &str) -> duckdb::Result<BTreeMap<(String, String), Decimal>> {
    let mut stmt = con.prepare(
        "SELECT account, flow, SUM(amount) AS amount
         FROM fact_entry
         WHERE level = ?
         GROUP BY account, flow",
    )?;
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

/// Calcule, par compte au niveau donné, l'écart entre F99 stocké et la somme
/// des flux constitutifs.
///
/// - `f99` provient du flux `'F99'` lu en base (materialisé par le pipeline).
/// - `somme` est la somme des flux passés en paramètre (`COMPONENT_FLOWS` au
///   niveau consolidated, `FUNC_FLOWS` au niveau reclassified).
///
/// `ecart = f99 - somme` doit valoir 0 à la tolérance près. Si le pipeline
/// perd un flux, génère un doublon, ou si quelqu'un écrit manuellement sur F99,
/// l'identité casse et `ok = false`.
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
        // F99 STOCKÉ en base (matérialisé par le pipeline). 0 si absent.
        let f99 = grid
            .get(&(acc.clone(), "F99".to_string()))
            .copied()
            .unwrap_or(Decimal::ZERO);
        // Σ des flux constitutifs — calcul indépendant de F99.
        let mut somme = Decimal::ZERO;
        for f in flows {
            somme += grid
                .get(&(acc.clone(), f.to_string()))
                .copied()
                .unwrap_or(Decimal::ZERO);
        }
        let ecart = f99 - somme;
        let ok = ecart.abs() < TOLERANCE;
        results.push(CheckResult {
            account: acc,
            f99,
            somme,
            ecart,
            ok,
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
/// Au niveau reclassified, seuls F00/F01/F20/F98 sont présents (pas d'écarts
/// F80/F81 en devise fonctionnelle) : on vérifie que leur somme reconstitue
/// bien le F99 fonctionnel stocké à ce niveau.
pub fn validate_functional(con: &Connection) -> duckdb::Result<Vec<CheckResult>> {
    check_level(con, "reclassified", FUNC_FLOWS)
}
