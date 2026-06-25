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
use duckdb::{params, Connection};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;

// ─────────────────────────────────────────────────────────────────────────────
//  Master data
// ─────────────────────────────────────────────────────────────────────────────

/// Config applicative : (key, value). Une seule entrée — la devise pivot.
/// Ici `pivot = EUR` : les taux de `sat_exchange_rate` convertissent tout vers EUR.
const APP_CONFIG: &[(&str, &str)] = &[("pivot_currency", "EUR")];

/// Catégories de scénario : (code, libelle).
const SCENARIO_CATEGORIES: &[(&str, &str)] = &[("REEL", "Réel")];

/// Variantes : (code, libelle).
const VARIANTS: &[(&str, &str)] = &[("BASE", "Base")];

/// Jeux de taux : (code, libelle).
const RATE_SETS: &[(&str, &str)] = &[("RATES", "Taux réels")];

/// Jeux de périmètre : (code, libelle). Symétrique de `RATE_SETS` (Q35).
const PERIMETER_SETS: &[(&str, &str)] = &[("PERIM_REEL", "Périmètre réel 2024")];

/// Consolidations de test : (libelle, phase, exercice, perimeter_set, variant,
/// presentation_currency, perimeter_period, rate_set, rate_period, ruleset_code,
/// statut). `a_nouveau_consolidation_id` est NULL pour le seed.
///
/// L'id est alloué par `nextval('seq_consolidation')` dans l'INSERT : sur une
/// base fraîchement créée, la 1ʳᵉ consolidation reçoit donc l'id déterministe 1
/// (et avance la séquence, de sorte que les éventuels INSERT ultérieurs en
/// DEFAULT nextval — tests créant une consolidation CUR/REEL_N1 — ne collent pas
/// cet id).
///
/// La consolidation `REEL` agrège toutes les références nécessaires à un run.
/// Avec `pivot = EUR` et `presentation_currency = EUR`, la conversion se
/// comporte comme avant (cross-rate = taux direct). `ruleset_code = NULL` : pas
/// de règles appliquées sur la consolidation de base.
const CONSOLIDATIONS: &[(
    &str,
    &str,
    &str,
    &str,
    &str,
    &str,
    &str,
    &str,
    &str,
    Option<&str>,
    &str,
)] = &[(
    "Réel 2024",
    "REEL",
    "2024",
    "PERIM_REEL",
    "BASE",
    "EUR",
    "2024",
    "RATES",
    "2024",
    None,
    "ouvert",
)];

/// Entités du groupe : (code, libelle, devise_fonctionnelle, entite_parent, statut).
const ENTITIES: &[(&str, &str, &str, Option<&str>, &str)] = &[
    ("M", "Mère", "EUR", None, "actif"),
    ("A", "Filiale A", "USD", Some("M"), "actif"),
    ("B", "Filiale B", "GBP", Some("M"), "actif"),
];

/// Périodes : (code, libelle, type, date_debut, date_fin, statut).
const PERIODS: &[(&str, &str, &str, &str, &str, &str)] = &[
    (
        "2023",
        "Exercice 2023",
        "exercice",
        "2023-01-01",
        "2023-12-31",
        "clôturé",
    ),
    (
        "2024",
        "Exercice 2024",
        "exercice",
        "2024-01-01",
        "2024-12-31",
        "ouvert",
    ),
];

/// Sous-classes de comptes : (code, libelle, classe).
const SOUS_CLASSES: &[(&str, &str, &str)] = &[
    ("actif", "Actif", "bilan"),
    ("passif", "Passif", "bilan"),
    ("charges", "Charges", "resultat"),
    ("produits", "Produits", "resultat"),
];

