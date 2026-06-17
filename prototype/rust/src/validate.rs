//! Vérifications d'identité de reconstruction des flux de clôture, et
//! validation des données de saisie (FK nature).
//!
//! Miroir de `prototype/python/conso/validate.py`.
//!
//! # Identité de reconstruction (par compte, par niveau)
//!
//! Pour chaque clôture C — flux auto-référentiel : `flux_de_report(C) = C` :
//!
//! ```text
//! C = Σ( flux X | flux_de_report(X) = C et X ≠ C )
//! ```
//!
//! Une clôture n'est jamais saisie : c'est un solde RECONSTRUIT par le pipeline
//! (cf. [`crate::pipeline::materialize_closures`]) comme la somme des flux qui y
//! reportent, puis stocké en base. La validation compare la clôture STOCKÉE à la
//! somme INDEPENDANTE de ses constituants lus au même niveau — ces deux quantités
//! sont produites par des requêtes SQL distinctes, donc toute incohérence
//! (pipeline cassé, écriture manuelle abusive sur une clôture, flux perdu) fait
//! dériver l'écart.
//!
//! **Data-driven** : ni les clôtures ni leurs constituants ne sont en dur. La
//! carte {clôture → constituants} est lue dans `dim_flow` au début de chaque
//! vérification. Ajouter un flux dans `dim_flow` l'intègre automatiquement.
//!
//! Au niveau `reclassified` (devise fonctionnelle), les écarts F80/F81 sont
//! absents (ils n'existent qu'après conversion) et valent donc 0 dans la somme :
//! la même identité tient aux deux niveaux sans configuration spéciale.

use crate::money::Money;
use duckdb::Connection;
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Seuil de tolérance pour l'écart : `Decimal("0.01")` (équivalent Python).
const TOLERANCE: Decimal = dec!(0.01);

/// Résultat de vérification d'identité pour un compte et une clôture.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Code du compte vérifié.
    pub account: String,
    /// Code du flux de clôture vérifié (ex. `F99`).
    pub closure: String,
    /// Clôture **stockée** lue depuis la base.
    pub closure_stored: Decimal,
    /// Σ des flux constitutifs (calculée indépendamment de la clôture stockée).
    pub somme: Decimal,
    /// `closure_stored - somme` (doit être ~0).
    pub ecart: Decimal,
    /// `true` si `|ecart| < TOLERANCE`.
    pub ok: bool,
}

