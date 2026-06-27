//! Chargement des données depuis fichiers CSV via `read_csv_auto()` de DuckDB.
//!
//! Remplace le seed en dur (`seed.rs`) par une lecture de fichiers CSV depuis
//! un répertoire `data/`. Aucune crate CSV externe : DuckDB fait tout.
//!
//! # Tables attendues et leurs fichiers
//!
//! | Fichier                     | Table cible              | Remarques                          |
//! |-----------------------------|--------------------------|------------------------------------|
//! | `app_config.csv`            | `app_config`             | singleton clé/valeur               |
//! | `scenario_categories.csv`   | `dim_scenario_category`  | lecture directe                    |
//! | `variants.csv`              | `dim_variant`            | lecture directe                    |
//! | `rate_sets.csv`             | `dim_rate_set`           | lecture directe                    |
//! | `consolidations.csv`        | `dim_consolidation`      | PK auto `id` (colonnes sans id)    |
//! | `entities.csv`              | `dim_entity`             | lecture directe                    |
//! | `periods.csv`               | `dim_period`             | lecture directe                    |
//! | `sous_classes.csv`          | `dim_sous_classe`        | lecture directe                    |
//! | `accounts.csv`              | `dim_account`            | schéma explicite + null_padding    |
//! | `flows.csv`                 | `dim_flow`               | lecture directe                    |
//! | `currencies.csv`            | `dim_currency`           | CAST `decimales` AS INTEGER        |
//! | `natures.csv`               | `dim_nature`             | lecture directe                    |
//! | `methods.csv`               | `dim_method`             | CAST `consolidated` AS BOOLEAN     |
//! | `perimeter.csv`             | `sat_perimeter`          | CAST `entree`/`sortie` AS BOOLEAN  |
//! | `rates.csv`                 | `sat_exchange_rate`      | `rate_set` en 1ère colonne (v2)    |
//! | `entries.csv`               | `stg_entry`              | 1ère colonne `phase`               |
//!
//! Les cellules vides sont lues comme NULL par `read_csv_auto`.
//!
//! L'ordre d'insertion respecte les FK logiques (cf. schema.rs commentaire
//! `ALL_DDL`) : `app_config` et `dim_rate_set` avant `sat_exchange_rate` ;
//! `dim_scenario_category`, `dim_variant` avant `dim_consolidation`.
//!
//! # Registre déclaratif
//!
//! La fonction [`load_all`] itère sur [`CSV_MAPPINGS`], un tableau `const`
//! ordonné selon les dépendances FK. Chaque entrée décrit un mapping
//! fichier → table → colonnes (+ casts éventuels). Ajouter un CSV = ajouter
//! une ligne au registre, sans toucher au code de génération SQL.

use duckdb::Connection;
use std::path::Path;

use crate::references;

/// Description d'un chargement CSV → table.
///
/// - `columns` : colonnes sélectionnées **dans l'ordre du SELECT** (et de la
///   table cible). Doivent correspondre aux en-têtes du fichier, sauf si
///   `use_explicit_schema` est activé (auquel cas l'ordre vient du dictionnaire
///   `columns={...}` passé au lecteur DuckDB).
/// - `casts` : paires `(colonne, type_sql)` pour les colonnes que
///   `read_csv_auto` peut mal inférer (BOOLEAN, INTEGER). Les autres colonnes
///   sont lues telles quelles.
/// - `use_explicit_schema` : quand `true`, on utilise `read_csv(..., columns={...})`
///   avec toutes les colonnes typées en `VARCHAR`, `header=true`, `delim=','`
///   et `null_padding=true`. Cas unique : `accounts.csv` (lignes incomplètes
///   en queue de fichier → `null_padding` obligatoire).
struct CsvMapping {
    file: &'static str,
    table: &'static str,
    columns: &'static [&'static str],
    casts: &'static [(&'static str, &'static str)],
    use_explicit_schema: bool,
}