/// Plan de compte : (code, libelle, classe, sous_classe).
///
/// `400` est rangé en `bilan` : c'est le « résultat de l'exercice », solde net
/// reporté au passif, pas un compte de P&L. Les comptes 6xx/7xx sont les
/// véritables comptes de produits et charges (classe `resultat`). Le regroupement
/// par nature (ex. `capitaux_propres` sur `100`) et la hiérarchie de compte
/// parent ne sont plus des colonnes en dur : ils se posent via [`seed_demo_attributes`].
const ACCOUNTS: &[(&str, &str, &str, &str)] = &[
    ("100", "Capital", "bilan", "passif"),
    ("200", "Immobilisations", "bilan", "actif"),
    ("300", "Stocks", "bilan", "actif"),
    ("400", "Résultat de l'exercice", "bilan", "passif"),
    ("600", "Achats", "resultat", "charges"),
    ("610", "Autres charges", "resultat", "charges"),
    ("640", "Dotations aux amort.", "resultat", "charges"),
    ("700", "Ventes", "resultat", "produits"),
    ("705", "Prestations", "resultat", "produits"),
    // Comptes de tiers (PCG) pour la modélisation interco bilan.
    (
        "467",
        "Divers comptes débiteurs et produits à recevoir",
        "bilan",
        "actif",
    ),
    (
        "468",
        "Divers comptes créditeurs et charges à payer",
        "bilan",
        "passif",
    ),
    ("471L", "Liaison élimination intragroupe", "bilan", "passif"),
];

/// Catalogue des flux — **dimension nue** (code, libelle). Tout le comportement
/// (taux, écart, report de clôture, à-nouveau) vit dans les schémas de flux
/// ci-dessous. Cf. docs/FLUX_CONSO.md §6.
const FLOWS: &[(&str, &str)] = &[
    ("F00", "Ouverture"),
    ("F01", "Entrée périmètre"),
    ("F20", "Variation"),
    ("F80", "Écart conv. ouverture"),
    ("F81", "Écart conv. variation"),
    ("F98", "Sortie périmètre"),
    ("F99", "Clôture"),
];

/// Schémas de flux : (code, libelle). Cf. docs/QUESTIONS_OUVERTES.md Q32.
const FLOW_SCHEMES: &[(&str, &str)] = &[
    (
        "BILAN",
        "Schéma bilan (taux du flux, écarts F80/F81, report F99→F00)",
    ),
    (
        "RESULTAT",
        "Schéma résultat (taux moyen, sans écart, sans à-nouveau)",
    ),
];

/// Articulation **complète** des flux par schéma :
/// (scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau).
///
/// `flux_de_report` : flux de clôture où ce flux se reporte (auto-référence
/// F99 → F99 = clôture reconstruite). `flux_a_nouveau` est NULL par défaut
/// (à-nouveau désactivé ; activé sur le F99 du schéma voulu). `BILAN` reproduit
/// l'ex-`dim_flow` ; `RESULTAT` met tout au taux moyen sans écart, et **n'a pas
/// d'à-nouveau** (le résultat ne reporte pas son solde en ouverture N+1).
const FLOW_SCHEME_ITEMS: &[(&str, &str, &str, Option<&str>, Option<&str>, Option<&str>)] = &[
    // BILAN — articulation par défaut (ex-dim_flow).
    ("BILAN", "F00", "close_n1", Some("F80"), Some("F99"), None),
    ("BILAN", "F01", "close_n1", Some("F80"), Some("F99"), None),
    ("BILAN", "F20", "avg", Some("F81"), Some("F99"), None),
    ("BILAN", "F80", "close_n", None, Some("F99"), None),
    ("BILAN", "F81", "close_n", None, Some("F99"), None),
    ("BILAN", "F98", "close_n", None, Some("F99"), None),
    ("BILAN", "F99", "close_n", None, Some("F99"), None),
    // RESULTAT — taux moyen, sans écart, sans à-nouveau.
    ("RESULTAT", "F00", "avg", None, Some("F99"), None),
    ("RESULTAT", "F01", "avg", None, Some("F99"), None),
    ("RESULTAT", "F20", "avg", None, Some("F99"), None),
    ("RESULTAT", "F80", "close_n", None, Some("F99"), None),
    ("RESULTAT", "F81", "close_n", None, Some("F99"), None),
    ("RESULTAT", "F98", "avg", None, Some("F99"), None),
    ("RESULTAT", "F99", "avg", None, Some("F99"), None),
];

