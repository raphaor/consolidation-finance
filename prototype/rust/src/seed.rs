//! Données de test — groupe multi-devise avec entrée et sortie de périmètre.
//!
//! Miroir de `prototype/python/conso/seed.py`.
//!
//! # Groupe de test
//!
//! | Entité    | Devise | Méthode   | % intégr. | Périmètre  |
//! |-----------|--------|-----------|-----------|------------|
//! | Mère M    | EUR    | globale   | 100 %     | continu    |
//! | Filiale A | USD    | globale   | 100 %     | ENTRE en N |
//! | Filiale B | GBP    | globale   | 100 %     | SORT en N  |
//!
//! Période traitée : `Entry_period = Period = "2024"` ; exercice précédent `"2023"`.
//! Devise de présentation : EUR.

use duckdb::{Connection, params};

// ─────────────────────────────────────────────────────────────────────────────
//  Master data
// ─────────────────────────────────────────────────────────────────────────────

/// Scénarios de test : (code, libelle, type, statut).
const SCENARIOS: &[(&str, &str, &str, &str)] = &[("REEL", "Réel 2024", "réel", "ouvert")];

/// Entités du groupe : (code, libelle, devise_fonctionnelle, entite_parent, statut).
const ENTITIES: &[(&str, &str, &str, Option<&str>, &str)] = &[
    ("M", "Mère",      "EUR", None,  "actif"),
    ("A", "Filiale A", "USD", Some("M"), "actif"),
    ("B", "Filiale B", "GBP", Some("M"), "actif"),
];

/// Périodes : (code, libelle, type, date_debut, date_fin, statut).
const PERIODS: &[(&str, &str, &str, &str, &str, &str)] = &[
    ("2023", "Exercice 2023", "exercice", "2023-01-01", "2023-12-31", "clôturé"),
    ("2024", "Exercice 2024", "exercice", "2024-01-01", "2024-12-31", "ouvert"),
];

/// Plan de compte : (code, libelle, classe, capitaux_propres, compte_parent).
const ACCOUNTS: &[(&str, &str, &str, bool, Option<&str>)] = &[
    ("100_Capital",         "Capital",         "equity",   true,  None),
    ("200_Immobilisations", "Immobilisations", "bilan",    false, None),
    ("300_Stocks",          "Stocks",          "bilan",    false, None),
    ("400_Resultat",        "Résultat",        "resultat", false, None),
];

/// Catalogue des flux — cf. docs/FLUX_CONSO.md §6.
/// (code, libelle, taux_conversion, flux_ecart).
const FLOWS: &[(&str, &str, &str, Option<&str>)] = &[
    ("F00", "Ouverture",               "close_n1", Some("F80")),
    ("F01", "Entrée périmètre",        "close_n1", Some("F80")),
    ("F20", "Variation",               "avg",      Some("F81")),
    ("F80", "Écart conv. ouverture",   "terminal", None),
    ("F81", "Écart conv. variation",   "terminal", None),
    ("F98", "Sortie périmètre",        "terminal", None),
    ("F99", "Clôture",                 "close_n",  None),
];

/// Devises référentielles : (code_iso, libelle, decimales).
const CURRENCIES: &[(&str, &str, i32)] = &[
    ("EUR", "Euro",           2),
    ("USD", "Dollar US",      2),
    ("GBP", "Livre sterling", 2),
];

// ─────────────────────────────────────────────────────────────────────────────
//  Tables satellites
// ─────────────────────────────────────────────────────────────────────────────

/// Périmètre de consolidation.
/// (entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie).
const PERIMETER: &[(&str, &str, &str, &str, f64, f64, bool, bool)] = &[
    ("M", "REEL", "2024", "globale", 1.00, 1.00, false, false),
    ("A", "REEL", "2024", "globale", 1.00, 1.00, true,  false), // ENTRE en N
    ("B", "REEL", "2024", "globale", 1.00, 1.00, false, true),  // SORT en N
];

/// Taux de change vers EUR.
///   - period '2023' : taux clôture N-1 (utilisé par close_n1)
///   - period '2024' : taux clôture N (close_n / terminal) et taux moyen (avg)
/// (currency_source, period, taux_close, taux_moyen).
const RATES: &[(&str, &str, Option<f64>, Option<f64>)] = &[
    ("USD", "2023", Some(0.92), None),
    ("USD", "2024", Some(0.90), Some(0.95)), // close_n = 0.90, avg = 0.95
    ("GBP", "2023", Some(1.15), None),
    ("GBP", "2024", Some(1.12), Some(1.18)), // close_n = 1.12, avg = 1.18
];

