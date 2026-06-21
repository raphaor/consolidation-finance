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
///
/// `required` = la colonne est **non-nullable** : une valeur vide/absente est
/// rejetée à l'écriture (cf. `masterdata::validate_references`). Les colonnes
/// nullables (auto-références parent, attributs optionnels de scénario…) restent
/// à `false` : vide y est autorisé (= NULL).
pub struct Reference {
    pub table: &'static str,
    pub column: &'static str,
    pub target_table: &'static str,
    pub target_column: &'static str,
    pub required: bool,
}

/// Référence **optionnelle** (colonne nullable) : vide autorisé.
const fn r(
    table: &'static str,
    column: &'static str,
    target_table: &'static str,
    target_column: &'static str,
) -> Reference {
    Reference {
        table,
        column,
        target_table,
        target_column,
        required: false,
    }
}

/// Référence **obligatoire** (colonne non-nullable) : vide rejeté.
const fn rq(
    table: &'static str,
    column: &'static str,
    target_table: &'static str,
    target_column: &'static str,
) -> Reference {
    Reference {
        table,
        column,
        target_table,
        target_column,
        required: true,
    }
}

/// Le graphe complet des références du modèle.
///
/// Les auto-références statiques (`dim_flow.flux_de_report → dim_flow.code`,
/// `dim_entity.entite_parent → dim_entity.code`) sont incluses : la validation à
/// l'écriture tolère la valeur égale à la PK de la ligne elle-même (cf.
/// `masterdata::validate_references`). L'ancienne `dim_account.compte_parent` est
/// désormais une **référence directe** dynamique (cf. [`dynamic_references`] et
/// `crate::custom_references`), donc absente de cette liste statique.
pub const REFERENCES: &[Reference] = &[
    // dim_scenario (v2)
    r("dim_scenario", "category", "dim_scenario_category", "code"),
    r("dim_scenario", "entry_period", "dim_period", "code"),
    r(
        "dim_scenario",
        "presentation_currency",
        "dim_currency",
        "code_iso",
    ),
    r("dim_scenario", "variant", "dim_variant", "code"),
    r("dim_scenario", "ruleset_code", "dim_ruleset", "code"),
    r("dim_scenario", "rate_set", "dim_rate_set", "code"),
    r("dim_scenario", "perimeter_set", "dim_perimeter_set", "code"),
    // Conso d'à-nouveau : auto-référence vers un autre scénario (N-1 figé).
    r("dim_scenario", "a_nouveau_scenario", "dim_scenario", "code"),
    // dim_entity
    rq(
        "dim_entity",
        "devise_fonctionnelle",
        "dim_currency",
        "code_iso",
    ),
    r("dim_entity", "entite_parent", "dim_entity", "code"),
    // dim_account (compte_parent est désormais une référence directe dynamique)
    r("dim_account", "sous_classe", "dim_sous_classe", "code"),
    // Schéma de flux du compte (nullable : NULL = défaut dérivé de la classe).
    r("dim_account", "flow_scheme", "dim_flow_scheme", "code"),
    // dim_flow est désormais une dimension nue (code, libelle) : tout le
    // comportement (taux, écart, report, à-nouveau) vit dans sat_flow_scheme_item.
    // sat_perimeter (perimeter_set/entity/period = PK ; methode obligatoire)
    rq(
        "sat_perimeter",
        "perimeter_set",
        "dim_perimeter_set",
        "code",
    ),
    rq("sat_perimeter", "entity", "dim_entity", "code"),
    rq("sat_perimeter", "period", "dim_period", "code"),
    rq("sat_perimeter", "methode", "dim_method", "code"),
    // sat_exchange_rate (rate_set/currency_source/period = PK)
    rq("sat_exchange_rate", "rate_set", "dim_rate_set", "code"),
    rq(
        "sat_exchange_rate",
        "currency_source",
        "dim_currency",
        "code_iso",
    ),
    rq("sat_exchange_rate", "period", "dim_period", "code"),
    // sat_flow_scheme_item (scheme/flow = PK ; flux_* nullables vers dim_flow)
    rq("sat_flow_scheme_item", "scheme", "dim_flow_scheme", "code"),
    rq("sat_flow_scheme_item", "flow", "dim_flow", "code"),
    r("sat_flow_scheme_item", "flux_ecart", "dim_flow", "code"),
    r("sat_flow_scheme_item", "flux_de_report", "dim_flow", "code"),
    r("sat_flow_scheme_item", "flux_a_nouveau", "dim_flow", "code"),
    // Écritures (stg_entry — mêmes cibles que fact_entry).
    // `analysis` / `analysis2` et les dimensions custom sont libres (pas de ref).
    // `partner` / `share` sont nullables ; les autres dimensions sont obligatoires.
    rq("stg_entry", "scenario", "dim_scenario", "code"),
    rq("stg_entry", "entity", "dim_entity", "code"),
    rq("stg_entry", "entry_period", "dim_period", "code"),
    rq("stg_entry", "period", "dim_period", "code"),
    rq("stg_entry", "account", "dim_account", "code"),
    rq("stg_entry", "flow", "dim_flow", "code"),
    rq("stg_entry", "currency", "dim_currency", "code_iso"),
    rq("stg_entry", "nature", "dim_nature", "code"),
    r("stg_entry", "partner", "dim_entity", "code"),
    r("stg_entry", "share", "dim_entity", "code"),
    // Jeux de règles
    rq("dim_ruleset_item", "ruleset_code", "dim_ruleset", "code"),
    rq("dim_ruleset_item", "rule_code", "dim_rule", "code"),
];

