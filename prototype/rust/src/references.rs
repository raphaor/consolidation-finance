//! Registre central des **références** entre tables (le graphe de jointures).
//!
//! Le modèle n'a pas de FK dures (DuckDB, choix du proto) : les liens entre
//! dimensions/objets reposent sur des codes en texte. Ce module **déclare** ces
//! liens en un seul endroit pour qu'ils deviennent vérifiables — une `(table,
//! colonne)` pointe vers une `(table_cible, colonne_cible)`. C'est la source de
//! vérité commune à :
//!
//! - la validation à l'écriture (master data, imports CSV, définitions de règles) ;
//! - (à venir) les dropdowns de l'UI et un endpoint « santé des données ».
//!
//! Les dimensions d'écriture sont déclarées sous `"stg_entry"` ; `fact_entry`
//! partageant les mêmes colonnes, [`entry_dimension_target`] sert aussi à valider
//! les valeurs des règles (selection / destination).

use duckdb::Connection;

/// Un lien référentiel : `table.column` doit exister dans `target_table.target_column`.
pub struct Reference {
    pub table: &'static str,
    pub column: &'static str,
    pub target_table: &'static str,
    pub target_column: &'static str,
}

const fn r(
    table: &'static str,
    column: &'static str,
    target_table: &'static str,
    target_column: &'static str,
) -> Reference {
    Reference { table, column, target_table, target_column }
}

/// Le graphe complet des références du modèle.
///
/// Les auto-références (`dim_flow.flux_de_report → dim_flow.code`,
/// `dim_account.compte_parent → dim_account.code`, `dim_entity.entite_parent →
/// dim_entity.code`) sont incluses : la validation à l'écriture tolère la valeur
/// égale à la PK de la ligne elle-même (cf. `masterdata::validate_references`).
pub const REFERENCES: &[Reference] = &[
    // dim_scenario (v2)
    r("dim_scenario", "category", "dim_scenario_category", "code"),
    r("dim_scenario", "entry_period", "dim_period", "code"),
    r("dim_scenario", "presentation_currency", "dim_currency", "code_iso"),
    r("dim_scenario", "variant", "dim_variant", "code"),
    r("dim_scenario", "ruleset_code", "dim_ruleset", "code"),
    r("dim_scenario", "rate_set", "dim_rate_set", "code"),
    // dim_entity
    r("dim_entity", "devise_fonctionnelle", "dim_currency", "code_iso"),
    r("dim_entity", "entite_parent", "dim_entity", "code"),
    // dim_account
    r("dim_account", "sous_classe", "dim_sous_classe", "code"),
    r("dim_account", "compte_parent", "dim_account", "code"),
    // dim_flow (auto-références : flux d'écart / de report)
    r("dim_flow", "flux_ecart", "dim_flow", "code"),
    r("dim_flow", "flux_de_report", "dim_flow", "code"),
    // sat_perimeter
    r("sat_perimeter", "entity", "dim_entity", "code"),
    r("sat_perimeter", "scenario", "dim_scenario", "code"),
    r("sat_perimeter", "period", "dim_period", "code"),
    r("sat_perimeter", "methode", "dim_method", "code"),
    // sat_exchange_rate
    r("sat_exchange_rate", "rate_set", "dim_rate_set", "code"),
    r("sat_exchange_rate", "currency_source", "dim_currency", "code_iso"),
    r("sat_exchange_rate", "period", "dim_period", "code"),
    // Écritures (stg_entry — mêmes cibles que fact_entry).
    // `analysis` / `analysis2` et les dimensions custom sont libres (pas de ref).
    r("stg_entry", "scenario", "dim_scenario", "code"),
    r("stg_entry", "entity", "dim_entity", "code"),
    r("stg_entry", "entry_period", "dim_period", "code"),
    r("stg_entry", "period", "dim_period", "code"),
    r("stg_entry", "account", "dim_account", "code"),
    r("stg_entry", "flow", "dim_flow", "code"),
    r("stg_entry", "currency", "dim_currency", "code_iso"),
    r("stg_entry", "nature", "dim_nature", "code"),
    r("stg_entry", "partner", "dim_entity", "code"),
    r("stg_entry", "share", "dim_entity", "code"),
    // Jeux de règles
    r("dim_ruleset_item", "ruleset_code", "dim_ruleset", "code"),
    r("dim_ruleset_item", "rule_code", "dim_rule", "code"),
];

/// Les références portées par une table donnée.
pub fn references_for(table: &str) -> impl Iterator<Item = &'static Reference> {
    // `table` est copié (owned) pour ne pas faire fuiter sa lifetime dans le
    // type de retour `impl Iterator` (les items, eux, sont `'static`).
    let table = table.to_owned();
    REFERENCES.iter().filter(move |r| r.table == table.as_str())
}

/// Cible référentielle d'une dimension d'écriture (selection / destination des
/// règles). `None` = dimension libre (analysis, custom…).
pub fn entry_dimension_target(dim: &str) -> Option<&'static Reference> {
    REFERENCES
        .iter()
        .find(|r| r.table == "stg_entry" && r.column == dim)
}

/// Cible d'une colonne de `sat_perimeter` (scope des règles). `None` = colonne
/// sans référence (pct_interet, pct_integration, entree, sortie).
pub fn perimeter_target(col: &str) -> Option<&'static Reference> {
    REFERENCES
        .iter()
        .find(|r| r.table == "sat_perimeter" && r.column == col)
}

/// `true` si `value` existe dans `target_table.target_column`.
///
/// Sécurité : `target_table` / `target_column` proviennent du registre `const`
/// (jamais de l'utilisateur) → interpolation sûre ; `value` est paramétré.
pub fn value_exists(
    con: &Connection,
    target_table: &str,
    target_column: &str,
    value: &str,
) -> duckdb::Result<bool> {
    con.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {target_table} WHERE \"{target_column}\" = ?)"),
        [value],
        |row| row.get::<_, bool>(0),
    )
}