// ─────────────────────────────────────────────────────────────────────────────
//  Écritures brutes (saisie source) — flux sociaux F00 et F20 uniquement
// ─────────────────────────────────────────────────────────────────────────────
//  Note : pour démontrer l'agrégation (étape A), le F20 du résultat de M
//         est volontairement éclaté en deux lignes (500 + 300 = 800).
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne de saisie brute :
/// (scenario, entity, entry_period, period, account, flow, currency,
///  partner, share, analysis, audit_id, amount).
type RawRow = (&'static str, &'static str, &'static str, &'static str,
               &'static str, &'static str, &'static str,
               Option<&'static str>, Option<&'static str>, Option<&'static str>,
               &'static str, f64);

const RAW: &[RawRow] = &[
    // ── Mère M (EUR) — périmètre continu ──
    ("REEL", "M", "2024", "2024", "100_Capital",         "F00", "EUR", None, None, None, "S-M-001", 10000.0),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F00", "EUR", None, None, None, "S-M-002",  5000.0),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-003",   500.0),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-004",   300.0),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F00", "EUR", None, None, None, "S-M-005", 12000.0),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F20", "EUR", None, None, None, "S-M-006",   500.0),
    ("REEL", "M", "2024", "2024", "300_Stocks",          "F00", "EUR", None, None, None, "S-M-007",  3000.0),

    // ── Filiale A (USD) — ENTRE en N ──
    ("REEL", "A", "2024", "2024", "100_Capital",         "F00", "USD", None, None, None, "S-A-001",  5000.0),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F00", "USD", None, None, None, "S-A-002",  2000.0),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F20", "USD", None, None, None, "S-A-003",   300.0),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F00", "USD", None, None, None, "S-A-004",  8000.0),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F20", "USD", None, None, None, "S-A-005",   400.0),

    // ── Filiale B (GBP) — SORT en N ──
    ("REEL", "B", "2024", "2024", "100_Capital",         "F00", "GBP", None, None, None, "S-B-001",  4000.0),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F00", "GBP", None, None, None, "S-B-002",  1500.0),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F20", "GBP", None, None, None, "S-B-003",   200.0),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F00", "GBP", None, None, None, "S-B-004",  6000.0),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F20", "GBP", None, None, None, "S-B-005",   300.0),
];

/// Insère toutes les données de test : master data, satellites et saisie brute.
///
/// Miroir de `conso/seed.py::seed_all`.
pub fn seed_all(con: &Connection) -> duckdb::Result<()> {
    // --- Dimensions ---
    for s in SCENARIOS {
        con.execute(
            "INSERT INTO dim_scenario VALUES (?, ?, ?, ?)",
            params![s.0, s.1, s.2, s.3],
        )?;
    }
    for e in ENTITIES {
        con.execute(
            "INSERT INTO dim_entity VALUES (?, ?, ?, ?, ?)",
            params![e.0, e.1, e.2, e.3, e.4],
        )?;
    }
    for p in PERIODS {
        con.execute(
            "INSERT INTO dim_period VALUES (?, ?, ?, ?, ?, ?)",
            params![p.0, p.1, p.2, p.3, p.4, p.5],
        )?;
    }
    for a in ACCOUNTS {
        con.execute(
            "INSERT INTO dim_account VALUES (?, ?, ?, ?, ?)",
            params![a.0, a.1, a.2, a.3, a.4],
        )?;
    }
    for f in FLOWS {
        con.execute(
            "INSERT INTO dim_flow VALUES (?, ?, ?, ?)",
            params![f.0, f.1, f.2, f.3],
        )?;
    }
    for c in CURRENCIES {
        con.execute(
            "INSERT INTO dim_currency VALUES (?, ?, ?)",
            params![c.0, c.1, c.2],
        )?;
    }

    // --- Tables satellites ---
    for p in PERIMETER {
        con.execute(
            "INSERT INTO sat_perimeter VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![p.0, p.1, p.2, p.3, p.4, p.5, p.6, p.7],
        )?;
    }
    for r in RATES {
        con.execute(
            "INSERT INTO sat_exchange_rate (currency_source, period, taux_close, taux_moyen) \
             VALUES (?, ?, ?, ?)",
            params![r.0, r.1, r.2, r.3],
        )?;
    }

    // --- Staging (saisie brute) ---
    for row in RAW {
        con.execute(
            "INSERT INTO stg_entry VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![row.0, row.1, row.2, row.3, row.4, row.5, row.6,
                    row.7, row.8, row.9, row.10, row.11],
        )?;
    }

    Ok(())
}
