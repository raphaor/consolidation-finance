//! Saisie manuelle d'écritures : `POST` / `PUT` / `DELETE /api/entries`.
//!
//! Cible `stg_entry` (saisie brute, niveau `raw` avant pipeline). Permet :
//! - l'ajout par lot (`POST`) de plusieurs lignes avant enregistrement ;
//! - l'édition unitaire (`PUT /api/entries/{id}`) ;
//! - la suppression unitaire (`DELETE /api/entries/{id}`).
//!
//! Toute ligne créée via cette API est marquée `source = 'MANUAL'`. L'édition
//! et la suppression sont **refusées** sur les lignes dont `source ≠ MANUAL`
//! (protection anti-écrasement des imports CSV).
//!
//! La lecture reste assurée par `GET /api/entries` (cf. `bin/server.rs`) ; cette
//! API ne fait que_mutations. Le pipeline n'est pas relancé automatiquement :
//! c'est à l'utilisateur de le déclencher via la page Pipeline après saisie.

use axum::{
    extract::{Path, State},
    Json,
};
use duckdb::types::Value as DbValue;
use duckdb::params_from_iter;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::HashSet;
use std::sync::Arc;

use crate::references;
use crate::state::{lock_con, AppError, AppState};

/// Marqueur de provenance appliqué à toute saisie manuelle. Les lignes dont
/// `source = MANUAL` sont éditables/supprimables via cette API ; les autres
/// (imports CSV) ne le sont pas.
pub const MANUAL_SOURCE: &str = "MANUAL";

/// Colonnes dimensionnelles de `stg_entry` dans l'ordre canonique du schéma
/// (hors `id`, `source`, `amount`). Sert à générer INSERT / UPDATE cohérents.
const DIM_COLS: &[&str] = &[
    "scenario",
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
];

/// Champs obligatoires d'une saisie (hors `amount`). Miroir des références
/// `required` du graphe `references_for("stg_entry")`, durci ici pour un
/// message d'erreur explicite par champ manquant.
const REQUIRED_COLS: &[&str] = &[
    "scenario",
    "entity",
    "entry_period",
    "period",
    "account",
    "flow",
    "currency",
    "nature",
];

/// Ligne unitaire d'écriture reçue du front. Toutes les dimensions sont en
/// `Option<String>` pour tolérer les champs vides : la validation se fait a
/// posteriori dans [`validate_entry_rows`]. `amount` est reçu en string pour
/// tolérer la virgule décimale (locale fr).
#[derive(Deserialize, Debug, Default, Clone)]
pub struct EntryInput {
    #[serde(default)]
    pub scenario: Option<String>,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub entry_period: Option<String>,
    #[serde(default)]
    pub period: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub flow: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub nature: Option<String>,
    #[serde(default)]
    pub partner: Option<String>,
    #[serde(default)]
    pub share: Option<String>,
    #[serde(default)]
    pub analysis: Option<String>,
    #[serde(default)]
    pub analysis2: Option<String>,
    #[serde(default)]
    pub amount: Option<String>,
}

/// Récupère la valeur d'une colonne par nom dans un [`EntryInput`].
fn col_value(row: &EntryInput, col: &str) -> Option<String> {
    match col {
        "scenario" => row.scenario.clone(),
        "entity" => row.entity.clone(),
        "entry_period" => row.entry_period.clone(),
        "period" => row.period.clone(),
        "account" => row.account.clone(),
        "flow" => row.flow.clone(),
        "currency" => row.currency.clone(),
        "nature" => row.nature.clone(),
        "partner" => row.partner.clone(),
        "share" => row.share.clone(),
        "analysis" => row.analysis.clone(),
        "analysis2" => row.analysis2.clone(),
        _ => None,
    }
}

/// Parse un montant : accepte la virgule ou le point décimal. Requis (non-vide).
fn parse_amount(raw: Option<&str>) -> Result<Decimal, AppError> {
    let s = raw.unwrap_or("").trim();
    if s.is_empty() {
        return Err(AppError::bad_request("montant manquant"));
    }
    let normalized = s.replace(',', ".");
    normalized.parse::<Decimal>().map_err(|_e| {
        AppError::bad_request(format!("montant invalide : '{s}'"))
    })
}

/// Construit les paramètres d'une ligne dans l'ordre `(12 dims, source, amount)`
/// — correspond à l'ordre canonique utilisé par les SQL INSERT et UPDATE.
/// Les `None` (ou chaînes vides) deviennent `NULL` ([`DbValue::Null`]).
fn row_params(row: &EntryInput, source: &str) -> Result<Vec<DbValue>, AppError> {
    let text = |v: &Option<String>| -> DbValue {
        match v {
            Some(s) if !s.is_empty() => DbValue::Text(s.clone()),
            _ => DbValue::Null,
        }
    };
    let mut vals: Vec<DbValue> = vec![
        text(&row.scenario),
        text(&row.entity),
        text(&row.entry_period),
        text(&row.period),
        text(&row.account),
        text(&row.flow),
        text(&row.currency),
        text(&row.nature),
        text(&row.partner),
        text(&row.share),
        text(&row.analysis),
        text(&row.analysis2),
        DbValue::Text(source.to_string()),
    ];
    let amount_dec = parse_amount(row.amount.as_deref())?;
    vals.push(DbValue::Decimal(amount_dec));
    Ok(vals)
}