/// Carte ordonnée `{ clôture → [constituants] }` lue depuis `dim_flow`.
///
/// - Clôtures : flux auto-référentiels (`code = flux_de_report`).
/// - Constituants d'une clôture C : flux X tels que `flux_de_report(X) = C` et
///   `X ≠ C`. L'ordre est stable (trié par code) pour des rendus reproductibles.
fn load_closure_components(con: &Connection) -> duckdb::Result<Vec<(String, Vec<String>)>> {
    let closures: Vec<String> = {
        let mut stmt = con.prepare(
            "SELECT code FROM dim_flow WHERE code = flux_de_report ORDER BY code",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut v = Vec::new();
        for r in rows {
            v.push(r?);
        }
        v
    };

    let mut out = Vec::with_capacity(closures.len());
    for c in &closures {
        let comps: Vec<String> = {
            let mut stmt = con.prepare(
                "SELECT code FROM dim_flow \
                 WHERE flux_de_report = ? AND code <> ? ORDER BY code",
            )?;
            let rows = stmt.query_map([c.clone(), c.clone()], |row| row.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        out.push((c.clone(), comps));
    }
    Ok(out)
}

/// Charge une grille (account, flow) → montant pour un niveau donné.
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

/// Pour chaque (compte × clôture), compare la clôture stockée à la Σ de ses
/// constituants au niveau donné.
///
/// Renvoie un `CheckResult` par couple (compte, clôture) où la clôture ou l'un de
/// ses constituants est présent. Les couples totalement absents (0 / 0) sont
/// ignorés pour ne pas bruiter le rendu.
///
/// `ecart = closure_stored - somme` doit valoir 0 à la tolérance près. Si le
/// pipeline perd un flux, génère un doublon, ou si quelqu'un écrit manuellement
/// sur une clôture sans passer par la reconstruction, l'identité casse → `ok = false`.
pub fn check_closures(con: &Connection, level: &str) -> duckdb::Result<Vec<CheckResult>> {
    let grid = load_grid(con, level)?;
    let closure_map = load_closure_components(con)?;

    // Comptes présents à ce niveau (triés).
    let accounts: Vec<String> = grid
        .keys()
        .map(|(acc, _)| acc.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut results = Vec::new();
    for acc in accounts {
        for (closure, comps) in &closure_map {
            // Clôture STOCKÉE en base (matérialisée par le pipeline). 0 si absente.
            let closure_stored = grid
                .get(&(acc.clone(), closure.clone()))
                .copied()
                .unwrap_or(Decimal::ZERO);
            // Σ des constituants — calcul indépendant de la clôture stockée.
            let mut somme = Decimal::ZERO;
            for cf in comps {
                somme += grid
                    .get(&(acc.clone(), cf.clone()))
                    .copied()
                    .unwrap_or(Decimal::ZERO);
            }
            // Skip les couples vides (ni clôture ni constituant sur ce compte).
            if closure_stored.is_zero() && somme.is_zero() {
                continue;
            }
            let ecart = closure_stored - somme;
            let ok = ecart.abs() < TOLERANCE;
            results.push(CheckResult {
                account: acc.clone(),
                closure: closure.clone(),
                closure_stored,
                somme,
                ecart,
                ok,
            });
        }
    }
    Ok(results)
}

/// Validation des identités de clôture au niveau consolidé (devise de
/// présentation, écarts inclus).
pub fn validate_consolidated(con: &Connection) -> duckdb::Result<Vec<CheckResult>> {
    check_closures(con, "consolidated")
}

/// Validation des identités de clôture au niveau reclassified (devise
/// fonctionnelle, écarts absents).
pub fn validate_functional(con: &Connection) -> duckdb::Result<Vec<CheckResult>> {
    check_closures(con, "reclassified")
}

// ─────────────────────────────────────────────────────────────────────────────
//  Validation de la saisie : colonne `nature` obligatoire et FK sur dim_nature.
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne d'anomalie de validation de la nature d'une écriture de `stg_entry`.
#[derive(Debug, Clone)]
pub struct NatureCheck {
    /// Code anomalie : `missing` (nature NULL/vide) ou `unknown` (absente de `dim_nature`).
    pub kind: &'static str,
    /// Nombre d'écritures concernées par cette anomalie.
    pub count: i64,
    /// Valeur observée (NULL vide pour `missing`, code nature inconnu sinon).
    pub nature: Option<String>,
}

/// Vérifie que `nature` est **obligatoire** (non-null, non-vide) sur `stg_entry`
/// et que chaque valeur pointe vers une ligne existante de `dim_nature` (FK).
///
/// Renvoie une ligne par anomalie :
///   - `missing` : écritures dont la nature est NULL ou vide.
///   - `unknown` : écritures dont la nature ne correspond à aucun code `dim_nature`.
///
/// Une liste vide signifie que toutes les écritures sont conformes.
pub fn check_natures(con: &Connection) -> duckdb::Result<Vec<NatureCheck>> {
    let mut stmt = con.prepare(
        "WITH diag AS (
            SELECT
                CASE
                    WHEN nature IS NULL OR nature = '' THEN '__MISSING__'
                    ELSE nature
                END AS nature_key,
                CASE
                    WHEN nature IS NULL OR nature = '' THEN 1
                    WHEN nature NOT IN (SELECT code FROM dim_nature) THEN 1
                    ELSE 0
                END AS bad
            FROM stg_entry
        )
        SELECT nature_key, COUNT(*) AS n
        FROM diag
        WHERE bad = 1
        GROUP BY nature_key
        ORDER BY nature_key",
    )?;
    let rows = stmt.query_map([], |row| {
        let key: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        if key == "__MISSING__" {
            Ok(NatureCheck { kind: "missing", count, nature: None })
        } else {
            Ok(NatureCheck { kind: "unknown", count, nature: Some(key) })
        }
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}