/// Devises référentielles : (code_iso, libelle, decimales).
const CURRENCIES: &[(&str, &str, i32)] = &[
    ("EUR", "Euro", 2),
    ("USD", "Dollar US", 2),
    ("GBP", "Livre sterling", 2),
];

/// Natures d'écriture : (code, libelle, rules).
///
/// La nature est une dimension obligatoire des écritures : deux écritures de
/// natures différentes ne sont jamais agrégées. `0LIASS` est la nature par
/// défaut de la saisie de liasse sociale.
const NATURES: &[(&str, &str, Option<&str>)] =
    &[("0LIASS", "Liasse", None), ("1AJUST", "Ajustement", None)];

/// Méthodes de consolidation : (code, libelle, consolidated).
///
/// Le flag `consolidated` pilote l'étape D (cf. `pipeline::consolidate`) :
/// seules les méthodes `consolidated = true` sont reprises au niveau
/// `consolidated`. La mise en équivalence (`consolidated = false`) est exclue
/// du MVP — l'ajouter consisterait à basculer le flag, sans toucher au SQL.
const METHODS: &[(&str, &str, bool)] = &[
    ("globale", "Globale", true),
    ("proportionnelle", "Proportionnelle", true),
    ("equivalence", "Mise en équivalence", false),
    // Variante de la globale réservée à la société mère/consolidante (même
    // mécanique : consolidated = true, pct_integration = 1.0). Elle n'existe que
    // pour permettre aux règles de **cibler la mère seule** via un scope
    // `methode = 'MERE'` (cf. docs/QUESTIONS_OUVERTES.md Q33). Non rattachée à une
    // entité dans le seed → aucun impact sur les runs golden.
    ("MERE", "Globale — société mère", true),
];

// ─────────────────────────────────────────────────────────────────────────────
//  Tables satellites
// ─────────────────────────────────────────────────────────────────────────────

/// Périmètre de consolidation.
/// (entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie).
/// (perimeter_set, entity, period, methode), (pct_interet, pct_integration, entree, sortie).
const PERIMETER: &[((&str, &str, &str, &str), (Decimal, Decimal, bool, bool))] = &[
    (
        ("PERIM_REEL", "M", "2024", "globale"),
        (dec!(1.00), dec!(1.00), false, false),
    ),
    (
        ("PERIM_REEL", "A", "2024", "globale"),
        (dec!(1.00), dec!(1.00), true, false),
    ), // ENTRE en N
    (
        ("PERIM_REEL", "B", "2024", "globale"),
        (dec!(1.00), dec!(1.00), false, true),
    ), // SORT en N
];

/// Taux de change vers le pivot (EUR).
///   - period '2024' : taux clôture N (close_n), taux moyen (avg) et taux
///     d'ouverture (taux_ouverture = clôture N-1, résout `close_n1` sans période
///     antérieure).
/// (rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture).
///
/// Tous rattachés au jeu `'RATES'` (cf. `dim_rate_set`). La PK est désormais
/// `(rate_set, currency_source, period)`.
const RATES: &[((&str, &str, &str), (Option<Decimal>, Option<Decimal>, Option<Decimal>))] = &[
    (("RATES", "USD", "2023"), (Some(dec!(0.92)), None, None)),
    (
        ("RATES", "USD", "2024"),
        (Some(dec!(0.90)), Some(dec!(0.95)), Some(dec!(0.92))),
    ), // close_n = 0.90, avg = 0.95, ouverture = 0.92 (clôture 2023)
    (("RATES", "GBP", "2023"), (Some(dec!(1.15)), None, None)),
    (
        ("RATES", "GBP", "2024"),
        (Some(dec!(1.12)), Some(dec!(1.18)), Some(dec!(1.15))),
    ), // close_n = 1.12, avg = 1.18, ouverture = 1.15 (clôture 2023)
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
/// (phase, entity, entry_period, period, account, flow, currency, nature,
///  partner, share, analysis, source, amount).
/// Le 1er champ est la **phase** de la remontée (ex. 'REEL'). Le 12e champ est
/// la **référence source** (`S-M-001`…), métadonnée non-dimensionnelle insérée
/// dans `stg_entry.source` (et NON dans `analysis2`, qui est une dimension
/// analytique : l'y mettre ferait de chaque ligne un « dont »).
type RawRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    &'static str,
    Decimal,
);

