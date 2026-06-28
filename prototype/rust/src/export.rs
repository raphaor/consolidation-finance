//! Export / import **complet** de l'état applicatif en un **paquet JSON unique**.
//!
//! - `GET  /api/export`     : sérialise toutes les tables persistantes en un seul
//!   objet JSON `{ table → [lignes] }` (+ tables dynamiques `car_<id>` / `lst_<id>`
//!   préfixées `_car:` / `_lst:`, et `_meta`).
//! - `POST /api/import/all` : restaure l'état depuis un tel paquet — **remplacement
//!   total** (purge des survivantes + DROP + CREATE du schéma, recréation des
//!   colonnes dynamiques, puis réinsertion de toutes les lignes). Ne relance **pas**
//!   le pipeline (comme `/api/reset`) : l'utilisateur clique « Lancer le
//!   pipeline » ensuite.
//!
//! `fact_entry` est volontairement exclue : c'est une table **dérivée**,
//! reconstruite par le pipeline depuis `stg_entry`.
//!
//! Couverture (format `conso-export-v3`) : 30 tables + tables dynamiques. Inclut
//! les **règles**, **dimensions custom**, **caractéristiques N1/N2**, **listes de
//! valeurs**, **références directes**, **contrôles**, **postes (aggregates)**,
//! **indicateurs** — capturer l'état applicatif complet pour sauvegarde /
//! restauration / seed initial (T3, `CONSO_SEED_JSON`).

use axum::{
    extract::{Query, State},
    routing::{get, post},
    Json, Router,
};
use duckdb::{params_from_iter, types::Value as DbValue, Connection};
use serde_json::{Map, Value as JsonValue};
use std::collections::HashSet;
use std::sync::Arc;

use crate::create_schema;
use crate::characteristics;
use crate::custom_references;
use crate::dimensions;
use crate::masterdata::{json_to_db_value, run_query};
use crate::references;
use crate::resolve;
use crate::state::{db_err, lock_con, AppError, AppState};

/// Tables persistantes, dans l'ordre d'insertion (dépendances amont d'abord).
///
/// Les **registres dynamiques** (`dim_custom_dimension`, `dim_custom_reference`,
/// `dim_characteristic`, `dim_characteristic_attribute`, `dim_value_list`)
/// précèdent les master data : leurs colonnes physiques (`x{id}`, `r{id}`,
/// colonne N1) sont ré-appliquées sur les master data juste avant leur
/// insertion (cf. [`apply_dynamic_physical_columns`]).
///
/// `fact_entry` est exclue (dérivée, reconstruite par le pipeline).
pub const TABLES: &[&str] = &[
    // 1. Config + scénario + période + devise
    "app_config",
    "dim_scenario_category",
    "dim_variant",
    "dim_rate_set",
    "dim_perimeter_set",
    "dim_period",
    "dim_currency",
    // 2. Règles (avant dim_consolidation : ruleset_code est FK vers dim_ruleset.id)
    "dim_rule",
    "dim_ruleset",
    "dim_ruleset_item",
    // 3. Registres dynamiques (avant master data : colonnes physiques à ré-appliquer)
    "dim_custom_dimension",
    "dim_custom_reference",
    "dim_characteristic",
    "dim_characteristic_attribute",
    "dim_value_list",
    // 4. Master data
    "dim_consolidation",
    "dim_entity",
    "dim_sous_classe",
    "dim_flow",
    "dim_flow_scheme",
    "dim_account",
    "dim_nature",
    "dim_method",
    // 5. Satellites
    "sat_perimeter",
    "sat_exchange_rate",
    "sat_flow_scheme_item",
    // 6. Staging (après dim_custom_dimension : porte les colonnes x{id})
    "stg_entry",
    // 7. Contrôles + postes + indicateurs (registres autonomes sans FK master)
    "dim_control",
    "dim_control_set",
    "dim_control_set_item",
    "dim_aggregate",
    "dim_indicator",
];

/// Format du paquet (cf. `_meta.format`).
pub const FORMAT: &str = "conso-export-v3";

/// GET /api/export — paquet JSON complet de l'état.
async fn export_all(State(state): State<Arc<AppState>>) -> Result<Json<JsonValue>, AppError> {
    let bundle = {
        let con = lock_con(&state)?;
        let mut obj = Map::new();

        // `SELECT *` par table : capture aussi les colonnes dynamiques
        // (`x{id}` sur stg_entry, `r{id}` et N1 sur les master data, `id` sur
        // les registres survivants).
        for t in TABLES {
            let rows = run_query(&con, &format!("SELECT * FROM {t}"), Vec::new())?;
            obj.insert((*t).to_string(), JsonValue::Array(rows));
        }

        // Coefficients : seuls les **utilisateur** sont exportés (les natifs sont
        // re-seedés par `create_schema` → éviter le doublon de PK).
        obj.insert(
            "dim_coefficient".to_string(),
            JsonValue::Array(run_query(
                &con,
                "SELECT code, libelle, expression, kind \
                 FROM dim_coefficient WHERE kind = 'user' ORDER BY code",
                Vec::new(),
            )?),
        );

        // Tables de valeurs dynamiques `car_<id>` et `lst_<id>` : une clé par
        // table, préfixée pour la distinguer des tables statiques.
        export_dynamic_tables(&con, &mut obj)?;

        let mut meta = Map::new();
        meta.insert("format".to_string(), JsonValue::String(FORMAT.to_string()));
        obj.insert("_meta".to_string(), JsonValue::Object(meta));

        JsonValue::Object(obj)
    };
    Ok(Json(bundle))
}