/// Convertit une erreur `Display` en `AppError` HTTP 500 (interne).
fn db_err_internal<E: std::fmt::Display>(e: E) -> AppError {
    AppError(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        e.to_string(),
    )
}

/// Valide un lot d'écritures entrantes :
/// 1. champs obligatoires renseignés (par ligne) ;
/// 2. montant parse-valide (par ligne) ;
/// 3. cohérence référentielle : anti-jointure des valeurs distinctes non-vides
///    de chaque colonne référente vers sa table cible.
///
/// Renvoie `Err` agrégé si au moins une ligne est invalide. La validation est
/// intentionnellement **bloquante** : aucune ligne n'est insérée si le lot est
/// invalide (transaction annulée côté handler).
pub fn validate_entry_rows(
    con: &duckdb::Connection,
    rows: &[EntryInput],
) -> Result<(), AppError> {
    if rows.is_empty() {
        return Err(AppError::bad_request("aucune ligne à insérer"));
    }
    let mut errors: Vec<String> = Vec::new();

    // 1 & 2. Champs obligatoires + format du montant pour chaque ligne.
    for (i, row) in rows.iter().enumerate() {
        let mut missing: Vec<&str> = Vec::new();
        for col in REQUIRED_COLS {
            let v = col_value(row, col);
            if v.map(|s| s.is_empty()).unwrap_or(true) {
                missing.push(col);
            }
        }
        if row
            .amount
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push("amount");
        }
        if !missing.is_empty() {
            errors.push(format!(
                "ligne {} : champ(s) obligatoire(s) manquant(s) : {}",
                i + 1,
                missing.join(", ")
            ));
        } else if let Err(e) = parse_amount(row.amount.as_deref()) {
            // On ne re-valide le format que si le champ est non-vide (sinon
            // déjà couvert par missing ci-dessus).
            errors.push(format!("ligne {} : {}", i + 1, e.1));
        }
    }

    // 3. Cohérence référentielle : pour chaque colonne référente de stg_entry,
    //    anti-jointure des valeurs non-vides distinctes avec la table cible.
    for r in references::references_for("stg_entry") {
        let col = r.column;
        // Valeurs distinctes non-vides du lot.
        let mut vals: Vec<String> = rows
            .iter()
            .filter_map(|row| col_value(row, col))
            .filter(|s| !s.is_empty())
            .collect();
        vals.sort();
        vals.dedup();
        if vals.is_empty() {
            continue;
        }
        let placeholders = vals.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT CAST(\"{tcol}\" AS VARCHAR) AS v \
             FROM {ttab} \
             WHERE CAST(\"{tcol}\" AS VARCHAR) IN ({placeholders})",
            tcol = r.target_column,
            ttab = r.target_table,
            placeholders = placeholders,
        );
        let params: Vec<DbValue> = vals.iter().map(|s| DbValue::Text(s.clone())).collect();
        let found: HashSet<String> = {
            let mut stmt = con.prepare(&sql).map_err(|e| {
                AppError::bad_request(format!("validation référence {col} : {e}"))
            })?;
            let rows_iter = stmt
                .query_map(params_from_iter(params), |row| row.get::<_, String>(0))
                .map_err(|e| AppError::bad_request(format!("validation référence {col} : {e}")))?;
            let mut out = HashSet::new();
            for x in rows_iter {
                out.insert(
                    x.map_err(|e| {
                        AppError::bad_request(format!("validation référence {col} : {e}"))
                    })?,
                );
            }
            out
        };
        let missing_vals: Vec<&str> = vals
            .iter()
            .filter(|v| !found.contains(*v))
            .map(|s| s.as_str())
            .collect();
        if !missing_vals.is_empty() {
            errors.push(format!(
                "{} : valeur(s) inconnue(s) de {} → {}",
                col,
                r.target_table,
                missing_vals.join(", ")
            ));
        }
    }

    if !errors.is_empty() {
        return Err(AppError::bad_request(format!(
            "saisie invalide : {}",
            errors.join(" ; ")
        )));
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Handlers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EntriesBatch {
    pub rows: Vec<EntryInput>,
}

/// `POST /api/entries` — insère un lot de saisies manuelles dans `stg_entry`.
///
/// Force `source = 'MANUAL'`. Le lot est validé en entier avant toute insertion
/// (validate_entry_rows), puis inséré dans une transaction : si une ligne
/// échoue, aucune n'est écrite.
///
/// Réponse : `{ "inserted": <n>, "ids": [<id>, ...] }`.
pub async fn create_entries(
    State(state): State<Arc<AppState>>,
    Json(batch): Json<EntriesBatch>,
) -> Result<Json<JsonValue>, AppError> {
    let rows = batch.rows;

    let con = lock_con(&state)?;
    validate_entry_rows(&con, &rows)?;

    con.execute_batch("BEGIN")
        .map_err(db_err_internal)?;

    let outcome: Result<(usize, Vec<i64>), AppError> = (|| {
        let mut count = 0usize;
        let mut ids: Vec<i64> = Vec::with_capacity(rows.len());
        let placeholders = DIM_COLS.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let col_list = DIM_COLS.join(", ");
        let sql = format!(
            "INSERT INTO stg_entry ({col_list}, source, amount) \
             VALUES ({placeholders}, ?, ?) \
             RETURNING id"
        );
        for row in &rows {
            let params = row_params(row, MANUAL_SOURCE)?;
            let id: i64 = con
                .query_row(&sql, params_from_iter(params), |r| r.get(0))
                .map_err(db_err_internal)?;
            ids.push(id);
            count += 1;
        }
        Ok((count, ids))
    })();

    match outcome {
        Ok((count, ids)) => {
            con.execute_batch("COMMIT").map_err(db_err_internal)?;
            Ok(Json(json!({ "inserted": count, "ids": ids })))
        }
        Err(e) => {
            // Best-effort rollback ; on ne masque pas l'erreur d'origine.
            let _ = con.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// `PUT /api/entries/{id}` — modifie une ligne `stg_entry` existante.
///
/// Refusé (HTTP 400) si la ligne n'a pas `source = 'MANUAL'` (protection des
/// imports CSV). Valide la nouvelle valeur comme une saisie fraîche.
pub async fn update_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(row): Json<EntryInput>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;

    // 1. La ligne doit exister et avoir source = MANUAL.
    let source: Option<String> = con
        .query_row(
            "SELECT source FROM stg_entry WHERE id = ?",
            &[&id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            duckdb::Error::QueryReturnedNoRows => {
                AppError::not_found(format!("écriture {id} introuvable"))
            }
            other => db_err_internal(other),
        })?;
    if source.as_deref() != Some(MANUAL_SOURCE) {
        return Err(AppError::bad_request(format!(
            "écriture {id} non éditable (source ≠ MANUAL) : protection anti-écrasement"
        )));
    }

    // 2. Valide la nouvelle valeur de la ligne.
    validate_entry_rows(&con, std::slice::from_ref(&row))?;

    // 3. UPDATE. row_params renvoie (12 dims, source, amount) — on garde le
    //    marqueur MANUAL pour ne pas le perdre (même si on pourrait le conserver
    //    via l'UPDATE SET source = source).
    let mut params = row_params(&row, MANUAL_SOURCE)?;
    params.push(DbValue::Int(id as i32));
    let set_list = DIM_COLS
        .iter()
        .map(|c| format!("{c} = ?"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "UPDATE stg_entry SET {set_list}, source = ?, amount = ? WHERE id = ?"
    );
    let n = con
        .execute(&sql, params_from_iter(params))
        .map_err(db_err_internal)?;
    Ok(Json(json!({ "updated": n, "id": id })))
}

/// `DELETE /api/entries/{id}` — supprime une ligne `stg_entry` existante.
///
/// Refusé (HTTP 400) si la ligne n'a pas `source = 'MANUAL'`.
pub async fn delete_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;

    let source: Option<String> = con
        .query_row(
            "SELECT source FROM stg_entry WHERE id = ?",
            &[&id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            duckdb::Error::QueryReturnedNoRows => {
                AppError::not_found(format!("écriture {id} introuvable"))
            }
            other => db_err_internal(other),
        })?;
    if source.as_deref() != Some(MANUAL_SOURCE) {
        return Err(AppError::bad_request(format!(
            "écriture {id} non supprimable (source ≠ MANUAL) : protection anti-écrasement"
        )));
    }

    let n = con
        .execute("DELETE FROM stg_entry WHERE id = ?", &[&id])
        .map_err(db_err_internal)?;
    Ok(Json(json!({ "deleted": n, "id": id })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn parse_amount_accepte_point_et_virgule() {
        assert_eq!(parse_amount(Some("123.45")).unwrap(), dec!(123.45));
        assert_eq!(parse_amount(Some("123,45")).unwrap(), dec!(123.45));
        assert_eq!(parse_amount(Some("  -42 ")).unwrap(), dec!(-42));
        assert!(parse_amount(None).is_err());
        assert!(parse_amount(Some("")).is_err());
        assert!(parse_amount(Some("abc")).is_err());
    }

    #[test]
    fn row_params_complete_null_pour_vides() {
        let row = EntryInput {
            scenario: Some("REEL".into()),
            entity: Some("M".into()),
            nature: Some("LIASSE".into()),
            amount: Some("100".into()),
            ..Default::default()
        };
        let v = row_params(&row, "MANUAL").unwrap();
        // 12 dims + source + amount = 14
        assert_eq!(v.len(), 14);
        // scenario rempli
        assert!(matches!(v[0], DbValue::Text(ref s) if s == "REEL"));
        // partner (index 8) vide → Null
        assert!(matches!(v[8], DbValue::Null));
        // source (index 12) = MANUAL
        assert!(matches!(v[12], DbValue::Text(ref s) if s == "MANUAL"));
        // amount = Decimal (index 13)
        assert!(matches!(v[13], DbValue::Decimal(_)));
    }
}
