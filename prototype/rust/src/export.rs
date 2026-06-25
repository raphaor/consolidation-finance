//! Export / import **complet** de l'état applicatif en un **paquet JSON unique**.
//!
//! - `GET  /api/export`     : sérialise toutes les tables persistantes en un seul
//!   objet JSON `{ table → [lignes] }` (+ `dim_custom_dimension` et `_meta`).
//! - `POST /api/import/all` : restaure l'état depuis un tel paquet — **remplacement
//!   total** (DROP + CREATE du schéma, recréation des dimensions custom, puis
//!   réinsertion de toutes les lignes). Ne relance **pas** le pipeline (comme
//!   `/api/reset`) : l'utilisateur clique « Lancer le pipeline » ensuite.
//!
//! `fact_entry` est volontairement exclue : c'est une table **dérivée**,
//! reconstruite par le pipeline depuis `stg_entry`.
//!
//! Contrairement à `load_all` (qui ne charge que les 16 CSV de référentiels),
//! ce paquet inclut aussi les **règles** (`dim_rule` / `dim_ruleset` /
//! `dim_ruleset_item`) et les **dimensions custom** — il capture donc l'état
//! que le seed CSV ne couvre pas. C'est le « tout exporter / ré-importer » côté
//! sauvegarde/restauration.

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use duckdb::{params_from_iter, types::Value as DbValue, Connection};
use serde_json::{Map, Value as JsonValue};
use std::sync::Arc;

use crate::create_schema;
use crate::dimensions;
use crate::masterdata::{json_to_db_value, run_query};
use crate::references;
use crate::resolve;
use crate::state::{db_err, lock_con, AppError, AppState};

/// Tables persistantes, dans l'ordre d'insertion (dépendances amont d'abord).
///
/// `fact_entry` est exclue (dérivée). `dim_custom_dimension` est gérée à part
/// (elle ne se contente pas d'insérer des lignes : elle recrée des colonnes).
const TABLES: &[&str] = &[
    "app_config",
    "dim_scenario_category",
    "dim_variant",
    "dim_rate_set",
    "dim_perimeter_set",
    "dim_rule",
    "dim_ruleset",
    "dim_ruleset_item",
    "dim_consolidation",
    "dim_entity",
    "dim_period",
    "dim_sous_classe",
    "dim_account",
    "dim_flow",
    "dim_flow_scheme",
    "dim_currency",
    "dim_nature",
    "dim_method",
    "sat_perimeter",
    "sat_exchange_rate",
    "sat_flow_scheme_item",
    "stg_entry",
];

/// GET /api/export — paquet JSON complet de l'état.
async fn export_all(State(state): State<Arc<AppState>>) -> Result<Json<JsonValue>, AppError> {
    let bundle = {
        let con = lock_con(&state)?;
        let mut obj = Map::new();

        // Dimensions custom d'abord (recréées en premier à l'import).
        obj.insert(
            "dim_custom_dimension".to_string(),
            JsonValue::Array(run_query(
                &con,
                "SELECT name, label FROM dim_custom_dimension ORDER BY name",
                Vec::new(),
            )?),
        );

        // `SELECT *` par table : capture aussi les colonnes custom de stg_entry.
        for t in TABLES {
            let rows = run_query(&con, &format!("SELECT * FROM {t}"), Vec::new())?;
            obj.insert((*t).to_string(), JsonValue::Array(rows));
        }

        // Coefficients : seuls les **utilisateur** sont exportés (les natifs sont
        // re-seedés par `create_schema` à l'import → éviter le doublon de PK).
        obj.insert(
            "dim_coefficient".to_string(),
            JsonValue::Array(run_query(
                &con,
                "SELECT code, libelle, expression, kind \
                 FROM dim_coefficient WHERE kind = 'user' ORDER BY code",
                Vec::new(),
            )?),
        );

        let mut meta = Map::new();
        meta.insert(
            "format".to_string(),
            JsonValue::String("conso-export-v2".to_string()),
        );
        obj.insert("_meta".to_string(), JsonValue::Object(meta));

        JsonValue::Object(obj)
    };
    Ok(Json(bundle))
}

/// POST /api/import/all — restaure l'état depuis un paquet (remplacement total).
async fn import_all(
    State(state): State<Arc<AppState>>,
    Json(bundle): Json<JsonValue>,
) -> Result<Json<JsonValue>, AppError> {
    let obj = bundle
        .as_object()
        .ok_or_else(|| AppError::bad_request("le paquet doit être un objet JSON"))?;

    let counts = {
        let con = lock_con(&state)?;

        // 1. Table rase : DROP + CREATE de tout le schéma.
        create_schema(&con).map_err(db_err)?;

        // 2. Recréer les dimensions custom (ALTER colonnes + registre) AVANT
        //    d'insérer stg_entry, qui peut porter ces colonnes.
        if let Some(JsonValue::Array(customs)) = obj.get("dim_custom_dimension") {
            for c in customs {
                let name = c
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| AppError::bad_request("dim_custom_dimension.name manquant"))?;
                let label = c.get("label").and_then(|v| v.as_str()).unwrap_or(name);
                dimensions::create_custom(&con, name, label).map_err(db_err)?;
            }
        }

        // 3. Insérer chaque table dans l'ordre des dépendances.
        let mut counts = Map::new();
        for t in TABLES {
            let n = insert_table(&con, t, obj.get(*t))?;
            counts.insert((*t).to_string(), JsonValue::Number(n.into()));
        }

        // 4. Coefficients utilisateur (les natifs ont été re-seedés par
        //    create_schema ; le paquet ne contient que les `kind = 'user'`).
        let n_coef = insert_table(&con, "dim_coefficient", obj.get("dim_coefficient"))?;
        counts.insert("dim_coefficient".to_string(), JsonValue::Number(n_coef.into()));

        JsonValue::Object(counts)
    };

    Ok(Json(
        serde_json::json!({ "status": "ok", "imported": counts }),
    ))
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
             INSERT INTO dim_rate_set (code, libelle) VALUES ('RT','R');",
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

        // Les FK sont stockées en id ; relues via les cibles.
        let (phase_ok, variant_ok): (bool, bool) = con
            .query_row(
                "SELECT
                   variant  = (SELECT id FROM dim_variant WHERE code='BASE'),
                   phase    = (SELECT id FROM dim_scenario_category WHERE code='REEL')
                 FROM dim_consolidation WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(phase_ok && variant_ok, "FK résolues en id à l'import");

        // Code inexistant : rejeté proprement.
        let bad = json!({ "id": 2, "variant": "NOPE" });
        assert!(insert_table(&con, "dim_consolidation", Some(&json!([bad]))).is_err());
    }
}