/// Tables exposées dans le preview (ordre d'affichage).
const PREVIEW_TABLES: &[&str] = &[
    "dim_custom_dimension",
    "app_config",
    "dim_scenario_category",
    "dim_variant",
    "dim_rate_set",
    "dim_perimeter_set",
    "dim_period",
    "dim_currency",
    "dim_rule",
    "dim_ruleset",
    "dim_ruleset_item",
    "dim_custom_reference",
    "dim_characteristic",
    "dim_characteristic_attribute",
    "dim_value_list",
    "dim_consolidation",
    "dim_entity",
    "dim_sous_classe",
    "dim_account",
    "dim_flow",
    "dim_flow_scheme",
    "dim_nature",
    "dim_method",
    "sat_perimeter",
    "sat_exchange_rate",
    "sat_flow_scheme_item",
    "stg_entry",
    "dim_control",
    "dim_control_set",
    "dim_control_set_item",
    "dim_aggregate",
    "dim_indicator",
    "dim_coefficient",
];

/// Libellés lisibles pour l'UI de sélection.
fn table_label(t: &str) -> &str {
    match t {
        "dim_custom_dimension" => "Dimensions custom",
        "app_config" => "Configuration",
        "dim_scenario_category" => "Catégories de scénario",
        "dim_variant" => "Variantes",
        "dim_rate_set" => "Jeux de taux",
        "dim_perimeter_set" => "Jeux de périmètre",
        "dim_period" => "Périodes",
        "dim_currency" => "Devises",
        "dim_rule" => "Règles",
        "dim_ruleset" => "Jeux de règles",
        "dim_ruleset_item" => "Items de jeux de règles",
        "dim_custom_reference" => "Références directes",
        "dim_characteristic" => "Caractéristiques (N1)",
        "dim_characteristic_attribute" => "Attributs (N2)",
        "dim_value_list" => "Listes de valeurs",
        "dim_consolidation" => "Consolidations",
        "dim_entity" => "Entités",
        "dim_sous_classe" => "Sous-classes",
        "dim_account" => "Plan de comptes",
        "dim_flow" => "Flux",
        "dim_flow_scheme" => "Schémas de flux",
        "dim_nature" => "Natures",
        "dim_method" => "Méthodes",
        "sat_perimeter" => "Périmètre",
        "sat_exchange_rate" => "Taux de change",
        "sat_flow_scheme_item" => "Items de schéma de flux",
        "stg_entry" => "Écritures (staging)",
        "dim_control" => "Contrôles",
        "dim_control_set" => "Jeux de contrôles",
        "dim_control_set_item" => "Items de jeux de contrôles",
        "dim_aggregate" => "Postes",
        "dim_indicator" => "Indicateurs",
        "dim_coefficient" => "Coefficients",
        other => other,
    }
}

/// POST /api/import/preview — analyse un paquet et retourne la liste des tables
/// avec leur nombre de lignes. L'UI utilise cette info pour afficher la sélection.
async fn import_preview(
    Json(bundle): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let obj = bundle
        .as_object()
        .ok_or_else(|| AppError::bad_request("le paquet doit être un objet JSON"))?;

    let meta = obj.get("_meta").cloned().unwrap_or(JsonValue::Object(Map::new()));

    let mut tables = Vec::new();
    for &t in PREVIEW_TABLES {
        let rows = match obj.get(t) {
            Some(JsonValue::Array(a)) => a.len(),
            _ => 0,
        };
        tables.push(serde_json::json!({
            "name": t,
            "label": table_label(t),
            "rows": rows,
        }));
    }

    // Tables dynamiques `_car:<id>` / `_lst:<id>`.
    let mut dyn_tables: Vec<&String> = obj
        .keys()
        .filter(|k| k.starts_with("_car:") || k.starts_with("_lst:"))
        .collect();
    dyn_tables.sort();
    for k in dyn_tables {
        let rows = obj.get(k).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let kind = if k.starts_with("_car:") { "Valeurs N1" } else { "Liste de valeurs" };
        tables.push(serde_json::json!({
            "name": k,
            "label": format!("{kind} ({})", &k[5..]),
            "rows": rows,
        }));
    }

    Ok(Json(serde_json::json!({
        "meta": meta,
        "tables": tables,
    })))
}

/// Paramètres de requête pour `import_all`.
#[derive(serde::Deserialize, Default)]
struct ImportParams {
    /// Tables à exclure (séparées par des virgules).
    exclude: Option<String>,
}

/// POST /api/import/all — restaure l'état depuis un paquet (remplacement total).
///
/// Query param `exclude` : liste de tables séparées par des virgules à ne pas
/// importer (ex. `?exclude=stg_entry,dim_consolidation`).
async fn import_all(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ImportParams>,
    Json(bundle): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let excluded: HashSet<&str> = params
        .exclude
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .collect();

    let counts = {
        let con = lock_con(&state)?;
        import_bundle(&con, &bundle, &excluded)?
    };

    Ok(Json(
        serde_json::json!({ "status": "ok", "imported": counts }),
    ))
}