/// Registre ordonné des mappings CSV → table.
///
/// L'ordre suit les dépendances FK logiques (cf. `ALL_DDL` dans schema.rs) :
/// `app_config` et `dim_rate_set` avant `sat_exchange_rate` ;
/// `dim_scenario_category`, `dim_variant` avant `dim_consolidation`.
static CSV_MAPPINGS: &[CsvMapping] = &[
    // 1. app_config : singleton clé/valeur (ex: pivot_currency=EUR).
    CsvMapping {
        file: "app_config.csv",
        table: "app_config",
        columns: &["key", "value"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 2. dim_scenario_category : phase de la remontée (REEL, BUDGET, ...).
    CsvMapping {
        file: "scenario_categories.csv",
        table: "dim_scenario_category",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 3. dim_variant : variante de la consolidation (réel, budget, ...).
    CsvMapping {
        file: "variants.csv",
        table: "dim_variant",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 4. dim_rate_set : jeu de taux (référencé par sat_exchange_rate).
    CsvMapping {
        file: "rate_sets.csv",
        table: "dim_rate_set",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 4b. dim_perimeter_set : jeu de périmètre (référencé par sat_perimeter, Q35).
    CsvMapping {
        file: "perimeter_sets.csv",
        table: "dim_perimeter_set",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 5a. dim_period : périodes (type, bornes, statut).
    //     Doit précéder dim_consolidation : exercice / perimeter_period / rate_period
    //     sont désormais des FK dim_period.id (B1).
    CsvMapping {
        file: "periods.csv",
        table: "dim_period",
        columns: &[
            "code",
            "libelle",
            "type",
            "date_debut",
            "date_fin",
            "statut",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
    // 5b. dim_currency : déplacée avant dim_consolidation (B1) — presentation_currency
    //     est désormais une FK dim_currency.id. Supprimée de sa position originale.
    CsvMapping {
        file: "currencies.csv",
        table: "dim_currency",
        columns: &["code_iso", "libelle", "decimales"],
        casts: &[("decimales", "INTEGER")],
        use_explicit_schema: false,
    },
    // 5c. dim_consolidation (ex dim_scenario) : PK technique auto `id`, omise du
    //     mapping (laissée au DEFAULT nextval). B1 : exercice / presentation_currency /
    //     perimeter_period / rate_period / ruleset_code stockés en id — résolus par
    //     build_insert_sql via les ri() du graphe references.rs.
    CsvMapping {
        file: "consolidations.csv",
        table: "dim_consolidation",
        columns: &[
            "libelle",
            "phase",
            "exercice",
            "perimeter_set",
            "variant",
            "presentation_currency",
            "perimeter_period",
            "rate_set",
            "rate_period",
            "ruleset_code",
            "a_nouveau_consolidation_id",
            "statut",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
    // 6. dim_entity : entités consolidées (devise, parent, statut).
    CsvMapping {
        file: "entities.csv",
        table: "dim_entity",
        columns: &[
            "code",
            "libelle",
            "devise_fonctionnelle",
            "entite_parent",
            "statut",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
    // 8. dim_sous_classe : sous-classes de compte (réf. de dim_account).
    //    `sens` (Q44) est optionnel (C/D) — absent du CSV ⇒ NULL.
    CsvMapping {
        file: "sous_classes.csv",
        table: "dim_sous_classe",
        columns: &["code", "libelle", "classe", "sens"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 9. dim_account : plan de compte. Schéma explicite + null_padding car le
    //    fichier peut contenir des lignes incomplètes (sous_classe vide). La 5e
    //    colonne `flow_scheme` est 100 % user-driven (Q45 : plus de défaut dérivé
    //    de la classe) — cf. v_flow_behavior. Nullable (option b : compte sans
    //    schéma toléré mais exclu de la conversion/clôture).
    CsvMapping {
        file: "accounts.csv",
        table: "dim_account",
        columns: &["code", "libelle", "classe", "sous_classe", "flow_scheme"],
        casts: &[],
        use_explicit_schema: true,
    },
    // 10. dim_flow : dimension nue (code, libelle). Le comportement vit dans les
    //     schémas de flux (flow_scheme_items), résolu par compte via v_flow_behavior.
    CsvMapping {
        file: "flows.csv",
        table: "dim_flow",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 10b. dim_flow_scheme : catalogue des schémas de flux (cf. Q32).
    CsvMapping {
        file: "flow_schemes.csv",
        table: "dim_flow_scheme",
        columns: &["code", "libelle"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 11. (dim_currency déplacée en 5b — avant dim_consolidation, B1).
    // 12. dim_nature : natures de traitement (code, libelle, rules JSON).
    CsvMapping {
        file: "natures.csv",
        table: "dim_nature",
        columns: &["code", "libelle", "rules"],
        casts: &[],
        use_explicit_schema: false,
    },
    // 12b. dim_method : méthodes de consolidation (cf. pipeline::consolidate).
    //      CAST consolidated AS BOOLEAN (sinon lu en VARCHAR/TINYINT par read_csv_auto).
    //      Chargé avant sat_perimeter (qui référence dim_method.code logiquement).
    CsvMapping {
        file: "methods.csv",
        table: "dim_method",
        columns: &["code", "libelle", "consolidated"],
        casts: &[("consolidated", "BOOLEAN")],
        use_explicit_schema: false,
    },
    // 13. sat_perimeter : périmètre de consolidation. CAST entree/sortie AS BOOLEAN.
    CsvMapping {
        file: "perimeter.csv",
        table: "sat_perimeter",
        columns: &[
            "perimeter_set",
            "entity",
            "period",
            "methode",
            "pct_interet",
            "pct_integration",
            "entree",
            "sortie",
        ],
        casts: &[("entree", "BOOLEAN"), ("sortie", "BOOLEAN")],
        use_explicit_schema: false,
    },
    // 14. sat_exchange_rate (v2) : `rate_set` en 1ère colonne (PK étendue).
    //      `taux_ouverture` = clôture N-1 portée par N (résout `close_n1`).
    CsvMapping {
        file: "rates.csv",
        table: "sat_exchange_rate",
        columns: &[
            "rate_set",
            "currency_source",
            "period",
            "taux_close",
            "taux_moyen",
            "taux_ouverture",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
    // 14b. sat_flow_scheme_item : articulation complète des flux par schéma (Q32).
    CsvMapping {
        file: "flow_scheme_items.csv",
        table: "sat_flow_scheme_item",
        columns: &[
            "scheme",
            "flow",
            "taux_conversion",
            "flux_ecart",
            "flux_de_report",
            "flux_a_nouveau",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
    // 15. stg_entry : saisie brute (liasses sociales) — 13 colonnes. La 1ère est
    //     `phase` (remontée), pas `scenario`.
    CsvMapping {
        file: "entries.csv",
        table: "stg_entry",
        columns: &[
            "phase",
            "entity",
            "entry_period",
            "period",
            "account",
            "flow",
            "currency",
            "nature",
            "partner",
            "share",
            "analysis",
            "analysis2",
            "amount",
        ],
        casts: &[],
        use_explicit_schema: false,
    },
];

/// Construit la clause `SELECT ... FROM <reader>` pour un mapping.
///
/// - Les colonnes sans cast sont sélectionnées telles quelles.
/// - Les colonnes dans `casts` sont enveloppées dans `CAST(col AS TYPE)`.
/// - Le lecteur est `read_csv_auto(...)` par défaut, ou `read_csv(...)` avec
///   schéma explicite (toutes colonnes en VARCHAR) + `null_padding` quand
///   `use_explicit_schema` est activé.
///
/// Le SQL produit est identique à celui des 15 INSERT historiques codés en dur.
fn build_insert_sql(m: &CsvMapping, csv_path: &str) -> String {
    // Le CSV fournit des **codes** ; une colonne migrée en clé technique (option A,
    // chantier B1) est résolue code→id par sous-requête sur sa dimension cible.
    // Le reader est aliasé `src` pour permettre la corrélation.
    let select_cols: Vec<String> = m
        .columns
        .iter()
        .map(|col| {
            if let Some(r) = references::REFERENCES.iter().find(|r| {
                r.table == m.table && r.column == *col && r.target_display_column.is_some()
            }) {
                let display = r.target_display_column.unwrap();
                // Code → id : INNER lookup ; code absent ⇒ NULL (cohérent avec la
                // nullabilité ; la validation des données le signalerait).
                format!(
                    "(SELECT t.id FROM {} t WHERE t.\"{}\" = src.\"{}\")",
                    r.target_table, display, col
                )
            } else {
                match m.casts.iter().find(|(c, _)| c == col) {
                    Some((_, ty)) => format!("CAST(src.\"{col}\" AS {ty})"),
                    None => format!("src.\"{col}\""),
                }
            }
        })
        .collect();

    let reader = if m.use_explicit_schema {
        let cols_dict: Vec<String> = m
            .columns
            .iter()
            .map(|c| format!("'{c}':'VARCHAR'"))
            .collect();
        format!(
            "read_csv('{csv_path}', auto_detect=false, columns={{{}}}, header=true, delim=',', null_padding=true)",
            cols_dict.join(",")
        )
    } else {
        format!("read_csv_auto('{csv_path}')")
    };

    // Liste de colonnes **explicite** : robuste si la table porte des colonnes
    // hors CSV (ex. `stg_entry.source`, non-dimensionnelle) — elles restent NULL.
    format!(
        "INSERT INTO {} ({}) SELECT {} FROM {} AS src",
        m.table,
        m.columns.join(", "),
        select_cols.join(", "),
        reader
    )
}

/// Charge tous les CSV d'un répertoire dans les tables du schéma.
///
/// Itère sur [`CSV_MAPPINGS`] (registre déclaratif ordonné selon les FK) et
/// exécute le SQL généré par [`build_insert_sql`]. Réutilise l'inférence de
/// types de DuckDB ; les CAST explicites (BOOLEAN, INTEGER) couvrent les
/// colonnes que `read_csv_auto` peut mal inférer.
///
/// # Erreurs
///
/// Toute erreur DuckDB (fichier manquant, type incompatible) remonte
/// immédiatement et interrompt le chargement.
pub fn load_all(con: &Connection, data_dir: &Path) -> duckdb::Result<()> {
    for m in CSV_MAPPINGS {
        let csv_path = data_dir.join(m.file).display().to_string();
        let sql = build_insert_sql(m, &csv_path);
        con.execute(&sql, [])?;
    }
    Ok(())
}