const RAW: &[RawRow] = &[
    // ── Mère M (EUR) — périmètre continu ──
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "100",
        "F00",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-001",
        dec!(10000),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "400",
        "F00",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-002",
        dec!(5000),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "400",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-003",
        dec!(500),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "400",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-004",
        dec!(300),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "200",
        "F00",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-005",
        dec!(12000),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "200",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-006",
        dec!(500),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "300",
        "F00",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-007",
        dec!(3000),
    ),
    // Comptes de P&L (classe « resultat ») — F20 uniquement
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "700",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-010",
        dec!(2000),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "705",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-011",
        dec!(1000),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "600",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-012",
        dec!(800),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "610",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-013",
        dec!(500),
    ),
    (
        "REEL",
        "M",
        "2024",
        "2024",
        "640",
        "F20",
        "EUR",
        "0LIASS",
        None,
        None,
        None,
        "S-M-014",
        dec!(200),
    ),
    // ── Filiale A (USD) — ENTRE en N ──
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "100",
        "F00",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-001",
        dec!(5000),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "400",
        "F00",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-002",
        dec!(2000),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "400",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-003",
        dec!(300),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "200",
        "F00",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-004",
        dec!(8000),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "200",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-005",
        dec!(400),
    ),
    // Comptes de P&L — F20 uniquement
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "700",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-010",
        dec!(1000),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "705",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-011",
        dec!(500),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "600",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-012",
        dec!(400),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "610",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-013",
        dec!(200),
    ),
    (
        "REEL",
        "A",
        "2024",
        "2024",
        "640",
        "F20",
        "USD",
        "0LIASS",
        None,
        None,
        None,
        "S-A-014",
        dec!(100),
    ),
    // ── Filiale B (GBP) — SORT en N ──
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "100",
        "F00",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-001",
        dec!(4000),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "400",
        "F00",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-002",
        dec!(1500),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "400",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-003",
        dec!(200),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "200",
        "F00",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-004",
        dec!(6000),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "200",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-005",
        dec!(300),
    ),
    // Comptes de P&L — F20 uniquement
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "700",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-010",
        dec!(800),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "705",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-011",
        dec!(400),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "600",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-012",
        dec!(300),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "610",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-013",
        dec!(200),
    ),
    (
        "REEL",
        "B",
        "2024",
        "2024",
        "640",
        "F20",
        "GBP",
        "0LIASS",
        None,
        None,
        None,
        "S-B-014",
        dec!(100),
    ),
];