/// Restaure l'état depuis un paquet — cœur métier d'`import_all`, isolé pour
/// être réutilisé au boot serveur (`CONSO_SEED_JSON`, T3) hors contexte HTTP.
///
/// Étapes :
/// 1. Purge les tables dynamiques `car_*` / `lst_*` et vide les registres
///    survivants (pour repartir propre — `create_schema` ne les vide pas).
/// 2. `create_schema` : DROP + CREATE complet, seed natifs (custom_reference,
///    coefficient builtin).
/// 3. Vide les natifs re-seedés qu'on va réinjecter depuis le paquet avec id
///    préservé (cohérence des colonnes physiques `r{id}`).
/// 4. Insère les registres dynamiques (`dim_custom_dimension`,
///    `dim_custom_reference`, `dim_characteristic*`, `dim_value_list`) avec ids
///    préservés — **avant** les master data.
/// 5. Ré-applique les colonnes physiques dynamiques (`x{id}`, `r{id}`, N1,
///    `c<attr_id>` sur `car_<id>`).
/// 6. Insère le reste dans l'ordre de [`TABLES`] (master data + satellites +
///    staging + contrôles + postes + indicateurs).
/// 7. Coefficients utilisateur.
/// 8. Valeurs des tables dynamiques `car_<id>` / `lst_<id>`.
/// 9. Recalage des séquences (ids explicites à l'insert).
/// 10. CHECKPOINT.
pub fn import_bundle(
    con: &Connection,
    bundle: &JsonValue,
    excluded: &HashSet<&str>,
) -> Result<Map<String, JsonValue>, AppError> {
    let obj = bundle
        .as_object()
        .ok_or_else(|| AppError::bad_request("le paquet doit être un objet JSON"))?;

    // 1. Purge préalable : tables dynamiques + registres survivants.
    purge_dynamic_tables(con).map_err(db_err)?;
    purge_survivor_registries(con).map_err(db_err)?;

    // 2. Table rase : DROP + CREATE de tout le schéma (re-seed natifs).
    create_schema(con).map_err(db_err)?;

    // 3. Vider les natifs que create_schema vient de peupler : on va les
    //    réinjecter depuis le paquet avec ids préservés (cohérence r{id}).
    con.execute("DELETE FROM dim_custom_reference", []).map_err(db_err)?;

    // 4 + 5. Insertion des registres dynamiques (ids préservés) + side-effects
    //        physiques. On insère dim_custom_dimension, dim_custom_reference,
    //        dim_characteristic, dim_characteristic_attribute, dim_value_list
    //        en premier (avant le reste de TABLES), puis on ré-applique les
    //        colonnes physiques.
    let mut counts = Map::new();
    let dynamic_registries = [
        "dim_custom_dimension",
        "dim_custom_reference",
        "dim_characteristic",
        "dim_characteristic_attribute",
        "dim_value_list",
    ];
    for t in dynamic_registries {
        if excluded.contains(t) {
            counts.insert(t.to_string(), JsonValue::Number(0.into()));
            continue;
        }
        let n = insert_table(con, t, obj.get(t))?;
        counts.insert(t.to_string(), JsonValue::Number(n.into()));
    }
    apply_dynamic_physical_columns(con).map_err(db_err)?;

    // 6. Insérer le reste des tables dans l'ordre (en skipant les registres
    //    dynamiques déjà insérés à l'étape 4).
    for t in TABLES {
        if dynamic_registries.contains(t) {
            continue; // déjà inséré + side-effects appliqués
        }
        if excluded.contains(t) {
            counts.insert((*t).to_string(), JsonValue::Number(0.into()));
            continue;
        }
        let n = insert_table(con, t, obj.get(*t))?;
        counts.insert((*t).to_string(), JsonValue::Number(n.into()));
    }

    // 7. Coefficients utilisateur (les natifs ont été re-seedés par create_schema
    //    ; le paquet ne contient que les `kind = 'user'`). On vide les user
    //    avant réinsertion pour éviter les doublons.
    if excluded.contains("dim_coefficient") {
        counts.insert("dim_coefficient".to_string(), JsonValue::Number(0.into()));
    } else {
        con.execute("DELETE FROM dim_coefficient WHERE kind = 'user'", [])
            .map_err(db_err)?;
        let n_coef = insert_table(con, "dim_coefficient", obj.get("dim_coefficient"))?;
        counts.insert("dim_coefficient".to_string(), JsonValue::Number(n_coef.into()));
    }

    // 8. Valeurs des tables dynamiques car_<id> / lst_<id>.
    let mut dyn_keys: Vec<&String> = obj
        .keys()
        .filter(|k| k.starts_with("_car:") || k.starts_with("_lst:"))
        .collect();
    dyn_keys.sort();
    for k in dyn_keys {
        if excluded.contains(k.as_str()) {
            counts.insert(k.clone(), JsonValue::Number(0.into()));
            continue;
        }
        let id = &k[5..]; // Skip "_car:" / "_lst:" (5 chars)
        let table = if k.starts_with("_car:") {
            format!("car_{id}")
        } else {
            format!("lst_{id}")
        };
        let n = match obj.get(k) {
            Some(JsonValue::Array(rows)) if !rows.is_empty() => {
                insert_dynamic_value_table(con, &table, rows)?
            }
            _ => 0,
        };
        counts.insert(k.clone(), JsonValue::Number(n.into()));
    }

    // 9. Recaler les séquences sur le MAX(id) des tables à PK auto-incrémentée.
    //    Un paquet exporté contient des ids explicites ; sans recalage, les
    //    inserts ultérieurs (DEFAULT nextval) entreraient en conflit de PK.
    for &(seq, table) in SEQUENCES_TO_RESCALE {
        let _ = con.execute(
            &format!("SELECT setval('{seq}', COALESCE((SELECT MAX(id) FROM {table}), 0))"),
            [],
        );
    }

    // Flushe tout le DDL + données importées dans le fichier .duckdb (WAL
    // propre) : évite une base illisible si le serveur est tué ensuite.
    let _ = con.execute("CHECKPOINT", []);

    Ok(counts)
}