/// Version **possédée** d'une [`Reference`] — nécessaire pour les références
/// **dynamiques** (caractéristiques N1/N2) dont les noms de tables/colonnes ne
/// sont pas `'static` (ex. `car_<code>`, colonnes d'attributs).
#[derive(Clone, Debug)]
pub struct OwnedReference {
    pub table: String,
    pub column: String,
    pub target_table: String,
    pub target_column: String,
    pub required: bool,
}

impl OwnedReference {
    fn from_static(r: &Reference) -> Self {
        OwnedReference {
            table: r.table.to_string(),
            column: r.column.to_string(),
            target_table: r.target_table.to_string(),
            target_column: r.target_column.to_string(),
            required: r.required,
        }
    }
}

/// Master data (table, colonne clé) d'une dimension d'écriture, si elle en a une.
/// Dérivé du graphe statique : `account → (dim_account, code)`,
/// `currency → (dim_currency, code_iso)`, `nature → (dim_nature, code)`, etc.
/// `None` pour les dimensions sans master data (analysis, analysis2, custom).
pub fn dimension_master(dim: &str) -> Option<(&'static str, &'static str)> {
    entry_dimension_target(dim).map(|r| (r.target_table, r.target_column))
}

/// Résout la cible référentielle d'un nom, qu'il désigne une **dimension**
/// d'écriture (master data statique) ou une **liste de valeurs** (`lst_<code>`,
/// cf. [`crate::value_lists`]). Renvoie `(table, colonne_clé)` possédés (les noms
/// de listes ne sont pas `'static`). `None` si le nom n'est ni l'un ni l'autre.
///
/// C'est le point d'entrée commun pour valider/résoudre la cible d'un attribut N2
/// de caractéristique : un champ peut viser une dimension existante **ou** une
/// liste de valeurs réutilisable.
pub fn target_master(con: &Connection, target: &str) -> Option<(String, String)> {
    if let Some((t, c)) = dimension_master(target) {
        return Some((t.to_string(), c.to_string()));
    }
    if crate::value_lists::list_exists(con, target) {
        return Some((crate::value_lists::value_table(target), "code".to_string()));
    }
    None
}