/// Insère toutes les données de test : master data, satellites et saisie brute.
///
/// Miroir de `conso/seed.py::seed_all`.
pub fn seed_all(con: &Connection) -> duckdb::Result<()> {
    // --- Config applicative ---
    for (k, v) in APP_CONFIG {
        con.execute("INSERT INTO app_config VALUES (?, ?)", params![k, v])?;
    }

    // --- Nouvelles dimensions référentielles (avant dim_scenario) ---
    for c in SCENARIO_CATEGORIES {
        con.execute(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES (?, ?)",
            params![c.0, c.1],
        )?;
    }
    for v in VARIANTS {
        con.execute(
            "INSERT INTO dim_variant (code, libelle) VALUES (?, ?)",
            params![v.0, v.1],
        )?;
    }
    for r in RATE_SETS {
        con.execute(
            "INSERT INTO dim_rate_set (code, libelle) VALUES (?, ?)",
            params![r.0, r.1],
        )?;
    }
    for ps in PERIMETER_SETS {
        con.execute(
            "INSERT INTO dim_perimeter_set (code, libelle) VALUES (?, ?)",
            params![ps.0, ps.1],
        )?;
    }

    // --- Dimensions ---
    for c in CONSOLIDATIONS {
        // `id` alloué par nextval : déterministe (1 sur base fraîche) et avance
        // la séquence (évite tout collision avec un INSERT ultérieur).
        con.execute(
            "INSERT INTO dim_consolidation
                (id, libelle, phase, exercice, perimeter_set, variant,
                 presentation_currency, perimeter_period, rate_set, rate_period,
                 ruleset_code, a_nouveau_consolidation_id, statut)
             VALUES (nextval('seq_consolidation'),
                     ?,
                     (SELECT id FROM dim_scenario_category WHERE code = ?),
                     ?,
                     (SELECT id FROM dim_perimeter_set WHERE code = ?),
                     (SELECT id FROM dim_variant WHERE code = ?),
                     ?, ?,
                     (SELECT id FROM dim_rate_set WHERE code = ?),
                     ?, ?, NULL, ?)",
            params![c.0, c.1, c.2, c.3, c.4, c.5, c.6, c.7, c.8, c.9, c.10],
        )?;
    }
    for e in ENTITIES {
        con.execute(
            "INSERT INTO dim_entity \
             (code, libelle, devise_fonctionnelle, entite_parent, statut) \
             VALUES (?, ?, ?, ?, ?)",
            params![e.0, e.1, e.2, e.3, e.4],
        )?;
    }
    for p in PERIODS {
        con.execute(
            "INSERT INTO dim_period \
             (code, libelle, type, date_debut, date_fin, statut) \
             VALUES (?, ?, ?, ?, ?, ?)",
            params![p.0, p.1, p.2, p.3, p.4, p.5],
        )?;
    }
    for sc in SOUS_CLASSES {
        con.execute(
            "INSERT INTO dim_sous_classe (code, libelle, classe) VALUES (?, ?, ?)",
            params![sc.0, sc.1, sc.2],
        )?;
    }
    for a in ACCOUNTS {
        // Liste de colonnes explicite : `flow_scheme` (ajoutée) reste NULL — le
        // schéma par défaut est dérivé de la classe à la conversion (cf.
        // pipeline::convert), surchargeable ensuite via le CRUD.
        con.execute(
            "INSERT INTO dim_account (code, libelle, classe, sous_classe) VALUES (?, ?, ?, ?)",
            params![a.0, a.1, a.2, a.3],
        )?;
    }
    for f in FLOWS {
        con.execute(
            "INSERT INTO dim_flow (code, libelle) VALUES (?, ?)",
            params![f.0, f.1],
        )?;
    }
    for s in FLOW_SCHEMES {
        con.execute(
            "INSERT INTO dim_flow_scheme (code, libelle) VALUES (?, ?)",
            params![s.0, s.1],
        )?;
    }
    for i in FLOW_SCHEME_ITEMS {
        con.execute(
            "INSERT INTO sat_flow_scheme_item \
             (scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau) \
             VALUES (?, ?, ?, ?, ?, ?)",
            params![i.0, i.1, i.2, i.3, i.4, i.5],
        )?;
    }
    for c in CURRENCIES {
        con.execute(
            "INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES (?, ?, ?)",
            params![c.0, c.1, c.2],
        )?;
    }
    for n in NATURES {
        con.execute(
            "INSERT INTO dim_nature (code, libelle, rules) VALUES (?, ?, ?)",
            params![n.0, n.1, n.2],
        )?;
    }
    for m in METHODS {
        con.execute(
            "INSERT INTO dim_method (code, libelle, consolidated) VALUES (?, ?, ?)",
            params![m.0, m.1, m.2],
        )?;
    }

    // --- Tables satellites ---
    for (k, v) in PERIMETER {
        con.execute(
            "INSERT INTO sat_perimeter \
             (perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![k.0, k.1, k.2, k.3, Money(v.0), Money(v.1), v.2, v.3],
        )?;
    }
    for (k, v) in RATES {
        con.execute(
            "INSERT INTO sat_exchange_rate \
             (rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture) \
             VALUES ((SELECT id FROM dim_rate_set WHERE code = ?), ?, ?, ?, ?, ?)",
            params![k.0, k.1, k.2, v.0.map(Money), v.1.map(Money), v.2.map(Money),],
        )?;
    }

    // --- Staging (saisie brute) ---
    for row in RAW {
        // `analysis2` reste NULL (dimension analytique) ; la réf. source (row.11)
        // va dans la colonne non-dimensionnelle `source`. Le 1er champ (row.0)
        // est la `phase` de la remontée (ex. 'REEL').
        con.execute(
            "INSERT INTO stg_entry \
                (phase, entity, entry_period, period, account, flow, currency, \
                 nature, partner, share, analysis, analysis2, source, amount) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)",
            params![
                row.0,
                row.1,
                row.2,
                row.3,
                row.4,
                row.5,
                row.6,
                row.7,
                row.8,
                row.9,
                row.10,
                row.11,
                Money(row.12),
            ],
        )?;
    }

    Ok(())
}