/// Tables à PK auto-incrémentée dont la séquence doit être recalée après import.
const SEQUENCES_TO_RESCALE: &[(&str, &str)] = &[
    ("seq_consolidation", "dim_consolidation"),
    ("seq_stg_entry", "stg_entry"),
    ("seq_dim_custom_reference", "dim_custom_reference"),
];

/// DROP toutes les tables dynamiques `car_*` et `lst_*` (avant `create_schema`).
/// Ces tables survivent au reset (hors `ALL_DROP`), il faut donc les dropper
/// explicitement pour que les tables orphelines (caractéristiques supprimées
/// dans le paquet) ne polluent pas la base.
fn purge_dynamic_tables(con: &Connection) -> duckdb::Result<()> {
    let names: Vec<String> = con.prepare(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'main' \
           AND (table_name LIKE 'car\\_%' ESCAPE '\\' \
                OR table_name LIKE 'lst\\_%' ESCAPE '\\')",
    )?
    .query_map([], |r| r.get::<_, String>(0))?
    .filter_map(Result::ok)
    .collect();
    for n in names {
        // Les noms viennent d'information_schema (sans interpolation utilisateur).
        let _ = con.execute(&format!("DROP TABLE IF EXISTS {n}"), []);
    }
    Ok(())
}

/// Vide les registres survivants user-editables avant `create_schema`.
///
/// Sans cela, `create_schema` (+ son `reapply`) ré-appliquerait des colonnes
/// physiques à partir des anciennes données, et l'insertion du paquet
/// collisionnerait (PK `(host, column)` etc.).
fn purge_survivor_registries(con: &Connection) -> duckdb::Result<()> {
    for t in [
        "dim_custom_dimension",
        "dim_characteristic",
        "dim_characteristic_attribute",
        "dim_value_list",
        "dim_custom_reference",
        "dim_control",
        "dim_control_set",
        "dim_control_set_item",
        "dim_aggregate",
        "dim_indicator",
    ] {
        // Ces tables sont en CREATE IF NOT EXISTS : elles existent après un
        // premier run ; sur base fraîche, le DELETE est juste un no-op silencieux.
        let _ = con.execute(&format!("DELETE FROM {t}"), []);
    }
    // dim_coefficient user : les natifs sont gérés par create_schema.
    let _ = con.execute("DELETE FROM dim_coefficient WHERE kind = 'user'", []);
    Ok(())
}

/// Ré-applique toutes les colonnes physiques issues des registres dynamiques,
/// après leur insertion. À appeler entre l'insertion des registres et celle des
/// master data.
fn apply_dynamic_physical_columns(con: &Connection) -> duckdb::Result<()> {
    // 0. Recréer les tables de valeurs `car_<id>` et `lst_<id>` à partir des
    //    registres (elles ont été droppées par purge_dynamic_tables, et
    //    create_schema ne les recréé pas — seul characteristics::create le fait
    //    via l'API, qu'on n'appelle pas ici pour préserver les ids).
    create_dynamic_value_tables(con)?;

    // 1. Colonnes `x{id}` des dimensions custom sur stg_entry / fact_entry.
    let customs = dimensions::load_customs(con).unwrap_or_default();
    dimensions::apply_custom_columns(con, &customs)?;

    // 2. Colonnes `<code>` N1 sur les master data (rattachement caractéristique).
    characteristics::reapply(con)?;

    // 3. Colonnes `r{id}` des références directes sur les master data.
    custom_references::reapply(con)?;

    // 4. Colonnes `c<attr_id>` des attributs N2 dans `car_<id>`.
    apply_attribute_columns(con)?;

    Ok(())
}

