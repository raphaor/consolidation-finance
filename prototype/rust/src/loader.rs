//! Chargement des données depuis fichiers CSV via `read_csv_auto()` de DuckDB.
//!
//! Remplace le seed en dur (`seed.rs`) par une lecture de fichiers CSV depuis
//! un répertoire `data/`. Aucune crate CSV externe : DuckDB fait tout.
//!
//! # Tables attendues et leurs fichiers
//!
//! | Fichier             | Table cible          | Remarques                          |
//! |---------------------|----------------------|------------------------------------|
//! | `scenarios.csv`     | `dim_scenario`       | lecture directe                    |
//! | `entities.csv`      | `dim_entity`         | lecture directe                    |
//! | `periods.csv`       | `dim_period`         | lecture directe                    |
//! | `sous_classes.csv`  | `dim_sous_classe`    | lecture directe                    |
//! | `accounts.csv`      | `dim_account`        | lecture directe                    |
//! | `flows.csv`         | `dim_flow`           | lecture directe                    |
//! | `currencies.csv`    | `dim_currency`       | CAST `decimales` AS INTEGER        |
//! | `perimeter.csv`     | `sat_perimeter`      | CAST `entree`/`sortie` AS BOOLEAN  |
//! | `rates.csv`         | `sat_exchange_rate`  | lecture directe                    |
//! | `entries.csv`       | `stg_entry`          | lecture directe                    |
//!
//! Les cellules vides sont lues comme NULL par `read_csv_auto`.

use duckdb::Connection;
use std::path::Path;

/// Charge tous les CSV d'un répertoire dans les tables du schéma.
///
/// Enchaîne 10 `INSERT ... SELECT ... FROM read_csv_auto(...)` en réutilisant
/// l'inférence de types de DuckDB. Les CAST explicites (BOOLEAN, INTEGER)
/// concernent les colonnes que `read_csv_auto` peut mal inférer (typiquement
/// `true`/`false` vus comme VARCHAR, ou les entiers courts).
///
/// # Erreurs
///
/// Toute erreur DuckDB (fichier manquant, type incompatible) remonte
/// immédiatement et interrompt le chargement.
pub fn load_all(con: &Connection, data_dir: &Path) -> duckdb::Result<()> {
    // Construit le chemin d'un fichier du répertoire `data_dir` sous forme de
    // chaîne, pour injection dans la clause `read_csv_auto('...')`.
    let csv_path = |file: &str| data_dir.join(file).display().to_string();

    // --- Dimensions (master data) ---
    con.execute(
        &format!(
            "INSERT INTO dim_scenario \
             SELECT code, libelle, type, statut \
             FROM read_csv_auto('{}')",
            csv_path("scenarios.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_entity \
             SELECT code, libelle, devise_fonctionnelle, entite_parent, statut \
             FROM read_csv_auto('{}')",
            csv_path("entities.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_period \
             SELECT code, libelle, type, date_debut, date_fin, statut \
             FROM read_csv_auto('{}')",
            csv_path("periods.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_sous_classe \
             SELECT code, libelle, classe \
             FROM read_csv_auto('{}')",
            csv_path("sous_classes.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_account \
             SELECT code, libelle, classe, sous_classe, technical_grouping, compte_parent \
             FROM read_csv('{}', auto_detect=false, columns={{'code':'VARCHAR','libelle':'VARCHAR','classe':'VARCHAR','sous_classe':'VARCHAR','technical_grouping':'VARCHAR','compte_parent':'VARCHAR'}}, header=true, delim=',', null_padding=true)",
            csv_path("accounts.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_flow \
             SELECT code, libelle, taux_conversion, flux_ecart, flux_de_report \
             FROM read_csv_auto('{}')",
            csv_path("flows.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO dim_currency \
             SELECT code_iso, libelle, CAST(decimales AS INTEGER) \
             FROM read_csv_auto('{}')",
            csv_path("currencies.csv")
        ),
        [],
    )?;

    // --- Tables satellites (règles de consolidation) ---
    con.execute(
        &format!(
            "INSERT INTO sat_perimeter \
             SELECT entity, scenario, period, methode, pct_interet, pct_integration, \
                    CAST(entree AS BOOLEAN), CAST(sortie AS BOOLEAN) \
             FROM read_csv_auto('{}')",
            csv_path("perimeter.csv")
        ),
        [],
    )?;
    con.execute(
        &format!(
            "INSERT INTO sat_exchange_rate \
             SELECT currency_source, period, taux_close, taux_moyen \
             FROM read_csv_auto('{}')",
            csv_path("rates.csv")
        ),
        [],
    )?;

    // --- Staging (saisie brute — liasses sociales) ---
    con.execute(
        &format!(
            "INSERT INTO stg_entry \
             SELECT scenario, entity, entry_period, period, account, flow, currency, \
                    partner, share, analysis, audit_id, amount \
             FROM read_csv_auto('{}')",
            csv_path("entries.csv")
        ),
        [],
    )?;

    Ok(())
}