/// `true` si les registres des caractéristiques existent (faux au tout premier
/// démarrage, avant exécution du DDL).
fn characteristic_registries_exist(con: &Connection) -> bool {
    con.query_row(
        "SELECT COUNT(*) = 2 FROM information_schema.tables \
         WHERE table_schema = 'main' \
           AND table_name IN ('dim_characteristic', 'dim_characteristic_attribute')",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Références **dynamiques** induites par les caractéristiques N1/N2 :
/// - `dim_<base>.<code> → car_<code>.code` (rattachement de la valeur N1) ;
/// - `car_<code>.<attr> → dim_<cible>.<clé>` (chaque attribut N2 typé).
///
/// Lues directement depuis les registres `dim_characteristic` /
/// `dim_characteristic_attribute` (tolère leur absence au premier démarrage).
/// Toutes nullables (`required = false`) : un membre peut ne pas être classé, un
/// attribut peut ne pas être renseigné.
pub fn dynamic_references(con: &Connection) -> Vec<OwnedReference> {
    if !characteristic_registries_exist(con) {
        return Vec::new();
    }
    let mut out = Vec::new();

    // N1 : colonne de rattachement sur la dimension de base → car_<code>.code
    if let Ok(mut stmt) =
        con.prepare("SELECT code, base_dimension FROM dim_characteristic ORDER BY code")
    {
        if let Ok(rows) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (code, base) in rows.flatten() {
                if let Some((base_table, _)) = dimension_master(&base) {
                    out.push(OwnedReference {
                        table: base_table.to_string(),
                        column: code.clone(),
                        target_table: format!("car_{code}"),
                        target_column: "code".to_string(),
                        required: false,
                    });
                }
            }
        }
    }

    // N2 : chaque attribut car_<char>.<name> → master data de la dimension cible
    if let Ok(mut stmt) = con.prepare(
        "SELECT characteristic_code, name, target_dimension \
         FROM dim_characteristic_attribute ORDER BY characteristic_code, name",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for (char_code, name, target) in rows.flatten() {
                // La cible d'un N2 peut être une dimension ou une liste de valeurs.
                if let Some((tt, tc)) = target_master(con, &target) {
                    out.push(OwnedReference {
                        table: format!("car_{char_code}"),
                        column: name,
                        target_table: tt,
                        target_column: tc,
                        required: false,
                    });
                }
            }
        }
    }

    // Références directes (patron B) : dim_<host>.<column> → master data cible.
    // Auto-références tolérées comme les statiques (ex. compte_parent → account).
    if custom_reference_registry_exists(con) {
        if let Ok(mut stmt) = con.prepare(
            "SELECT host_dimension, column_name, target_dimension \
             FROM dim_custom_reference ORDER BY host_dimension, column_name",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            }) {
                for (host, column, target) in rows.flatten() {
                    if let (Some((ht, _)), Some((tt, tc))) =
                        (dimension_master(&host), dimension_master(&target))
                    {
                        out.push(OwnedReference {
                            table: ht.to_string(),
                            column,
                            target_table: tt.to_string(),
                            target_column: tc.to_string(),
                            required: false,
                        });
                    }
                }
            }
        }
    }

    out
}

/// `true` si le registre des références directes existe (faux au premier
/// démarrage, avant exécution du DDL).
fn custom_reference_registry_exists(con: &Connection) -> bool {
    con.query_row(
        "SELECT COUNT(*) = 1 FROM information_schema.tables \
         WHERE table_schema = 'main' AND table_name = 'dim_custom_reference'",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Toutes les références : statiques (`REFERENCES`) + dynamiques (caractéristiques).
/// Source unique pour la validation à l'écriture, la santé des données et les
/// dropdowns dès lors que les caractéristiques entrent en jeu.
pub fn all_references(con: &Connection) -> Vec<OwnedReference> {
    let mut out: Vec<OwnedReference> = REFERENCES.iter().map(OwnedReference::from_static).collect();
    out.extend(dynamic_references(con));
    out
}

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