/// Recréé les tables de valeurs `car_<id>` (depuis `dim_characteristic`) et
/// `lst_<id>` (depuis `dim_value_list`). Tables survie-vide après
/// [`purge_dynamic_tables`] ; sans cela l'insert des valeurs N1/listes échoue.
fn create_dynamic_value_tables(con: &Connection) -> duckdb::Result<()> {
    // car_<id> : une table par caractéristique N1.
    let mut char_rows = con.prepare("SELECT id FROM dim_characteristic ORDER BY id")?;
    let char_ids: Vec<i64> = char_rows
        .query_map([], |r| r.get::<_, i64>(0))?
        .filter_map(Result::ok)
        .collect();
    drop(char_rows);
    for id in char_ids {
        // CREATE IF NOT EXISTS : no-op si la table existe déjà.
        let _ = con.execute(
            &format!("CREATE TABLE IF NOT EXISTS car_{id} (code TEXT PRIMARY KEY, libelle TEXT)"),
            [],
        );
    }

    // lst_<id> : une table par liste de valeurs.
    let mut list_rows = con.prepare("SELECT id FROM dim_value_list ORDER BY id")?;
    let list_ids: Vec<i64> = list_rows
        .query_map([], |r| r.get::<_, i64>(0))?
        .filter_map(Result::ok)
        .collect();
    drop(list_rows);
    for id in list_ids {
        let _ = con.execute(
            &format!("CREATE TABLE IF NOT EXISTS lst_{id} (code TEXT PRIMARY KEY, libelle TEXT)"),
            [],
        );
    }
    Ok(())
}

/// Crée les colonnes physiques `c<attr_id>` dans `car_<char_id>` pour chaque
/// attribut N2 déclaré dans `dim_characteristic_attribute`. Sans cela, l'insert
/// des valeurs N1 (qui portent les `c<attr_id>`) échouerait.
fn apply_attribute_columns(con: &Connection) -> duckdb::Result<()> {
    // On ne touche qu'aux tables car_<id> existantes ; le préfixe `car_` est
    // fixe (jamais d'interpolation utilisateur non contrôlée).
    let rows: Vec<(i64, i64)> = con.prepare(
        "SELECT a.id, c.id \
         FROM dim_characteristic_attribute a \
         JOIN dim_characteristic c ON a.characteristic_code = c.code \
         ORDER BY c.id, a.id",
    )?
    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
    .filter_map(Result::ok)
    .collect();
    for (attr_id, char_id) in rows {
        // ALTER ADD COLUMN silencieux si la colonne existe déjà (DuckDB Ignore).
        let _ = con.execute(
            &format!("ALTER TABLE car_{char_id} ADD COLUMN \"c{attr_id}\" TEXT"),
            [],
        );
    }
    Ok(())
}

/// Exporte les tables dynamiques `car_<id>` et `lst_<id>` sous des clés
/// `_car:<id>` / `_lst:<id>` dans le paquet. Itère sur les registres pour ne
/// lister que les tables existantes.
fn export_dynamic_tables(
    con: &Connection,
    obj: &mut Map<String, JsonValue>,
) -> Result<(), AppError> {
    // car_<id> : une table par caractéristique N1.
    let char_rows = run_query(
        con,
        "SELECT id FROM dim_characteristic ORDER BY id",
        Vec::new(),
    )?;
    let char_ids: Vec<i64> = char_rows
        .iter()
        .filter_map(|r| r.get("id").and_then(JsonValue::as_i64))
        .collect();
    for id in char_ids {
        let table = format!("car_{id}");
        let rows = run_query(con, &format!("SELECT * FROM {table}"), Vec::new())?;
        obj.insert(format!("_car:{id}"), JsonValue::Array(rows));
    }

    // lst_<id> : une table par liste de valeurs.
    let list_rows = run_query(
        con,
        "SELECT id FROM dim_value_list ORDER BY id",
        Vec::new(),
    )?;
    let list_ids: Vec<i64> = list_rows
        .iter()
        .filter_map(|r| r.get("id").and_then(JsonValue::as_i64))
        .collect();
    for id in list_ids {
        let table = format!("lst_{id}");
        let rows = run_query(con, &format!("SELECT * FROM {table}"), Vec::new())?;
        obj.insert(format!("_lst:{id}"), JsonValue::Array(rows));
    }
    Ok(())
}

/// Insère les lignes d'une table à partir de leur tableau JSON.
///
/// Chaque ligne est un objet `{ colonne → valeur }`. On insère colonne par
/// colonne (clés de l'objet) : robuste aux colonnes custom et à l'ordre. Les
/// types sont laissés à DuckDB (cast implicite à l'INSERT : texte→DATE,
/// double→DECIMAL, etc.), comme pour l'import CSV.
fn insert_table(
    con: &Connection,
    table: &str,
    data: Option<&JsonValue>,
) -> Result<usize, AppError> {
    let rows = match data {
        Some(JsonValue::Array(a)) => a,
        _ => return Ok(0),
    };
    insert_rows(con, table, rows)
}