/// Seede la **démo d'élimination interco** : la règle `ELI_700` + le jeu
/// `RS_INTERCO`. Elle est absente des CSV car la `definition` est du JSON, mal
/// adapté au format CSV ; on la pose donc en code (même esprit que [`seed_all`]).
///
/// Reproduit la fixture canonique de `tests/rules.rs::elim_700_json` : élimine
/// l'interco du compte 700 entre entités en méthode `globale` (op 1 extourne
/// partner hérité, op 2 pose la contrepartie partner vidé, le tout en nature
/// `2ELI`). La consolidation `REEL` référence `RS_INTERCO` (cf.
/// `consolidations.csv`), et `entries.csv` contient la ligne interco M→A sur 700
/// que la règle matche.
///
/// Appelée par le serveur **après l'import CSV** (démarrage sur base vierge /
/// `POST /api/reset`), pas par [`seed_all`] (les tests créent leurs propres
/// règles). À exécuter sur un schéma fraîchement créé → INSERT simples.
pub fn seed_demo_rules(con: &Connection) -> duckdb::Result<()> {
    const ELI_700: &str = r#"{
        "scope": [
            {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
            {"target": "partner", "dim": "methode", "op": "=", "val": "globale"}
        ],
        "operations": [
            {
                "seq": 1, "level": "consolidated",
                "selection": [
                    {"dim": "account", "op": "=", "val": "700"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "pct_integration"},
                "multiplicateur": -1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "inherit"}
                }
            },
            {
                "seq": 2, "level": "consolidated",
                "selection": [
                    {"dim": "account", "op": "=", "val": "700"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "pct_integration"},
                "multiplicateur": 1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "null"}
                }
            }
        ]
    }"#;
    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
        params!["ELI_700", "Élimination interco 700", ELI_700],
    )?;
    con.execute(
        "INSERT INTO dim_ruleset (code, libelle) VALUES (?, ?)",
        params!["RS_INTERCO", "Élimination interco (démo)"],
    )?;
    con.execute(
        "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) VALUES (?, ?, ?)",
        params!["RS_INTERCO", 1, "ELI_700"],
    )?;
    Ok(())
}

