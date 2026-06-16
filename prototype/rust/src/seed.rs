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

use crate::money::Money;
use duckdb::{Connection, params};
use rust_decimal::Decimal;
use rust_decimal::prelude::*;

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
///
/// `400_Resultat` est rangé en `bilan` : c'est le « résultat de l'exercice »,
/// solde net reporté au passif, pas un compte de P&L. Les comptes 6xx/7xx sont
/// les véritables comptes de produits et charges (classe `resultat`).
const ACCOUNTS: &[(&str, &str, &str, bool, Option<&str>)] = &[
    ("100_Capital",         "Capital",                "equity",   true,  None),
    ("200_Immobilisations", "Immobilisations",        "bilan",    false, None),
    ("300_Stocks",          "Stocks",                 "bilan",    false, None),
    ("400_Resultat",        "Résultat de l'exercice", "bilan",    false, None),
    ("600_Achats",          "Achats",                 "resultat", false, None),
    ("610_Charges",         "Autres charges",         "resultat", false, None),
    ("640_Dotations",       "Dotations aux amort.",   "resultat", false, None),
    ("700_Produits",        "Ventes",                 "resultat", false, None),
    ("705_Prestations",     "Prestations de services","resultat", false, None),
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
const PERIMETER: &[((&str, &str, &str, &str), (Decimal, Decimal, bool, bool))] = &[
    (("M", "REEL", "2024", "globale"), (dec!(1.00), dec!(1.00), false, false)),
    (("A", "REEL", "2024", "globale"), (dec!(1.00), dec!(1.00), true,  false)), // ENTRE en N
    (("B", "REEL", "2024", "globale"), (dec!(1.00), dec!(1.00), false, true)),  // SORT en N
];

/// Taux de change vers EUR.
///   - period '2023' : taux clôture N-1 (utilisé par close_n1)
///   - period '2024' : taux clôture N (close_n / terminal) et taux moyen (avg)
/// (currency_source, period, taux_close, taux_moyen).
const RATES: &[((&str, &str), (Option<Decimal>, Option<Decimal>))] = &[
    (("USD", "2023"), (Some(dec!(0.92)), None)),
    (("USD", "2024"), (Some(dec!(0.90)), Some(dec!(0.95)))), // close_n = 0.90, avg = 0.95
    (("GBP", "2023"), (Some(dec!(1.15)), None)),
    (("GBP", "2024"), (Some(dec!(1.12)), Some(dec!(1.18)))), // close_n = 1.12, avg = 1.18
];

// ─────────────────────────────────────────────────────────────────────────────
//  Écritures brutes (saisie source) — flux sociaux F00 et F20 uniquement
// ─────────────────────────────────────────────────────────────────────────────
//  Note : pour démontrer l'agrégation (étape A), le F20 du résultat de M
//         est volontairement éclaté en deux lignes (500 + 300 = 800).
//  Note : les comptes de P&L (6xx/7xx) n'ont que du F20 (flux de période,
//         pas d'ouverture). Les comptes de bilan ont F00 (ouverture N) et
//         du F20 (mouvements de l'exercice).
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne de saisie brute :
/// (scenario, entity, entry_period, period, account, flow, currency,
///  partner, share, analysis, audit_id, amount).
type RawRow = (&'static str, &'static str, &'static str, &'static str,
               &'static str, &'static str, &'static str,
               Option<&'static str>, Option<&'static str>, Option<&'static str>,
               &'static str, Decimal);

const RAW: &[RawRow] = &[
    // ── Mère M (EUR) — périmètre continu ──
    ("REEL", "M", "2024", "2024", "100_Capital",         "F00", "EUR", None, None, None, "S-M-001", dec!(10000)),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F00", "EUR", None, None, None, "S-M-002", dec!(5000)),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-003", dec!(500)),
    ("REEL", "M", "2024", "2024", "400_Resultat",        "F20", "EUR", None, None, None, "S-M-004", dec!(300)),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F00", "EUR", None, None, None, "S-M-005", dec!(12000)),
    ("REEL", "M", "2024", "2024", "200_Immobilisations", "F20", "EUR", None, None, None, "S-M-006", dec!(500)),
    ("REEL", "M", "2024", "2024", "300_Stocks",          "F00", "EUR", None, None, None, "S-M-007", dec!(3000)),
    // Comptes de P&L (classe « resultat ») — F20 uniquement
    ("REEL", "M", "2024", "2024", "700_Produits",        "F20", "EUR", None, None, None, "S-M-010", dec!(2000)),
    ("REEL", "M", "2024", "2024", "705_Prestations",     "F20", "EUR", None, None, None, "S-M-011", dec!(1000)),
    ("REEL", "M", "2024", "2024", "600_Achats",          "F20", "EUR", None, None, None, "S-M-012", dec!(800)),
    ("REEL", "M", "2024", "2024", "610_Charges",         "F20", "EUR", None, None, None, "S-M-013", dec!(500)),
    ("REEL", "M", "2024", "2024", "640_Dotations",       "F20", "EUR", None, None, None, "S-M-014", dec!(200)),

    // ── Filiale A (USD) — ENTRE en N ──
    ("REEL", "A", "2024", "2024", "100_Capital",         "F00", "USD", None, None, None, "S-A-001", dec!(5000)),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F00", "USD", None, None, None, "S-A-002", dec!(2000)),
    ("REEL", "A", "2024", "2024", "400_Resultat",        "F20", "USD", None, None, None, "S-A-003", dec!(300)),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F00", "USD", None, None, None, "S-A-004", dec!(8000)),
    ("REEL", "A", "2024", "2024", "200_Immobilisations", "F20", "USD", None, None, None, "S-A-005", dec!(400)),
    // Comptes de P&L — F20 uniquement
    ("REEL", "A", "2024", "2024", "700_Produits",        "F20", "USD", None, None, None, "S-A-010", dec!(1000)),
    ("REEL", "A", "2024", "2024", "705_Prestations",     "F20", "USD", None, None, None, "S-A-011", dec!(500)),
    ("REEL", "A", "2024", "2024", "600_Achats",          "F20", "USD", None, None, None, "S-A-012", dec!(400)),
    ("REEL", "A", "2024", "2024", "610_Charges",         "F20", "USD", None, None, None, "S-A-013", dec!(200)),
    ("REEL", "A", "2024", "2024", "640_Dotations",       "F20", "USD", None, None, None, "S-A-014", dec!(100)),

    // ── Filiale B (GBP) — SORT en N ──
    ("REEL", "B", "2024", "2024", "100_Capital",         "F00", "GBP", None, None, None, "S-B-001", dec!(4000)),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F00", "GBP", None, None, None, "S-B-002", dec!(1500)),
    ("REEL", "B", "2024", "2024", "400_Resultat",        "F20", "GBP", None, None, None, "S-B-003", dec!(200)),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F00", "GBP", None, None, None, "S-B-004", dec!(6000)),
    ("REEL", "B", "2024", "2024", "200_Immobilisations", "F20", "GBP", None, None, None, "S-B-005", dec!(300)),
    // Comptes de P&L — F20 uniquement
    ("REEL", "B", "2024", "2024", "700_Produits",        "F20", "GBP", None, None, None, "S-B-010", dec!(800)),
    ("REEL", "B", "2024", "2024", "705_Prestations",     "F20", "GBP", None, None, None, "S-B-011", dec!(400)),
    ("REEL", "B", "2024", "2024", "600_Achats",          "F20", "GBP", None, None, None, "S-B-012", dec!(300)),
    ("REEL", "B", "2024", "2024", "610_Charges",         "F20", "GBP", None, None, None, "S-B-013", dec!(200)),
    ("REEL", "B", "2024", "2024", "640_Dotations",       "F20", "GBP", None, None, None, "S-B-014", dec!(100)),
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
    for (k, v) in PERIMETER {
        con.execute(
            "INSERT INTO sat_perimeter VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![k.0, k.1, k.2, k.3, Money(v.0), Money(v.1), v.2, v.3],
        )?;
    }
    for (k, v) in RATES {
        con.execute(
            "INSERT INTO sat_exchange_rate (currency_source, period, taux_close, taux_moyen) \
             VALUES (?, ?, ?, ?)",
            params![
                k.0,
                k.1,
                v.0.map(Money),
                v.1.map(Money),
            ],
        )?;
    }

    // --- Staging (saisie brute) ---
    for row in RAW {
        con.execute(
            "INSERT INTO stg_entry VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                row.0, row.1, row.2, row.3, row.4, row.5, row.6,
                row.7, row.8, row.9, row.10, Money(row.11),
            ],
        )?;
    }

    Ok(())
}