/// Implémentation partagée par [`insert_table`] et [`insert_dynamic_value_table`].
fn insert_rows(
    con: &Connection,
    table: &str,
    rows: &[JsonValue],
) -> Result<usize, AppError> {
    let mut n = 0usize;
    for row in rows {
        let robj = row
            .as_object()
            .ok_or_else(|| AppError::bad_request(format!("{table} : ligne non-objet")))?;
        if robj.is_empty() {
            continue;
        }
        let cols: Vec<String> = robj.keys().map(|k| format!("\"{k}\"")).collect();
        let placeholders = vec!["?"; cols.len()].join(", ");
        // Traduction code→id des FK migrées en clé technique (option A, chantier
        // B1) : un paquet exporté **avant** la refonte porte ces FK en codes ;
        // on les résout vers l'id de la cible (déjà insérée — l'ordre de TABLES
        // place les dimensions amont avant). Les autres colonnes : conversion
        // directe.
        let vals: Vec<DbValue> = robj
            .iter()
            .map(|(k, v)| import_db_value(con, table, k, v))
            .collect::<Result<_, _>>()?;
        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({placeholders})",
            cols.join(", ")
        );
        con.execute(&sql, params_from_iter(vals))
            .map_err(|e| AppError::bad_request(format!("{table} : insertion impossible — {e}")))?;
        n += 1;
    }
    Ok(n)
}

/// Insertion d'une table de valeurs dynamique `car_<id>` / `lst_<id>`. Les
/// colonnes N2 (`c<attr_id>`) sont déjà créées par [`apply_attribute_columns`].
fn insert_dynamic_value_table(
    con: &Connection,
    table: &str,
    rows: &[JsonValue],
) -> Result<usize, AppError> {
    insert_rows(con, table, rows)
}