/// Seede les **attributs de dimension du plan de comptes** : recrée, via les
/// mécanismes pilotables, ce qui était auparavant codé en dur sur `dim_account`.
///
/// - **Caractéristique** `groupement` (N1 sur `account`) avec la valeur
///   `capitaux_propres`, affectée au compte `10` (Capital et réserves) ;
/// - **Référence directe** `compte_parent` (patron B) sur `account → account`
///   puis **chargement de la hiérarchie** depuis `account_parents.csv` (`code,
///   compte_parent`) si le fichier est présent dans `data_dir`.
///
/// Appelée par le serveur **après l'import CSV** (démarrage sur base vierge /
/// `POST /api/reset`), comme [`seed_demo_rules`]. Renvoie [`AppError`] car elle
/// s'appuie sur les fonctions de haut niveau (validation incluse).
///
/// **Idempotente** : les registres de caractéristiques / références directes
/// survivent au reset (hors `ALL_DROP`) ; on ne recrée donc la caractéristique
/// et la référence que si elles sont absentes, et la hiérarchie est rejouée à
/// chaque appel (UPDATE depuis le CSV).
pub fn seed_demo_attributes(
    con: &Connection,
    data_dir: &std::path::Path,
) -> Result<(), crate::state::AppError> {
    use crate::{characteristics, custom_references};
    use serde_json::{json, Map};

    // La caractéristique « groupement » et la référence « compte_parent » vivent
    // dans des registres qui survivent au reset : ne (re)créer que si absentes.
    let groupement_present: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_characteristic WHERE code = 'groupement'",
            [],
            |r| r.get(0),
        )
        .map_err(crate::state::db_err)?;
    if !groupement_present {
        // Caractéristique « groupement » + valeur « capitaux_propres » sur le compte 10.
        characteristics::create_characteristic(
            con,
            "groupement",
            "Groupement technique",
            "account",
        )?;
        let mut val = Map::new();
        val.insert("code".into(), json!("capitaux_propres"));
        val.insert("libelle".into(), json!("Capitaux propres"));
        characteristics::create_value(con, "groupement", &val)?;
        characteristics::assign(con, "groupement", "10", Some("capitaux_propres"))?;
    }

    let compte_parent_present: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM dim_custom_reference \
             WHERE host_dimension = 'account' AND column_name = 'compte_parent'",
            [],
            |r| r.get(0),
        )
        .map_err(crate::state::db_err)?;
    if !compte_parent_present {
        // Référence directe « compte_parent » : account → account (hiérarchie).
        custom_references::create(con, "account", "compte_parent", "account")?;
    }

    // Chargement de la hiérarchie depuis `account_parents.csv` (si présent).
    // UPDATE en masse plutôt que 800+ appels `assign` : les valeurs viennent du
    // plan de comptes (CSV de confiance) et la colonne existe (réf. ci-dessus).
    let parents_csv = data_dir.join("account_parents.csv");
    if parents_csv.exists() {
        let path = parents_csv.display().to_string();
        con.execute(
            &format!(
                "UPDATE dim_account AS a SET compte_parent = src.compte_parent \
                 FROM read_csv('{path}', auto_detect=false, \
                     columns={{'code':'VARCHAR','compte_parent':'VARCHAR'}}, \
                     header=true, delim=',') AS src \
                 WHERE a.code = src.code"
            ),
            [],
        )
        .map_err(crate::state::db_err)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn parent_of(con: &Connection, code: &str) -> Option<String> {
        con.query_row(
            "SELECT compte_parent FROM dim_account WHERE code = ?",
            [code],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// `seed_demo_attributes` charge la hiérarchie `compte_parent` depuis
    /// `account_parents.csv` (via `UPDATE ... FROM read_csv`) et reste idempotente
    /// (registres caractéristique / référence directe survivant au reset).
    #[test]
    fn charge_la_hierarchie_compte_parent_et_reste_idempotente() {
        let con = Connection::open_in_memory().expect("open in-memory");
        crate::schema::create_schema(&con).expect("create_schema");
        con.execute_batch(
            "INSERT INTO dim_account (code, libelle, classe, sous_classe) VALUES \
                ('10','Capital et réserves','bilan','passif'), \
                ('101','Capital','bilan','passif'), \
                ('1011','Capital souscrit','bilan','passif');",
        )
        .expect("seed accounts");

        // CSV de hiérarchie dans un répertoire temporaire unique.
        let dir = std::env::temp_dir().join(format!("conso_seed_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("account_parents.csv")).unwrap();
        writeln!(f, "code,compte_parent\n101,10\n1011,101").unwrap();
        drop(f);

        seed_demo_attributes(&con, &dir).expect("seed_demo_attributes");
        assert_eq!(parent_of(&con, "10"), None, "racine sans parent");
        assert_eq!(parent_of(&con, "101").as_deref(), Some("10"));
        assert_eq!(parent_of(&con, "1011").as_deref(), Some("101"));

        // Deuxième appel (simule POST /api/reset) : pas de conflit de registre.
        seed_demo_attributes(&con, &dir).expect("second seed_demo_attributes idempotent");
        assert_eq!(parent_of(&con, "1011").as_deref(), Some("101"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