/// Valeur à insérer pour `(table, col)` à l'import : pour une FK migrée en clé
/// technique (contrat code, cf. [`references::Reference::target_display_column`]),
/// résout le **code** du paquet vers l'`id` de la cible ; sinon, conversion JSON→DB
/// directe. Vide/non-texte sur une telle FK ⇒ `NULL`.
fn import_db_value(
    con: &Connection,
    table: &str,
    col: &str,
    v: &JsonValue,
) -> Result<DbValue, AppError> {
    if let Some(r) = references::REFERENCES.iter().find(|r| {
        r.table == table && r.column == col && r.target_display_column.is_some()
    }) {
        // Tolère un paquet **déjà** en id (réimport d'un export récent) : un nombre
        // est inséré tel quel ; seul un code (texte) est résolu.
        return match v {
            JsonValue::String(s) if !s.is_empty() => {
                let id = resolve::resolve_id(con, r.target_table, s)
                    .map_err(db_err)?
                    .ok_or_else(|| {
                        AppError::bad_request(format!(
                            "{table}.{col} : code '{s}' absent de {}",
                            r.target_table
                        ))
                    })?;
                Ok(DbValue::BigInt(id))
            }
            JsonValue::Null | JsonValue::String(_) => Ok(DbValue::Null),
            other => Ok(json_to_db_value(other)),
        };
    }
    Ok(json_to_db_value(v))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/export", get(export_all))
        .route("/api/import/preview", post(import_preview))
        .route("/api/import/all", post(import_all))
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;
    use serde_json::json;

    /// Un paquet d'**avant la refonte** (FK consolidation en codes) se restaure
    /// dans le schéma actuel (FK en id) : `import_db_value` résout code→id.
    #[test]
    fn import_resout_les_fk_code_vers_id() {
        let con = Connection::open_in_memory().unwrap();
        create_schema(&con).unwrap();
        // Dimensions cibles (codes) — insérées avant la consolidation.
        con.execute_batch(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES ('REEL','Réel');
             INSERT INTO dim_variant (code, libelle) VALUES ('BASE','Base');
             INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PS','P');
             INSERT INTO dim_rate_set (code, libelle) VALUES ('RT','R');
             INSERT INTO dim_period (code, libelle) VALUES ('2024','Exercice 2024');
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);",
        )
        .unwrap();

        // Ligne dim_consolidation telle qu'un vieux paquet la porte (FK en codes).
        let row = json!({
            "id": 1, "libelle": "Réel", "phase": "REEL", "exercice": "2024",
            "perimeter_set": "PS", "variant": "BASE", "presentation_currency": "EUR",
            "perimeter_period": "2024", "rate_set": "RT", "rate_period": "2024",
            "ruleset_code": null, "a_nouveau_consolidation_id": null, "statut": "ouvert"
        });
        let n = insert_table(&con, "dim_consolidation", Some(&json!([row]))).unwrap();
        assert_eq!(n, 1);

        // Toutes les FK stockées en id ; relues via les cibles.
        let (phase_ok, variant_ok, exercice_ok, pres_ok): (bool, bool, bool, bool) = con
            .query_row(
                "SELECT
                   phase    = (SELECT id FROM dim_scenario_category WHERE code='REEL'),
                   variant  = (SELECT id FROM dim_variant WHERE code='BASE'),
                   exercice = (SELECT id FROM dim_period WHERE code='2024'),
                   presentation_currency = (SELECT id FROM dim_currency WHERE code_iso='EUR')
                 FROM dim_consolidation WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert!(
            phase_ok && variant_ok && exercice_ok && pres_ok,
            "FK résolues en id à l'import"
        );

        // Code inexistant : rejeté proprement.
        let bad = json!({ "id": 2, "variant": "NOPE" });
        assert!(insert_table(&con, "dim_consolidation", Some(&json!([bad]))).is_err());
    }

    /// Round-trip export → import → export préserve toutes les données,
    /// y compris les tables dynamiques (caractéristiques, listes, références
    /// directes, contrôles, indicateurs).
    #[test]
    fn round_trip_preserve_toutes_les_tables_dynamiques() {
        let con = Connection::open_in_memory().unwrap();
        create_schema(&con).unwrap();

        // Dimensions de base minimales pour permettre l'insertion de master data.
        con.execute_batch(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES ('REEL','Réel');
             INSERT INTO dim_variant (code, libelle) VALUES ('BASE','Base');
             INSERT INTO dim_period (code, libelle) VALUES ('2024','2024');
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);
             INSERT INTO dim_account (code, libelle, sous_classe) VALUES ('101','Cap','1'),
                                                                     ('102','Cap2','1');
             INSERT INTO dim_sous_classe (code, libelle) VALUES ('1','Bilan');",
        )
        .unwrap();

        // Caractéristique N1 sur account + attribut N2 + valeur.
        characteristics::create_characteristic(&con, "comportement", "Comportement", "account")
            .unwrap();
        let char_id: i64 = con
            .query_row(
                "SELECT id FROM dim_characteristic WHERE code = 'comportement'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Attribut N2 cible = account (déjà en master data).
        con.execute(
            "INSERT INTO dim_characteristic_attribute \
             (characteristic_code, name, libelle, target_dimension) \
             VALUES ('comportement', 'compte_destination', 'Dest', 'account')",
            [],
        )
        .unwrap();
        //ensure_characteristic_attribute_ids peut ne pas avoir tourné ; on
        // attribue l'id manuellement si la colonne id est NULL.
        let attr_id: i64 = {
            // Forcer un id si NULL via UPDATE (la séquence n'est pas nécessaire
            // ici, on assigne explicitement).
            let id: Option<i64> = con
                .query_row(
                    "SELECT id FROM dim_characteristic_attribute \
                     WHERE characteristic_code = 'comportement' AND name = 'compte_destination'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            match id {
                Some(i) => i,
                None => {
                    con.execute(
                        "UPDATE dim_characteristic_attribute SET id = 1 \
                         WHERE characteristic_code = 'comportement' \
                           AND name = 'compte_destination'",
                        [],
                    )
                    .unwrap();
                    1
                }
            }
        };
        // Créer la colonne c<attr_id> dans car_<char_id>.
        con.execute(
            &format!("ALTER TABLE car_{char_id} ADD COLUMN \"c{attr_id}\" TEXT"),
            [],
        )
        .unwrap();
        con.execute(
            &format!(
                "INSERT INTO car_{char_id} (code, libelle, \"c{attr_id}\") \
                 VALUES ('actif', 'Actif', '101')"
            ),
            [],
        )
        .unwrap();

        // Référence directe user (r{id}) sur dim_account.
        custom_references::create(&con, "account", "compte_parent", "account").unwrap();
        // Valoriser la colonne r{id} sur quelques comptes.
        let ref_col = custom_references::col_of_ref(&con, "account", "compte_parent").unwrap();
        con.execute_batch(&format!(
            "UPDATE dim_account SET \"{ref_col}\" = '101' WHERE code = '102';"
        ))
        .unwrap();

        // Liste de valeurs user.
        crate::value_lists::create_list(&con, "secteurs", "Secteurs").unwrap();
        let list_id: i64 = con
            .query_row(
                "SELECT id FROM dim_value_list WHERE code = 'secteurs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        con.execute(
            &format!("INSERT INTO lst_{list_id} (code, libelle) VALUES ('A','Agriculture')"),
            [],
        )
        .unwrap();

        // Contrôle + jeu + item.
        con.execute(
            "INSERT INTO dim_control (code, libelle, definition) \
             VALUES ('CTRL1', 'Bilan équilibré', '{}')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_control_set (code, libelle) VALUES ('JEU1', 'Jeu principal')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_control_set_item (set_code, control_code, ord) \
             VALUES ('JEU1', 'CTRL1', 1)",
            [],
        )
        .unwrap();

        // Poste + indicateur.
        con.execute(
            "INSERT INTO dim_aggregate (code, libelle, level, definition) \
             VALUES ('TOTAL_ACTIF', 'Total actif', 'consolidated', '{}')",
            [],
        )
        .unwrap();
        con.execute(
            "INSERT INTO dim_indicator (code, libelle, expression, grain, format) \
             VALUES ('EBITDA', 'EBITDA', 'TOTAL_ACTIF', '{}', 'montant')",
            [],
        )
        .unwrap();

        // ── 1er export ───────────────────────────────────────────────────
        let mut bundle1 = Map::new();
        for t in TABLES {
            let rows = run_query(&con, &format!("SELECT * FROM {t}"), Vec::new()).unwrap();
            bundle1.insert((*t).to_string(), JsonValue::Array(rows));
        }
        bundle1.insert(
            "dim_coefficient".to_string(),
            JsonValue::Array(run_query(
                &con,
                "SELECT code, libelle, expression, kind FROM dim_coefficient WHERE kind='user'",
                Vec::new(),
            )
            .unwrap()),
        );
        export_dynamic_tables(&con, &mut bundle1).unwrap();
        let bundle1 = JsonValue::Object(bundle1);

        // ── Import du paquet dans une base fraîche ───────────────────────
        let con2 = Connection::open_in_memory().unwrap();
        let excluded = HashSet::new();
        import_bundle(&con2, &bundle1, &excluded).unwrap();

        // ── 2e export pour comparer ─────────────────────────────────────
        let mut bundle2 = Map::new();
        for t in TABLES {
            let rows = run_query(&con2, &format!("SELECT * FROM {t}"), Vec::new()).unwrap();
            bundle2.insert((*t).to_string(), JsonValue::Array(rows));
        }
        bundle2.insert(
            "dim_coefficient".to_string(),
            JsonValue::Array(run_query(
                &con2,
                "SELECT code, libelle, expression, kind FROM dim_coefficient WHERE kind='user'",
                Vec::new(),
            )
            .unwrap()),
        );
        export_dynamic_tables(&con2, &mut bundle2).unwrap();
        let bundle2 = JsonValue::Object(bundle2);

        // Vérifications ciblées (comparaison brute difficile : ordres, ids
        // auto-attribués par seed_native peuvent différer). On contrôle les
        // tables métier critiques.
        let get = |b: &JsonValue, k: &str| -> usize {
            b.get(k).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
        };

        // Caractéristiques + attributs + valeurs préservés.
        assert_eq!(get(&bundle2, "dim_characteristic"), 1, "caractéristique");
        assert_eq!(
            get(&bundle2, "dim_characteristic_attribute"),
            1,
            "attribut N2"
        );
        assert_eq!(get(&bundle2, "dim_value_list"), 1, "liste de valeurs");
        // dim_custom_reference : user + natifs (natifs re-seedés par
        // create_schema puis conservés). On vérifie que la référence user est
        // bien là.
        let refs_user: usize = con2
            .query_row(
                "SELECT COUNT(*) FROM dim_custom_reference \
                 WHERE column_name = 'compte_parent' \
                   AND COALESCE(native, FALSE) = FALSE",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(refs_user, 1, "1 référence user conservée après round-trip");

        // Valeurs N1 (car_<char_id>) présentes dans le paquet via _car:<id>.
        let car_keys: Vec<&String> = bundle2
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("_car:"))
            .collect();
        assert_eq!(car_keys.len(), 1, "1 table car_<id>");
        let car_rows = bundle2.get(car_keys[0]).unwrap().as_array().unwrap();
        assert_eq!(car_rows.len(), 1, "1 valeur N1");
        // L'attribut N2 doit être peuplé.
        let v = car_rows[0].as_object().unwrap();
        let dest = v.values().find(|vv| vv.as_str() == Some("101"));
        assert!(dest.is_some(), "attribut N2 'compte_destination' = 101");

        // Valeurs lst_<list_id>.
        let lst_keys: Vec<&String> = bundle2
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with("_lst:"))
            .collect();
        assert_eq!(lst_keys.len(), 1, "1 table lst_<id>");
        let lst_rows = bundle2.get(lst_keys[0]).unwrap().as_array().unwrap();
        assert_eq!(lst_rows.len(), 1, "1 valeur de liste");

        // Contrôles + postes + indicateurs.
        assert_eq!(get(&bundle2, "dim_control"), 1);
        assert_eq!(get(&bundle2, "dim_control_set"), 1);
        assert_eq!(get(&bundle2, "dim_control_set_item"), 1);
        assert_eq!(get(&bundle2, "dim_aggregate"), 1);
        assert_eq!(get(&bundle2, "dim_indicator"), 1);

        // Master data préservée.
        assert_eq!(get(&bundle2, "dim_account"), 2, "2 comptes");

        // Colonne physique r{id} de la référence user : on doit retrouver la
        // valeur '101' attachée au compte '102' après import.
        let ref_col2 = custom_references::col_of_ref(&con2, "account", "compte_parent").unwrap();
        let parent102: String = con2
            .query_row(
                &format!(
                    "SELECT COALESCE(\"{ref_col2}\", '<null>') FROM dim_account WHERE code = '102'"
                ),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent102, "101", "rattachement r{{id}} préservé après round-trip");
    }

    /// Le filtre `exclude` permet de skipper des tables (utile au boot T3 pour
    /// réinjecter le staging à part, par exemple).
    #[test]
    fn import_bundle_respecte_exclude() {
        let con = Connection::open_in_memory().unwrap();
        create_schema(&con).unwrap();
        con.execute_batch(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES ('REEL','Réel');
             INSERT INTO dim_period (code, libelle) VALUES ('2024','2024');
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);
             INSERT INTO dim_variant (code, libelle) VALUES ('BASE','B');",
        )
        .unwrap();
        let mut excluded = HashSet::new();
        excluded.insert("dim_period");
        let bundle = json!({
            "dim_period": [{"code":"2099","libelle":"Futur"}],
            "dim_currency": [{"code_iso":"USD","libelle":"Dollar","decimales":2}],
        });
        let counts = import_bundle(&con, &bundle, &excluded).unwrap();
        assert_eq!(
            counts.get("dim_period").and_then(|v| v.as_u64()),
            Some(0),
            "dim_period exclue"
        );
        // dim_currency a été insérée.
        let usd: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM dim_currency WHERE code_iso = 'USD'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(usd, 1, "dim_currency insérée");
    }
}
