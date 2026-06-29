//! Serveur HTTP/JSON exposant le moteur de consolidation via une API REST.
//!
//! Binaire `conso-server` du crate `conso-engine`. Démarre un serveur Axum
//! sur le port 3000 (par défaut) et expose les endpoints suivants :
//!
//! | Méthode | Route        | Description                       |
//! |---------|--------------|-----------------------------------|
//! | GET     | /api/health  | Health check                      |
//! | GET     | /api/levels  | Comptes par niveau                |
//! | GET     | /api/bilan   | Bilan par flux (consolidated)     |
//! | GET     | /api/entries | Écritures filtrées par niveau     |
//! | POST    | /api/run     | Déclenche le pipeline             |
//! | POST    | /api/reset   | Reset DB + reimport CSV           |
//!
//! # Configuration (variables d'environnement)
//!
//! - `CONSO_PORT`          : port d'écoute (défaut : 3000).
//! - `CONSO_DB_PATH`       : chemin du fichier DuckDB (défaut : `conso.duckdb`).
//! - `CONSO_CSV_DIR`       : répertoire contenant les CSV (défaut : `data`).
//! - `CONSO_WEB_DIR`       : répertoire du frontend buildé à servir en statique (défaut : `../../web/dist` depuis `prototype/rust`). Si absent, seule l'API est exposée.
//! - `CONSO_FORCE_RESEED`  : `1` pour forcer le rechargement CSV au démarrage (DROP schéma + import + pipeline), même si la base existe déjà. Utile après une évolution du schéma. À chaud, préférer `POST /api/reset`.
//!
//! # Persistance
//!
//! Au démarrage, les CSV ne sont réimportés que si la base est vierge (schéma
//! absent). Sinon, la base DuckDB existante est conservée telle quelle : les
//! éditions de master data faites via l'UI (périmètre, taux, entités…)
//! survivent aux redémarrages. Pour repartir des CSV : `POST /api/reset` ou
//! `CONSO_FORCE_RESEED=1`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use duckdb::params_from_iter;
use duckdb::types::Value as DbValue;
use duckdb::Connection;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};

use conso_engine::rules::{run_ruleset_at_level, validate_definition, RuleResult, RulesetReport};
use conso_engine::state::{db_err, lock_con, AppError, AppState};
use conso_engine::{
    characteristics, coefficients, controls, create_schema, custom_references, dimensions, entries,
    export, import, indicators, masterdata, money::Money, references, run_pipeline,
    run_pipeline_with_hook, seed_demo_controls,
    value_lists, ConvertParams,
};

// ─────────────────────────────────────────────────────────────────────────────
//  État partagé et erreurs
// ─────────────────────────────────────────────────────────────────────────────
//
// `AppState`, `AppError`, `db_err` et `lock_con` sont définis dans
// `conso_engine::state` et partagés avec les modules `masterdata` et `import`.

// ─────────────────────────────────────────────────────────────────────────────
//  DTO sérialisés en JSON
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `/api/levels` : nombre d'écritures à un niveau de stockage.
#[derive(Serialize)]
struct LevelCount {
    level: String,
    count: i64,
}

/// Ligne `/api/bilan` : montant agrégé par (compte, flux, nature) au niveau demandé.
///
/// `amount` est sérialisé en **nombre** JSON (feature `serde-float` de
/// `rust_decimal`) — le frontend TS attend `amount: 9774.0`, pas une chaîne.
#[derive(Serialize)]
struct BilanRow {
    account: String,
    flow: String,
    nature: String,
    /// Sens du compte dérivé de `dim_sous_classe.classe`/code : `"C"` créditeur
    /// (passif, produits) ou `"D"` débiteur (actif, charges). Permet au rapport
    /// de calculer le total comme Σ(comptes C) − Σ(comptes D) — équilibre du
    /// bilan, résultat du P&L. `"?"` si la sous-classe est inconnue/nulle.
    sens: String,
    amount: Decimal,
}

// Sens comptable (Q44) : désormais user-driven via `dim_sous_classe.sens`
// (colonne C/D). Les rapports bilan/P&L signent via un LEFT JOIN sur cette
// colonne (COALESCE '?' si NULL). L'ancien CASE en dur `SENS_CASE` est supprimé
// — `sous_classe` devient renommable (plus de code en dur référençant ses valeurs).

/// Avertissement de cohérence de l'à-nouveau (non bloquant), remonté au client
/// pour affichage dans l'UI plutôt qu'en console (cf. docs/A_NOUVEAU.md §5.1).
#[derive(Serialize)]
struct CoherenceWarning {
    kind: String,
    entity: String,
    detail: String,
}

/// Réponse `/api/run` : nombre de lignes produites à chaque étape du pipeline.
#[derive(Serialize)]
struct PipelineResult {
    corporate: usize,
    converted: usize,
    consolidated: usize,
    /// Identifiant (`id`) de la consolidation utilisée pour le run (explicite
    /// dans le body ou première consolidation `'ouvert'` trouvée).
    consolidation: i64,
    /// Ruleset exécuté après le pipeline (NULL si la consolidation n'en référence pas).
    ruleset: Option<String>,
    /// Rapport du ruleset, présent si `ruleset` est `Some`.
    ruleset_report: Option<RulesetReport>,
    /// Avertissements de cohérence de l'à-nouveau (non bloquants). Tableau vide
    /// si aucun ; remplace l'ancien `eprintln!` console.
    a_nouveau_warnings: Vec<CoherenceWarning>,
}

/// Réponse `/api/reset` : statut + nombre d'écritures brutes rechargées.
#[derive(Serialize)]
struct ResetResult {
    status: &'static str,
    entries: i64,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Paramètres de requête (query string)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BilanQuery {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default)]
    consolidation: Option<i64>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
}

#[derive(Deserialize)]
struct EntriesQuery {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    /// Filtre par consolidation (PK `dim_consolidation.id`) pour les niveaux
    /// `fact_entry` (corporate / converted / consolidated).
    #[serde(default)]
    consolidation: Option<i64>,
    /// Filtre par phase pour le niveau `raw` (`stg_entry.phase`).
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
    /// Filtre par provenance (`source`) : ex. `MANUAL` pour ne voir que les
    /// saisies manuelles, ou toute autre valeur de `stg_entry.source`. N'affecte
    /// que le niveau `raw` (le pipeline ne propage pas `source` aux niveaux
    /// corporate/converted/consolidated).
    #[serde(default)]
    source: Option<String>,
}

fn default_level() -> String {
    "consolidated".to_string()
}

fn default_limit() -> i64 {
    100
}

/// Construit le fragment SQL et les paramètres pour les filtres optionnels
/// `consolidation` (PK de la conso, isolation d'un run dans `fact_entry`),
/// `entity`, `entry_period` (exercice clôturé), `period` (période impactée par
/// l'écriture) et `nature`. Renvoie une chaîne préfixée par " AND ..." prête à
/// concaténer après un WHERE existant. Le filtre `consolidation` ne s'applique
/// qu'aux niveaux `fact_entry` (le niveau `raw` / `stg_entry` filtre sur `phase`,
/// géré séparément par l'appelant).
/// Filtres pour `stg_entry` (colonnes TEXT).
fn build_filters(
    consolidation: &Option<i64>,
    entity: &Option<String>,
    entry_period: &Option<String>,
    period: &Option<String>,
    nature: &Option<String>,
) -> (String, Vec<DbValue>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    if let Some(c) = consolidation {
        sql.push_str(" AND consolidation_id = ?");
        params.push(DbValue::BigInt(*c));
    }
    if let Some(e) = entity {
        sql.push_str(" AND entity = ?");
        params.push(DbValue::Text(e.clone()));
    }
    if let Some(ep) = entry_period {
        sql.push_str(" AND entry_period = ?");
        params.push(DbValue::Text(ep.clone()));
    }
    if let Some(p) = period {
        sql.push_str(" AND period = ?");
        params.push(DbValue::Text(p.clone()));
    }
    if let Some(n) = nature {
        sql.push_str(" AND nature = ?");
        params.push(DbValue::Text(n.clone()));
    }
    (sql, params)
}

/// Filtres pour `fact_entry` (colonnes INTEGER après étape 4 B1).
/// Les codes TEXT sont résolus en ids via sous-requêtes.
fn build_filters_fe(
    consolidation: &Option<i64>,
    entity: &Option<String>,
    entry_period: &Option<String>,
    period: &Option<String>,
    nature: &Option<String>,
) -> (String, Vec<DbValue>) {
    let mut sql = String::new();
    let mut params = Vec::new();
    if let Some(c) = consolidation {
        sql.push_str(" AND consolidation_id = ?");
        params.push(DbValue::BigInt(*c));
    }
    if let Some(e) = entity {
        sql.push_str(" AND entity = (SELECT id FROM dim_entity WHERE code = ?)");
        params.push(DbValue::Text(e.clone()));
    }
    if let Some(ep) = entry_period {
        sql.push_str(" AND entry_period = (SELECT id FROM dim_period WHERE code = ?)");
        params.push(DbValue::Text(ep.clone()));
    }
    if let Some(p) = period {
        sql.push_str(" AND period = (SELECT id FROM dim_period WHERE code = ?)");
        params.push(DbValue::Text(p.clone()));
    }
    if let Some(n) = nature {
        sql.push_str(" AND nature = (SELECT id FROM dim_nature WHERE code = ?)");
        params.push(DbValue::Text(n.clone()));
    }
    (sql, params)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Handlers
// ─────────────────────────────────────────────────────────────────────────────

/// GET /api/health — health check simple, toujours 200.
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// GET /api/levels — nombre de lignes stockées à chaque niveau de `fact_entry`.
///
/// Même SQL que `report::print_level_counts`, mais renvoyé en JSON.
async fn get_levels(State(state): State<Arc<AppState>>) -> Result<Json<Vec<LevelCount>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                "SELECT level, COUNT(*) AS n
                 FROM fact_entry
                 GROUP BY level
                 ORDER BY CASE level
                     WHEN 'corporate'    THEN 1
                     WHEN 'converted'    THEN 2
                     WHEN 'consolidated' THEN 3
                 END",
            )
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(LevelCount {
                    level: row.get(0)?,
                    count: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
        // stmt et MutexGuard libérés ici
    };
    Ok(Json(rows))
}

/// GET /api/bilan?level=consolidated — bilan par flux.
///
/// Le « bilan » au sens large (actif + passif + capitaux propres) regroupe les
/// comptes de classe `bilan`. Les comptes de `resultat` (P&L : classes 6/7) sont
/// exclus — ils sont exposés via `/api/compte-resultat`. On join `dim_account`
/// pour filtrer sur la classe.
async fn get_bilan(
    Query(q): Query<BilanQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BilanRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Totaux = lignes principales uniquement : on exclut les « dont »
        // (dimensions analytiques renseignées), qui sont des détails de la ligne
        // de même grain sans la dimension. Cf. dimensions::analytical_cols.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let of_which: String = dimensions::analytical_cols(&dims)
            .iter()
            .map(|c| format!(" AND e.{c} IS NULL"))
            .collect();
        let (fsql, fparams) = build_filters_fe(
            &q.consolidation,
            &q.entity,
            &q.entry_period,
            &q.period,
            &q.nature,
        );
        // fact_entry stocke account/flow/nature en INTEGER (étape 4 B1) →
        // JOINs sur id pour résoudre les codes et filtrer par classe.
        let sql = format!(
            "SELECT a.code AS account, df.code AS flow, n.code AS nature,
                    COALESCE(sc.sens, '?') AS sens, SUM(e.amount) AS amount
             FROM fact_entry e
             JOIN dim_account a ON a.id = e.account
             JOIN dim_flow df ON df.id = e.flow
             JOIN dim_nature n ON n.id = e.nature
             LEFT JOIN dim_sous_classe sc ON sc.id = a.sous_classe
             WHERE e.level = ? AND a.classe = 'bilan' {fsql}{of_which}
             GROUP BY a.code, df.code, n.code, a.sous_classe, sc.sens
             ORDER BY a.code, df.code, n.code"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(4)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
                    nature: row.get(2)?,
                    sens: row.get(3)?,
                    amount: m.into_decimal(),
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/compte-resultat?level=consolidated — compte de résultat par flux.
///
/// Restreint aux comptes de classe « resultat » (P&L : produits et charges).
async fn get_compte_resultat(
    Query(q): Query<BilanQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<BilanRow>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Totaux = lignes principales uniquement (exclut les « dont »). Idem bilan.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let of_which: String = dimensions::analytical_cols(&dims)
            .iter()
            .map(|c| format!(" AND e.{c} IS NULL"))
            .collect();
        let (fsql, fparams) = build_filters_fe(
            &q.consolidation,
            &q.entity,
            &q.entry_period,
            &q.period,
            &q.nature,
        );
        let sql = format!(
             "SELECT a.code AS account, df.code AS flow, n.code AS nature,
                     COALESCE(sc.sens, '?') AS sens, SUM(e.amount) AS amount
              FROM fact_entry e
              JOIN dim_account a ON a.id = e.account
              JOIN dim_flow df ON df.id = e.flow
              JOIN dim_nature n ON n.id = e.nature
              LEFT JOIN dim_sous_classe sc ON sc.id = a.sous_classe
              WHERE e.level = ? AND a.classe = 'resultat' {fsql}{of_which}
              GROUP BY a.code, df.code, n.code, a.sous_classe, sc.sens
              ORDER BY a.code, df.code, n.code"
        );
        let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
        params.extend(fparams);
        let mut stmt = con.prepare(&sql).map_err(db_err)?;
        let iter = stmt
            .query_map(params_from_iter(params), |row| {
                let m: Money = row.get(4)?;
                Ok(BilanRow {
                    account: row.get(0)?,
                    flow: row.get(1)?,
                    nature: row.get(2)?,
                    sens: row.get(3)?,
                    amount: m.into_decimal(),
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/entries?level=consolidated&limit=100&offset=0 — écritures paginées.
///
/// Colonnes **dynamiques** : toutes les dimensions propagées (built-in +
/// **custom**) + `id`, `level`, `amount`. La sérialisation générique
/// ([`masterdata::run_query`]) renvoie un objet JSON par ligne, donc les
/// dimensions custom apparaissent automatiquement (vue Écritures pilotée par
/// `/api/meta/dimensions`).
///
/// Niveau spécial `raw` : lit la saisie brute (`stg_entry`) avant pipeline.
/// `stg_entry` porte un `id` PK auto-incrémenté (seq_stg_entry) qui identifie
/// chaque ligne pour l'édition/suppression via PUT/DELETE /api/entries. La
/// colonne `source` (provenance) est exposée pour filtrer les saisies manuelles
/// (`source=MANUAL`).
async fn get_entries(
    Query(q): Query<EntriesQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<JsonValue>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        // Colonnes propagées (built-in + custom) depuis le registre.
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        let col_list = dimensions::propagated_cols(&dims).join(", ");
        let (sql, params): (String, Vec<DbValue>) = if q.level == "raw" {
            // Niveau raw (stg_entry) : le filtre de remontée porte sur `phase`
            // (et non consolidation_id, absent de stg_entry). Les autres filtres
            // (entity/entry_period/period/nature) sont communs.
            let (mut fsql, mut fparams) = build_filters(
                &None,
                &q.entity,
                &q.entry_period,
                &q.period,
                &q.nature,
            );
            if let Some(ph) = &q.phase {
                fsql.push_str(" AND phase = ?");
                fparams.push(DbValue::Text(ph.clone()));
            }
            // Filtre source (raw uniquement) : n'a pas de sens aux autres
            // niveaux car `source` n'est pas propagée par le pipeline.
            let source_clause = match &q.source {
                Some(s) => {
                    let mut p = Vec::new();
                    p.push(DbValue::Text(s.clone()));
                    (format!(" AND source = ?"), p)
                }
                None => (String::new(), Vec::new()),
            };
            // Le WHERE sur stg_entry est composé à partir des filtres standards
            // (préfixés " AND ..." par build_filters + phase) et du filtre source.
            let has_filters = !fsql.is_empty() || !source_clause.0.is_empty();
            let where_stg = if has_filters {
                format!("WHERE {}", {
                    let mut combined = String::new();
                    if !fsql.is_empty() {
                        combined.push_str(fsql.trim_start_matches(" AND "));
                    }
                    if !source_clause.0.is_empty() {
                        if !combined.is_empty() {
                            combined.push_str(" AND");
                        }
                        combined.push_str(source_clause.0.trim_start_matches(" AND "));
                    }
                    combined
                })
            } else {
                String::new()
            };
            let sql = format!(
                "SELECT id, {col_list}, source, 'raw' AS level, amount
                 FROM stg_entry {where_stg}
                 ORDER BY id
                 LIMIT ? OFFSET ?"
            );
            fparams.extend(source_clause.1);
            fparams.push(DbValue::BigInt(q.limit));
            fparams.push(DbValue::BigInt(q.offset));
            (sql, fparams)
        } else {
            let (fsql, fparams) = build_filters_fe(
                &q.consolidation,
                &q.entity,
                &q.entry_period,
                &q.period,
                &q.nature,
            );
            // Résoudre les colonnes INTEGER de fact_entry en codes TEXT pour l'UI.
            // Chaque dim avec master data → alias JOIN + projection code.
            // Les dims libres (analysis, analysis2, custom) → e.{dim} directement.
            let mut fe_joins = String::new();
            let mut fe_select: Vec<String> = Vec::new();
            for dim in &dims {
                let name = &dim.name;
                if let Some((table, code_col)) = references::dimension_master(name) {
                    let alias = format!("_fe{name}");
                    let join_type = if dim.nullable() { "LEFT JOIN" } else { "JOIN" };
                    fe_joins.push_str(&format!(
                        "\n{join_type} {table} {alias} ON {alias}.id = e.{name}"
                    ));
                    fe_select.push(format!("{alias}.{code_col} AS {name}"));
                } else {
                    fe_select.push(format!("e.{name}"));
                }
            }
            let fe_select_list = fe_select.join(", ");
            let sql = format!(
                "SELECT e.id, {fe_select_list}, NULL AS source, e.level, e.amount
                 FROM fact_entry e{fe_joins}
                 WHERE e.level = ? {fsql}
                 ORDER BY e.id
                 LIMIT ? OFFSET ?"
            );
            let mut params: Vec<DbValue> = vec![DbValue::Text(q.level.clone())];
            params.extend(fparams);
            params.push(DbValue::BigInt(q.limit));
            params.push(DbValue::BigInt(q.offset));
            (sql, params)
        };
        masterdata::run_query(&con, &sql, params)?
    };
    Ok(Json(rows))
}

/// Corps accepté par `POST /api/run`.
///
/// `consolidation_id` est optionnel : s'il est absent (body `{}`), le handler
/// sélectionne la première consolidation de statut `'ouvert'`. C'est l'amorti
/// de rétro-compatibilité pendant le développement.
#[derive(Deserialize, Default)]
struct RunBody {
    #[serde(default)]
    consolidation_id: Option<i64>,
}

/// GET /api/consolidations — liste des consolidations avec leurs paramètres dépliés.
///
/// Sert le dropdown UI de `PipelinePage`. Une entrée par consolidation ; les FK
/// (phase, variant, rate_set, ruleset_code) sont ramenées telles quelles (leurs
/// libellés sont récupérés côté UI via les tables master data).
#[derive(Serialize)]
struct ConsolidationSummary {
    id: i64,
    libelle: Option<String>,
    phase: Option<String>,
    exercice: Option<String>,
    perimeter_set: Option<String>,
    variant: Option<String>,
    presentation_currency: Option<String>,
    perimeter_period: Option<String>,
    rate_set: Option<String>,
    rate_period: Option<String>,
    ruleset_code: Option<String>,
    a_nouveau_consolidation_id: Option<i64>,
    statut: Option<String>,
}

/// GET /api/consolidations — liste de toutes les consolidations avec leurs paramètres.
async fn list_consolidations(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ConsolidationSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare(
                // Toutes les FK de dim_consolidation sont en clé technique (id, B1) :
                // résolues par JOIN pour exposer les codes en contrat externe.
                "SELECT c.id, c.libelle, sc.code AS phase, p_ex.code AS exercice,
                        ps.code AS perimeter_set, v.code AS variant,
                        cur.code_iso AS presentation_currency, p_pp.code AS perimeter_period,
                        rs.code AS rate_set, p_rp.code AS rate_period,
                        rset.code AS ruleset_code, c.a_nouveau_consolidation_id, c.statut
                 FROM dim_consolidation c
                 LEFT JOIN dim_scenario_category sc ON sc.id = c.phase
                 LEFT JOIN dim_period p_ex ON p_ex.id = c.exercice
                 LEFT JOIN dim_perimeter_set ps ON ps.id = c.perimeter_set
                 LEFT JOIN dim_variant v ON v.id = c.variant
                 LEFT JOIN dim_currency cur ON cur.id = c.presentation_currency
                 LEFT JOIN dim_period p_pp ON p_pp.id = c.perimeter_period
                 LEFT JOIN dim_rate_set rs ON rs.id = c.rate_set
                 LEFT JOIN dim_period p_rp ON p_rp.id = c.rate_period
                 LEFT JOIN dim_ruleset rset ON rset.id = c.ruleset_code
                 ORDER BY c.id",
            )
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(ConsolidationSummary {
                    id: row.get(0)?,
                    libelle: row.get(1)?,
                    phase: row.get(2)?,
                    exercice: row.get(3)?,
                    perimeter_set: row.get(4)?,
                    variant: row.get(5)?,
                    presentation_currency: row.get(6)?,
                    perimeter_period: row.get(7)?,
                    rate_set: row.get(8)?,
                    rate_period: row.get(9)?,
                    ruleset_code: row.get(10)?,
                    a_nouveau_consolidation_id: row.get(11)?,
                    statut: row.get(12)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// POST /api/run — déclenche le pipeline 3 étapes (sur la consolidation du body,
/// sinon la 1ère consolidation `'ouvert'`) et, si la consolidation référence un
/// ruleset, exécute celui-ci après le pipeline.
///
/// Workflow :
/// 1. Résolution de l'id consolidation (body ou défaut `'ouvert'`).
/// 2. `ConvertParams::load_params(con, consolidation_id)`.
/// 3. `DELETE FROM fact_entry WHERE consolidation_id = ?` (reset de la conso
///    courante ; les autres consolidations — snapshot d'à-nouveau figé — sont
///    préservées).
/// 4. `run_pipeline(con, &params)`.
/// 5. Si `consolidation.ruleset_code` non NULL : `run_ruleset(con, ruleset_code)`.
/// 6. Retourne `{ counts, consolidation, ruleset?, ruleset_report? }`.
async fn run_pipeline_handler(
    State(state): State<Arc<AppState>>,
    body: Option<Json<RunBody>>,
) -> Result<Json<PipelineResult>, AppError> {
    let result = {
        let con = lock_con(&state)?;
        // 1. Résolution de la consolidation : explicite dans le body, sinon 1ère 'ouvert'.
        let consolidation_id: i64 = match body.and_then(|b| b.0.consolidation_id) {
            Some(id) => id,
            None => con
                .query_row(
                    "SELECT id FROM dim_consolidation \
                     WHERE statut = 'ouvert' OR statut IS NULL \
                     ORDER BY id LIMIT 1",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| {
                    AppError::bad_request(format!(
                        "aucune consolidation 'ouvert' trouvée (précisez {{\"consolidation_id\":N}}) : {e}"
                    ))
                })?,
        };

        // 2. Chargement des params depuis dim_consolidation + app_config.
        let params = ConvertParams::load_params(&con, consolidation_id).map_err(db_err)?;

        // 3. Lecture du ruleset_code (NULL si la consolidation n'en référence pas).
        //    B1 : stocké en id → résolu en code par LEFT JOIN.
        let ruleset_code: Option<String> = con
            .query_row(
                "SELECT rs.code FROM dim_consolidation c \
                 LEFT JOIN dim_ruleset rs ON rs.id = c.ruleset_code \
                 WHERE c.id = ?",
                [consolidation_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(db_err)?;

        // 4. Vider les résultats du pipeline de la CONSOLIDATION COURANTE avant
        //    de relancer (isolation par consolidation_id : les autres
        //    consolidations — ex. snapshot d'à-nouveau figé — sont préservées).
        //    Cf. docs/A_NOUVEAU.md §2.3 / §3.
        con.execute(
            "DELETE FROM fact_entry WHERE consolidation_id = ?",
            [consolidation_id],
        )
        .map_err(db_err)?;

        // 5. Pipeline. Si la consolidation référence un ruleset, on **intercale**
        //    ses règles au niveau qu'elles ciblent (hook post-étape) : une règle
        //    `converted` est injectée juste après l'étape C, puis consolidée par
        //    l'étape D — propagation identique à une écriture manuelle. Sinon,
        //    pipeline natif seul.
        let mut rule_results: Vec<RuleResult> = Vec::new();
        let counts = match &ruleset_code {
            Some(code) => {
                let mut hook = |c: &Connection, level: &str| -> duckdb::Result<()> {
                    rule_results.extend(run_ruleset_at_level(
                        c,
                        code,
                        level,
                        Some(consolidation_id),
                    )?);
                    Ok(())
                };
                run_pipeline_with_hook(&con, &params, &mut hook)
                    .map_err(db_err)?
                    .counts()
            }
            None => run_pipeline(&con, &params).map_err(db_err)?.counts(),
        };

        // 5b. À-nouveau : contrôle de cohérence **non bloquant** (cf.
        //     docs/A_NOUVEAU.md §5.1, A5 : statut `ouvert` toléré → on alerte).
        //     Les anomalies sont remontées dans la réponse (UI) au lieu de la
        //     console.
        let mut a_nouveau_warnings: Vec<CoherenceWarning> = Vec::new();
        if let Some(a_nouveau_id) = params.a_nouveau_consolidation_id {
            match conso_engine::validate::check_a_nouveau_coherence(
                &con,
                consolidation_id,
                a_nouveau_id,
                &params.exercice,
            ) {
                Ok(anomalies) => {
                    a_nouveau_warnings = anomalies
                        .into_iter()
                        .map(|a| CoherenceWarning {
                            kind: a.kind.to_string(),
                            entity: a.entity,
                            detail: a.detail,
                        })
                        .collect();
                }
                Err(e) => a_nouveau_warnings.push(CoherenceWarning {
                    kind: "controle_echoue".to_string(),
                    entity: String::new(),
                    detail: format!("contrôle de cohérence à-nouveau échoué : {e}"),
                }),
            }
        }

        // 6. Rapport du ruleset (agrégé depuis les niveaux intercalés).
        let ruleset_report = ruleset_code.as_ref().map(|code| RulesetReport {
            ruleset: code.clone(),
            total_generated: rule_results.iter().map(|r| r.generated).sum(),
            rules: rule_results,
        });

        PipelineResult {
            corporate: counts[0],
            converted: counts[1],
            consolidated: counts[2],
            consolidation: consolidation_id,
            ruleset: ruleset_code,
            ruleset_report,
            a_nouveau_warnings,
        }
    };
    Ok(Json(result))
}

/// POST /api/reset — reset complet : DROP + CREATE du schéma, puis import JSON
/// si `CONSO_SEED_JSON` est défini (sinon base vide). Ne lance pas le pipeline.
async fn reset_handler(State(state): State<Arc<AppState>>) -> Result<Json<ResetResult>, AppError> {
    let entries = {
        let con = lock_con(&state)?;
        create_schema(&con).map_err(db_err)?; // DROP + CREATE (idempotent)
        if let Some(json_path) = &state.seed_json {
            let raw = std::fs::read_to_string(json_path).map_err(|e| {
                db_err(format!(
                    "CONSO_SEED_JSON illisible ({json_path}) : {e}"
                ))
            })?;
            let bundle: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::bad_request(format!(
                    "CONSO_SEED_JSON ({json_path}) n'est pas un JSON valide : {e}"
                ))
            })?;
            let excluded = std::collections::HashSet::<&str>::new();
            export::import_bundle(&con, &bundle, &excluded)?;
        }
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM stg_entry", [], |row| row.get(0))
            .map_err(db_err)?;
        // Flushe le DDL du reset dans le fichier (WAL propre) — cf. CHECKPOINT
        // au démarrage.
        let _ = con.execute("CHECKPOINT", []);
        n
    };
    Ok(Json(ResetResult {
        status: "ok",
        entries,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Dimensions — registre central (built-in + custom)
//
//  Trois endpoints :
//  - GET    /api/meta/dimensions        : liste toutes les dimensions
//  - POST   /api/meta/dimensions        : crée une dimension custom
//  - DELETE /api/meta/dimensions/{name} : supprime une dimension custom
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `GET /api/meta/dimensions` : une dimension du registre.
#[derive(Serialize)]
struct DimensionInfo {
    name: String,
    category: String,
    custom: bool,
    label: String,
    pilotable: bool,
}

/// Corps accepté par `POST /api/meta/dimensions`.
#[derive(Deserialize)]
struct DimensionBody {
    name: String,
    label: String,
    #[serde(default)]
    target_dimension: Option<String>,
}

impl DimensionInfo {
    fn from_def(d: &dimensions::DimDef) -> Self {
        Self {
            name: d.name.clone(),
            category: format!("{:?}", d.category),
            custom: d.custom,
            label: d.label.clone(),
            pilotable: d.pilotable(),
        }
    }
}

/// GET /api/meta/dimensions — liste toutes les dimensions (built-in + custom).
async fn list_dimensions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DimensionInfo>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let dims = dimensions::load_all(&con).map_err(db_err)?;
        dims.iter().map(DimensionInfo::from_def).collect()
    };
    Ok(Json(rows))
}

/// POST /api/meta/dimensions — crée une dimension custom.
///
/// Valide le nom via `dimensions::is_valid_custom_name` et refuse les
/// doublons (built-in ou déjà présente dans le registre). Répond 201 + la
/// dimension créée.
async fn create_dimension(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DimensionBody>,
) -> Result<(StatusCode, Json<DimensionInfo>), AppError> {
    let info = {
        let con = lock_con(&state)?;
        dimensions::create_custom(&con, &body.name, &body.label, body.target_dimension.as_deref())
            .map_err(|e| {
            // Les erreurs de validation sont des `InvalidParameterName` → 400.
            AppError::bad_request(e.to_string())
        })?;
        DimensionInfo {
            name: body.name.clone(),
            category: "Analytical".to_string(),
            custom: true,
            label: body.label.clone(),
            pilotable: true,
        }
    };
    Ok((StatusCode::CREATED, Json(info)))
}

/// DELETE /api/meta/dimensions/{name} — supprime une dimension custom.
async fn delete_dimension(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        match dimensions::delete_custom(&con, &name) {
            Ok(()) => 1,
            Err(e) => {
                // Inexistante → 404 ; autre erreur DuckDB → 500.
                if matches!(e, duckdb::Error::InvalidParameterName(_)) {
                    return Err(AppError::not_found(e.to_string()));
                }
                return Err(db_err(e));
            }
        }
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

/// Corps accepté par `POST /api/meta/dimensions/{name}/rename`.
#[derive(Deserialize)]
struct DimensionRenameBody {
    new_name: String,
}

/// POST /api/meta/dimensions/{name}/rename — renomme une dimension custom.
///
/// La colonne physique (`x{id}`) reste inchangée. Seul le nom API mute.
/// Retourne 409 si `new_name` est déjà utilisé, 404 si `name` est inconnu.
async fn rename_dimension(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<DimensionRenameBody>,
) -> Result<Json<JsonValue>, AppError> {
    let con = lock_con(&state)?;
    dimensions::rename_custom(&con, &name, &body.new_name).map_err(|e| {
        if matches!(e, duckdb::Error::InvalidParameterName(_)) {
            AppError::not_found(e.to_string())
        } else {
            db_err(e)
        }
    })?;
    Ok(Json(serde_json::json!({ "renamed": true, "name": body.new_name })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Règles de consolidation — CRUD + exécution
//
//  Tables : `dim_rule` (bibliothèque), `dim_ruleset` + `dim_ruleset_item` (jeux
//  ordonnés). L'exécution d'un ruleset délègue à `conso_engine::rules::run_ruleset`.
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne `GET /api/rules` : résumé d'une règle (sans la définition, qui peut
/// être volumineuse).
#[derive(Serialize)]
struct RuleSummary {
    code: String,
    libelle: Option<String>,
}

/// Réponse `GET /api/rules/{code}` et `POST /api/rules` : règle complète avec
/// définition parsée en JSON.
#[derive(Serialize)]
struct RuleDetail {
    code: String,
    libelle: Option<String>,
    definition: JsonValue,
}

/// Corps accepté par `POST /api/rules` et `PUT /api/rules/{code}` :
/// `definition` peut être un objet JSON (re-sérialisé en TEXT) ou une chaîne
/// (utilisée telle quelle si déjà du JSON valide).
#[derive(Deserialize)]
struct RuleBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    definition: JsonValue,
}

/// Item ordonné d'un ruleset, joint à `dim_rule.libelle` quand la règle existe.
#[derive(Serialize)]
struct RulesetItemOut {
    ordre: i64,
    rule_code: String,
    #[serde(default)]
    libelle: Option<String>,
}

/// Réponse `GET /api/rulesets/{code}` : jeu + items ordonnés.
#[derive(Serialize)]
struct RulesetDetail {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    items: Vec<RulesetItemOut>,
}

/// Résumé d'un ruleset (sans items).
#[derive(Serialize)]
struct RulesetSummary {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
}

/// Corps accepté par `POST /api/rulesets` et `PUT /api/rulesets/{code}`.
#[derive(Deserialize)]
struct RulesetBody {
    code: String,
    #[serde(default)]
    libelle: Option<String>,
    #[serde(default)]
    items: Vec<RulesetItemIn>,
}

#[derive(Deserialize)]
struct RulesetItemIn {
    ordre: i64,
    rule_code: String,
}

/// Sérialise une `JsonValue` en chaîne compacte pour stockage TEXT.
fn definition_to_text(def: &JsonValue) -> Result<String, AppError> {
    serde_json::to_string(def)
        .map_err(|e| AppError::bad_request(format!("définition non sérialisable : {e}")))
}

/// Parse une chaîne TEXT en `JsonValue` (fallback sur la chaîne brute si elle
/// n'est pas du JSON valide — mais on attend du JSON).
fn text_to_definition(s: &str) -> JsonValue {
    serde_json::from_str(s).unwrap_or(JsonValue::String(s.to_string()))
}

/// GET /api/rules — liste toutes les règles (code, libelle).
async fn list_rules(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RuleSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_rule ORDER BY code")
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(RuleSummary {
                    code: row.get(0)?,
                    libelle: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/rules/{code} — détail d'une règle (définition parsée en JSON).
///
/// Les valeurs JSON sont **dénormalisées** (ids → codes) avant renvoi : le
/// stockage interne utilise des ids immuables (chantier B1), mais le contrat
/// API expose des codes pour que l'éditeur de règles reste lisible.
async fn get_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RuleDetail>, AppError> {
    let (rule_code, libelle, def_text) = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle, definition FROM dim_rule WHERE code = ?")
            .map_err(db_err)?;
        let mut iter = stmt
            .query_map([&code], |row| {
                let def: Option<String> = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    def,
                ))
            })
            .map_err(db_err)?;
        iter.next()
            .transpose()
            .map_err(db_err)?
            .ok_or_else(|| AppError::not_found(format!("règle {code} introuvable")))?
    };
    // Dénormaliser : traduire les ids stockés en codes pour l'API.
    let definition = if let Some(raw) = def_text {
        let con = lock_con(&state)?;
        let denorm = conso_engine::json_migration::denormalize_rule_definition(&con, &raw)
            .map_err(db_err)?;
        text_to_definition(&denorm)
    } else {
        JsonValue::Null
    };
    Ok(Json(RuleDetail {
        code: rule_code,
        libelle,
        definition,
    }))
}

/// POST /api/rules — crée une règle.
async fn create_rule(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RuleBody>,
) -> Result<(StatusCode, Json<RuleDetail>), AppError> {
    let definition_text = definition_to_text(&body.definition)?;
    let detail = {
        let con = lock_con(&state)?;
        let exists: bool = con
            .query_row(
                "SELECT COUNT(*) > 0 FROM dim_rule WHERE code = ?",
                [&body.code],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if exists {
            return Err(AppError::conflict(format!(
                "règle {} existe déjà",
                body.code
            )));
        }
        validate_definition(&con, &definition_text).map_err(AppError::bad_request)?;
        let definition_text = conso_engine::json_migration::normalize_rule_definition(
            &con, &definition_text,
        )
        .map_err(db_err)?;
        con.execute(
            "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
            params_from_iter(vec![
                DbValue::Text(body.code.clone()),
                body.libelle
                    .clone()
                    .map(DbValue::Text)
                    .unwrap_or(DbValue::Null),
                DbValue::Text(definition_text),
            ]),
        )
        .map_err(db_err)?;
        RuleDetail {
            code: body.code.clone(),
            libelle: body.libelle.clone(),
            definition: body.definition.clone(),
        }
    };
    Ok((StatusCode::CREATED, Json(detail)))
}

/// PUT /api/rules/{code} — modifie libelle et/ou definition d'une règle.
async fn update_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RuleBody>,
) -> Result<Json<RuleDetail>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request(
            "le `code` du body ne correspond pas à l'URL",
        ));
    }
    let definition_text = definition_to_text(&body.definition)?;
    let detail = {
        let con = lock_con(&state)?;
        validate_definition(&con, &definition_text).map_err(AppError::bad_request)?;
        let definition_text = conso_engine::json_migration::normalize_rule_definition(
            &con, &definition_text,
        )
        .map_err(db_err)?;
        let n = con
            .execute(
                "UPDATE dim_rule SET libelle = ?, definition = ? WHERE code = ?",
                params_from_iter(vec![
                    body.libelle
                        .clone()
                        .map(DbValue::Text)
                        .unwrap_or(DbValue::Null),
                    DbValue::Text(definition_text),
                    DbValue::Text(code.clone()),
                ]),
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(AppError::not_found(format!("règle {code} introuvable")));
        }
        RuleDetail {
            code: body.code.clone(),
            libelle: body.libelle.clone(),
            definition: body.definition.clone(),
        }
    };
    Ok(Json(detail))
}

/// DELETE /api/rules/{code} — supprime une règle.
///
/// Si la règle est référencée par un `dim_ruleset_item`, on renvoie 409
/// (Conflict) avec un message listant les rulesets concernés ; l'utilisateur
/// doit d'abord retirer la règle des jeux qui la référencent.
async fn delete_rule(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        // Vérifier les références avant suppression.
        let referees: Vec<String> = {
            let mut stmt = con
                .prepare(
                    "SELECT DISTINCT rs.code \
                     FROM dim_ruleset_item i \
                     JOIN dim_ruleset rs ON rs.id = i.ruleset_code \
                     WHERE i.rule_code = ? \
                     ORDER BY rs.code",
                )
                .map_err(db_err)?;
            let iter = stmt
                .query_map([&code], |row| row.get::<_, String>(0))
                .map_err(db_err)?;
            let mut v = Vec::new();
            for r in iter {
                v.push(r.map_err(db_err)?);
            }
            v
        };
        if !referees.is_empty() {
            return Err(AppError::conflict(format!(
                "la règle {code} est référencée par les rulesets : {}",
                referees.join(", ")
            )));
        }
        con.execute("DELETE FROM dim_rule WHERE code = ?", [&code])
            .map_err(db_err)?
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

/// GET /api/rulesets — liste tous les rulesets (sans items).
async fn list_rulesets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RulesetSummary>>, AppError> {
    let rows = {
        let con = lock_con(&state)?;
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_ruleset ORDER BY code")
            .map_err(db_err)?;
        let iter = stmt
            .query_map([], |row| {
                Ok(RulesetSummary {
                    code: row.get(0)?,
                    libelle: row.get(1)?,
                })
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r.map_err(db_err)?);
        }
        out
    };
    Ok(Json(rows))
}

/// GET /api/rulesets/{code} — détail + items ordonnés (avec libellés des règles).
async fn get_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<RulesetDetail>, AppError> {
    let detail = {
        let con = lock_con(&state)?;
        build_ruleset_detail(&con, &code)?
    };
    Ok(Json(detail))
}

/// POST /api/rulesets — crée un ruleset avec ses items.
async fn create_ruleset(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RulesetBody>,
) -> Result<(StatusCode, Json<RulesetDetail>), AppError> {
    let detail = {
        let con = lock_con(&state)?;
        let exists: bool = con
            .query_row(
                "SELECT COUNT(*) > 0 FROM dim_ruleset WHERE code = ?",
                [&body.code],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if exists {
            return Err(AppError::conflict(format!(
                "ruleset {} existe déjà",
                body.code
            )));
        }
        con.execute(
            "INSERT INTO dim_ruleset (code, libelle) VALUES (?, ?)",
            params_from_iter(vec![
                DbValue::Text(body.code.clone()),
                body.libelle
                    .clone()
                    .map(DbValue::Text)
                    .unwrap_or(DbValue::Null),
            ]),
        )
        .map_err(db_err)?;
        insert_ruleset_items(&con, &body.code, &body.items)?;
        build_ruleset_detail(&con, &body.code)?
    };
    Ok((StatusCode::CREATED, Json(detail)))
}

/// PUT /api/rulesets/{code} — modifie un ruleset (libellé + réordonnancement
/// complet des items).
async fn update_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<RulesetBody>,
) -> Result<Json<RulesetDetail>, AppError> {
    if body.code != code {
        return Err(AppError::bad_request(
            "le `code` du body ne correspond pas à l'URL",
        ));
    }
    let detail = {
        let con = lock_con(&state)?;
        let n = con
            .execute(
                "UPDATE dim_ruleset SET libelle = ? WHERE code = ?",
                params_from_iter(vec![
                    body.libelle
                        .clone()
                        .map(DbValue::Text)
                        .unwrap_or(DbValue::Null),
                    DbValue::Text(code.clone()),
                ]),
            )
            .map_err(db_err)?;
        if n == 0 {
            return Err(AppError::not_found(format!("ruleset {code} introuvable")));
        }
        // Réordonnancement complet : on supprime tous les items puis on
        // ré-insère ceux du body.
        con.execute(
            "DELETE FROM dim_ruleset_item WHERE ruleset_code = (SELECT id FROM dim_ruleset WHERE code = ?)",
            [&code],
        )
        .map_err(db_err)?;
        insert_ruleset_items(&con, &code, &body.items)?;
        build_ruleset_detail(&con, &code)?
    };
    Ok(Json(detail))
}

/// DELETE /api/rulesets/{code} — supprime le ruleset + ses items.
async fn delete_ruleset(
    Path(code): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<JsonValue>, AppError> {
    let deleted = {
        let con = lock_con(&state)?;
        con.execute(
            "DELETE FROM dim_ruleset_item WHERE ruleset_code = (SELECT id FROM dim_ruleset WHERE code = ?)",
            [&code],
        )
        .map_err(db_err)?;
        let n = con
            .execute("DELETE FROM dim_ruleset WHERE code = ?", [&code])
            .map_err(db_err)?;
        n
    };
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers rulesets (locales au module binaire)
// ─────────────────────────────────────────────────────────────────────────────

/// Insère les items d'un ruleset dans l'ordre donné.
fn insert_ruleset_items(
    con: &Connection,
    ruleset_code: &str,
    items: &[RulesetItemIn],
) -> Result<(), AppError> {
    for item in items {
        con.execute(
            "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) \
             VALUES ((SELECT id FROM dim_ruleset WHERE code = ?), ?, ?)",
            params_from_iter(vec![
                DbValue::Text(ruleset_code.to_string()),
                DbValue::BigInt(item.ordre),
                DbValue::Text(item.rule_code.clone()),
            ]),
        )
        .map_err(db_err)?;
    }
    Ok(())
}

/// Reconstruit un `RulesetDetail` depuis la base (après insert/update).
///
/// Renvoie `AppError::not_found` si le ruleset n'existe pas.
fn build_ruleset_detail(con: &Connection, code: &str) -> Result<RulesetDetail, AppError> {
    let header = {
        let mut stmt = con
            .prepare("SELECT code, libelle FROM dim_ruleset WHERE code = ?")
            .map_err(db_err)?;
        let mut iter = stmt
            .query_map([code], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(db_err)?;
        iter.next()
            .transpose()
            .map_err(db_err)?
            .ok_or_else(|| AppError::not_found(format!("ruleset {code} introuvable")))?
    };
    let mut stmt = con
        .prepare(
            "SELECT i.ordre, i.rule_code, r.libelle \
             FROM dim_ruleset_item i \
             LEFT JOIN dim_rule r ON r.code = i.rule_code \
             WHERE i.ruleset_code = (SELECT id FROM dim_ruleset WHERE code = ?) \
             ORDER BY i.ordre",
        )
        .map_err(db_err)?;
    let iter = stmt
        .query_map([code], |row| {
            Ok(RulesetItemOut {
                ordre: row.get(0)?,
                rule_code: row.get(1)?,
                libelle: row.get(2)?,
            })
        })
        .map_err(db_err)?;
    let mut items = Vec::new();
    for r in iter {
        items.push(r.map_err(db_err)?);
    }
    Ok(RulesetDetail {
        code: header.0,
        libelle: header.1,
        items,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
//  Démarrage
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        std::process::exit(0);
    }
    if let Err(msg) = validate_args(&args[1..]) {
        eprintln!("conso-server: {msg}");
        eprintln!();
        eprintln!("Usage: conso-server [--help]");
        eprintln!("Essayez 'conso-server --help' pour plus d'informations.");
        std::process::exit(2);
    }

    // --- Configuration via env (pas de clap pour un prototype) ---
    let port: u16 = std::env::var("CONSO_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    let db_path = std::env::var("CONSO_DB_PATH").unwrap_or_else(|_| "conso.duckdb".to_string());
    let csv_dir = std::env::var("CONSO_CSV_DIR").unwrap_or_else(|_| "data".to_string());
    let seed_json = std::env::var("CONSO_SEED_JSON").ok().filter(|s| !s.is_empty());
    let web_dir = std::env::var("CONSO_WEB_DIR").unwrap_or_else(|_| "../../web/dist".to_string());

    println!("▶ Ouverture de DuckDB ({db_path})…");
    let con = Connection::open(&db_path)
        .unwrap_or_else(|e| panic!("✗ Impossible d'ouvrir DuckDB ({db_path}) : {e}"));

    // Schéma + chargement initial des CSV.
    //
    // IMPORTANT : on ne recharge les CSV que si la base n'est pas déjà
    // initialisée. Sinon, `create_schema` (DROP de toutes les tables) +
    // `load_all` effaceraient à chaque démarrage les éditions de master data
    // faites via l'UI (périmètre, taux, entités…). La base DuckDB est ainsi la
    // source de vérité entre redémarrages.
    //
    // Pour forcer un rechargement complet (ex. après évolution du schéma) :
    //   - POST /api/reset (à chaud), ou
    //   - CONSO_FORCE_RESEED=1 au démarrage.
    let force_reseed = std::env::var("CONSO_FORCE_RESEED").unwrap_or_default() == "1";
    let schema_exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = 'fact_entry'",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    if schema_exists && !force_reseed {
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM fact_entry", [], |r| r.get(0))
            .unwrap_or(0);
        println!(
            "   Base déjà initialisée ({n} lignes dans fact_entry) — CSV non réimportés, éditions UI préservées."
        );
        println!("   (Pour forcer le rechargement : POST /api/reset ou CONSO_FORCE_RESEED=1)");
        // Migration idempotente : ajoute la colonne `native` au registre des
        // références directes (introduite après les premières bases) et peuple
        // les FK natives (account.sous_classe, entity.entite_parent, …).
        if let Err(e) = custom_references::migrate_native(&con) {
            eprintln!("   ⚠ migrate_native (non bloquant) : {e}");
        }
        // Migration idempotente : garantit la table `dim_coefficient` + les
        // coefficients natifs sur une base existante (volet formules, Q43), sans
        // reset. Sinon GET /api/coefficients et les règles qui les référencent
        // échoueraient (table absente).
        if let Err(e) = coefficients::ensure_schema(&con) {
            eprintln!("   ⚠ ensure_schema coefficients (non bloquant) : {e}");
        }
        // Idem volet 2 (indicateurs) : tables dim_aggregate / dim_indicator.
        if let Err(e) = indicators::ensure_schema(&con) {
            eprintln!("   ⚠ ensure_schema indicateurs (non bloquant) : {e}");
        }
        // Migration in-place (chantier B1, étape 1) : dote chaque dimension d'un
        // `id` technique en backfillant les lignes existantes, sans reset ni
        // perte des objets présents.
        if let Err(e) = conso_engine::surrogate::ensure_ids(&con) {
            eprintln!("   ⚠ ensure_ids surrogate (non bloquant) : {e}");
        }
        // Migration in-place (chantier B1, étape 3) : reconstruit dim_consolidation
        // en convertissant ses FK à contrat code (phase/perimeter_set/variant/
        // rate_set) du TEXT vers la clé technique (id), sur une base existante.
        // Idempotent ; à exécuter après ensure_ids.
        if let Err(e) = conso_engine::surrogate::migrate_consolidation_fk_to_id(&con) {
            eprintln!("   ⚠ migrate_consolidation_fk_to_id (non bloquant) : {e}");
        }
        // 2ᵉ vague B1 : exercice / presentation_currency / perimeter_period /
        // rate_period / ruleset_code → INTEGER. À exécuter après la 1ʳᵉ vague.
        if let Err(e) = conso_engine::surrogate::migrate_consolidation_fk_to_id_v2(&con) {
            eprintln!("   ⚠ migrate_consolidation_fk_to_id_v2 (non bloquant) : {e}");
        }
        // B1 dim_entity : devise_fonctionnelle / entite_parent → INTEGER.
        if let Err(e) = conso_engine::surrogate::migrate_entity_fk_to_id(&con) {
            eprintln!("   ⚠ migrate_entity_fk_to_id (non bloquant) : {e}");
        }
        // Idem pour sat_exchange_rate.rate_set (PK → reconstruction de table).
        // Rend la dimension `rate_set` entièrement flippée (renommable).
        if let Err(e) = conso_engine::surrogate::migrate_sat_exchange_rate_fk_to_id(&con) {
            eprintln!("   ⚠ migrate_sat_exchange_rate_fk_to_id (non bloquant) : {e}");
        }
        // Idem pour sat_perimeter.perimeter_set (PK → reconstruction). Rend la
        // dimension `perimeter_set` entièrement flippée (renommable).
        if let Err(e) = conso_engine::surrogate::migrate_sat_perimeter_fk_to_id(&con) {
            eprintln!("   ⚠ migrate_sat_perimeter_fk_to_id (non bloquant) : {e}");
        }
        // Q44 : colonne `sens` sur dim_sous_classe (user-driven) + backfill des
        // codes natifs (retire le dur SENS_CASE des rapports).
        if let Err(e) = conso_engine::surrogate::ensure_sous_classe_sens(&con) {
            eprintln!("   ⚠ ensure_sous_classe_sens (non bloquant) : {e}");
        }
        // B1 : bascule dim_account.sous_classe du code vers l'id (FK native).
        if let Err(e) = conso_engine::surrogate::migrate_account_sous_classe_to_id(&con) {
            eprintln!("   ⚠ migrate_account_sous_classe_to_id (non bloquant) : {e}");
        }
        // B1 : bascule dim_account.flow_scheme du code vers l'id (FK native).
        // Rend la dimension `flow_scheme` entièrement flippée (renommable) avec
        // sat_flow_scheme_item.scheme ci-dessous. Q45 : flow_scheme est 100 %
        // user-driven (plus de défaut dérivé de la classe).
        if let Err(e) = conso_engine::surrogate::migrate_account_flow_scheme_to_id(&con) {
            eprintln!("   ⚠ migrate_account_flow_scheme_to_id (non bloquant) : {e}");
        }
        // B1 : bascule sat_flow_scheme_item.scheme (PK → reconstruction) du code
        // vers l'id. Seconde réf. vers dim_flow_scheme → dimension renommable.
        if let Err(e) =
            conso_engine::surrogate::migrate_sat_flow_scheme_item_scheme_to_id(&con)
        {
            eprintln!("   ⚠ migrate_sat_flow_scheme_item_scheme_to_id (non bloquant) : {e}");
        }
        // B1 : bascule sat_perimeter.methode (colonne hors PK → add+update+drop+rename)
        // du code TEXT vers l'id INTEGER. Rend dim_method entièrement flippée (renommable).
        if let Err(e) = conso_engine::surrogate::migrate_sat_perimeter_methode_to_id(&con) {
            eprintln!("   ⚠ migrate_sat_perimeter_methode_to_id (non bloquant) : {e}");
        }
        // B1 : bascule dim_ruleset_item.ruleset_code (PK composite → reconstruction)
        // du code TEXT vers l'id INTEGER. Rend dim_ruleset entièrement flippée (renommable).
        if let Err(e) = conso_engine::surrogate::migrate_ruleset_item_fk_to_id(&con) {
            eprintln!("   ⚠ migrate_ruleset_item_fk_to_id (non bloquant) : {e}");
        }
        // B1 étape 4 : bascule les 10 colonnes dimensionnelles de fact_entry
        // du code TEXT vers l'id INTEGER. Met aussi à jour la vue v_flow_behavior.
        // fact_entry est une donnée dérivée → troncature + reconstruction ; le
        // pipeline doit être relancé après cette migration.
        if let Err(e) = conso_engine::surrogate::migrate_fact_entry_to_ids(&con) {
            eprintln!("   ⚠ migrate_fact_entry_to_ids (non bloquant) : {e}");
        }
        // B1 étape 5 : renomme les tables de valeurs car_<code>→car_<id>
        // et lst_<code>→lst_<id>. Idempotent ; nécessite ensure_ids (déjà appelé).
        if let Err(e) = conso_engine::surrogate::ensure_characteristic_attribute_ids(&con) {
            eprintln!("   ⚠ ensure_characteristic_attribute_ids (non bloquant) : {e}");
        }
        if let Err(e) = conso_engine::surrogate::migrate_characteristic_tables_to_id(&con) {
            eprintln!("   ⚠ migrate_characteristic_tables_to_id (non bloquant) : {e}");
        }
        if let Err(e) = conso_engine::surrogate::migrate_value_list_tables_to_id(&con) {
            eprintln!("   ⚠ migrate_value_list_tables_to_id (non bloquant) : {e}");
        }
        // B1 étape 6 : bascule les valeurs JSON (règles / postes / indicateurs)
        // de codes strings vers ids entiers. Idempotent.
        if let Err(e) = conso_engine::json_migration::migrate_json_to_ids(&con) {
            eprintln!("   ⚠ migrate_json_to_ids (non bloquant) : {e}");
        }
        // B1 étape 8 : bascule app_config.pivot_currency (code TEXT) →
        // pivot_currency_id (INTEGER id). Immunise la devise pivot au renommage.
        if let Err(e) = conso_engine::surrogate::migrate_pivot_currency_to_id(&con) {
            eprintln!("   ⚠ migrate_pivot_currency_to_id (non bloquant) : {e}");
        }
        // B1 étape 9 : colonnes physiques N2 <attr_name> → c<attr_id>.
        if let Err(e) = conso_engine::surrogate::migrate_attribute_columns_to_id(&con) {
            eprintln!("   ⚠ migrate_attribute_columns_to_id (non bloquant) : {e}");
        }
        // B1 étape 10 : colonnes custom <name> → x<id> sur fact_entry/stg_entry.
        if let Err(e) = conso_engine::surrogate::migrate_custom_dimension_columns_to_id(&con) {
            eprintln!("   ⚠ migrate_custom_dimension_columns_to_id (non bloquant) : {e}");
        }
        // B1 étape 11 : id + r<id> sur dim_custom_reference et ses colonnes hôtes.
        if let Err(e) = conso_engine::surrogate::ensure_custom_reference_ids(&con) {
            eprintln!("   ⚠ ensure_custom_reference_ids (non bloquant) : {e}");
        }
        if let Err(e) = conso_engine::surrogate::migrate_custom_reference_columns_to_id(&con) {
            eprintln!("   ⚠ migrate_custom_reference_columns_to_id (non bloquant) : {e}");
        }
        // B1 étape 13 : PK id réelle sur les dimensions (reconstruction tables).
        if let Err(e) = conso_engine::surrogate::migrate_dims_pk_to_id(&con) {
            eprintln!("   ⚠ migrate_dims_pk_to_id (non bloquant) : {e}");
        }
        // §11 : colonne target_dimension sur dim_custom_dimension.
        if let Err(e) = conso_engine::dimensions::migrate_custom_dimension_target(&con) {
            eprintln!("   ⚠ migrate_custom_dimension_target (non bloquant) : {e}");
        }
        // Marqueur de version de schéma (idempotent).
        let _ = con.execute(
            "INSERT INTO app_config (key, value) VALUES ('schema_version', '12') \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [],
        );
        // Contrôles de données exemples (idempotent : INSERT OR IGNORE).
        if let Err(e) = seed_demo_controls(&con) {
            eprintln!("   ⚠ seed_demo_controls (non bloquant) : {e}");
        }
    } else {
        if force_reseed {
            println!("   CONSO_FORCE_RESEED=1 — réinitialisation complète demandée.");
        }
        // Boot sur base vierge (ou CONSO_FORCE_RESEED=1) : on construit le
        // schéma, puis — si CONSO_SEED_JSON est défini — on y importe le paquet
        // JSON. Sinon, base vide : l'utilisateur enchaîne avec
        // POST /api/import/all. Le pipeline initial n'est lancé que si le JSON
        // a peuplé au moins une consolidation 'ouvert'.
        //
        // Les seed_demo_* (règles/contrôles/caractéristiques démo) ne sont plus
        // appelés au boot : le JSON fait foi (T3, choix « JSON-only »). Si le
        // JSON ne contient pas ces tables, elles restent vides — c'est attendu.
        create_schema(&con).expect("✗ create_schema");
        if let Some(json_path) = &seed_json {
            println!("   CONSO_SEED_JSON={json_path} — import du paquet…");
            let raw = std::fs::read_to_string(json_path).unwrap_or_else(|e| {
                panic!("✗ Impossible de lire CONSO_SEED_JSON ({json_path}) : {e}")
            });
            let bundle: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!("✗ CONSO_SEED_JSON ({json_path}) n'est pas un JSON valide : {e}")
            });
            let excluded = std::collections::HashSet::<&str>::new();
            let counts = export::import_bundle(&con, &bundle, &excluded)
                .expect("✗ import_bundle (CONSO_SEED_JSON)");
            let total: usize = counts
                .values()
                .filter_map(|v| v.as_u64().map(|n| n as usize))
                .sum();
            println!("   Import JSON : {total} lignes restaurées sur {} tables.", counts.len());
        } else {
            println!("   CONSO_SEED_JSON non défini — schéma seul (base vide).");
            println!("   (Pour bootstrapper : POST /api/import/all avec un paquet JSON)");
        }

        // Pipeline initial pour exposer des données exploitables dès le démarrage.
        // En cas d'échec, on continue : l'utilisateur peut POST /api/run.
        // On sélectionne la 1ère consolidation 'ouvert' (les params viennent de
        // dim_consolidation).
        let initial_consolidation: Option<i64> = con
            .query_row(
                "SELECT id FROM dim_consolidation \
                 WHERE statut = 'ouvert' OR statut IS NULL \
                 ORDER BY id LIMIT 1",
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok();
        match initial_consolidation {
            Some(id) => match ConvertParams::load_params(&con, id) {
                Ok(params) => match run_pipeline(&con, &params) {
                    Ok(report) => {
                        let counts = report.counts();
                        println!(
                            "   Pipeline initial (consolidation {id}) : corporate={}, converted={}, consolidated={}",
                            counts[0], counts[1], counts[2]
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "⚠ Pipeline initial échoué (le serveur démarre quand même) : {e}"
                        );
                    }
                },
                Err(e) => {
                    eprintln!(
                        "⚠ load_params initial échoué pour la consolidation {id} (le serveur démarre quand même) : {e}"
                    );
                }
            },
            None => {
                eprintln!("⚠ Aucune consolidation 'ouvert' trouvée — pipeline initial sauté.");
            }
        }
    }

    // Force un CHECKPOINT : flushe le DDL des migrations / de l'init dans le
    // fichier .duckdb et vide le WAL. Sans cela, un arrêt non propre (Ctrl+C,
    // kill) laisserait dans le WAL des opérations DDL (ex. ALTER … SET DEFAULT
    // nextval) que DuckDB ne sait pas toujours rejouer → base illisible au
    // redémarrage (« GetDefaultDatabase … »). Non bloquant.
    if let Err(e) = con.execute("CHECKPOINT", []) {
        eprintln!("   ⚠ CHECKPOINT au démarrage (non bloquant) : {e}");
    }

    let state = Arc::new(AppState {
        con: Mutex::new(con),
        csv_dir,
        seed_json,
    });

    // CORS permissif pour le prototype : autorise le frontend React (Vite,
    // localhost:5173) et tout autre origine. À restreindre en production.
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    // Servir le frontend buildé en statique (SPA : fallback sur index.html pour
    // toutes les routes non-API). Si le répertoire n'existe pas, seule l'API reste
    // exposée — utile en dev (Vite sert le frontend sur :5173 avec proxy /api).
    let serve_dir =
        ServeDir::new(&web_dir).not_found_service(ServeFile::new(format!("{web_dir}/index.html")));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/levels", get(get_levels))
        .route("/api/bilan", get(get_bilan))
        .route("/api/compte-resultat", get(get_compte_resultat))
        .route(
            "/api/entries",
            get(get_entries).post(entries::create_entries),
        )
        .route(
            "/api/entries/{id}",
            put(entries::update_entry).delete(entries::delete_entry),
        )
        .route("/api/run", post(run_pipeline_handler))
        .route("/api/reset", post(reset_handler))
        .route("/api/consolidations", get(list_consolidations))
        // Dimensions — registre central (built-in + custom)
        .route(
            "/api/meta/dimensions",
            get(list_dimensions).post(create_dimension),
        )
        .route("/api/meta/dimensions/{name}", delete(delete_dimension))
        .route(
            "/api/meta/dimensions/{name}/rename",
            post(rename_dimension),
        )
        // Règles de consolidation (CRUD). L'exécution des règles passe par le
        // pipeline (/api/run applique le ruleset du scénario), pas par une route
        // standalone.
        .route("/api/rules", get(list_rules).post(create_rule))
        .route(
            "/api/rules/{code}",
            get(get_rule).put(update_rule).delete(delete_rule),
        )
        .route("/api/rulesets", get(list_rulesets).post(create_ruleset))
        .route(
            "/api/rulesets/{code}",
            get(get_ruleset).put(update_ruleset).delete(delete_ruleset),
        )
        .merge(masterdata::router())
        .merge(characteristics::router())
        .merge(custom_references::router())
        .merge(value_lists::router())
        .merge(coefficients::router())
        .merge(indicators::router())
        .merge(controls::router())
        .merge(import::router())
        .merge(export::router())
        .fallback_service(serve_dir)
        .layer(cors)
        .with_state(state);

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .unwrap_or_else(|e| panic!("✗ bind 0.0.0.0:{port} : {e}"));
    println!(
        "▶ conso-server en écoute sur http://localhost:{port} (frontend servi depuis {web_dir})"
    );
    axum::serve(listener, app).await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
//  Aide (--help / -h) et validation des arguments
// ─────────────────────────────────────────────────────────────────────────────

fn print_help() {
    println!(
        "conso-server — Serveur HTTP exposant le moteur de consolidation via une API REST.

Sert aussi le frontend React buildé (SPA) si CONSO_WEB_DIR existe.

Au démarrage : si la base est déjà initialisée, elle est conservée telle quelle
(éditions UI préservées). Sinon, schéma seul (base vide), puis — si
CONSO_SEED_JSON est défini — import du paquet JSON.

USAGE
    conso-server [--help]

VARIABLES D'ENVIRONNEMENT
    CONSO_PORT          Port d'écoute (défaut : 3000)
    CONSO_DB_PATH       Fichier DuckDB (défaut : conso.duckdb)
    CONSO_SEED_JSON     Paquet JSON à importer au boot sur base vierge / au reset
                        (défaut : non défini → base vide)
    CONSO_CSV_DIR       Répertoire des CSV — legacy, utilisé par conso-engine CLI
                        (défaut : data)
    CONSO_WEB_DIR       Répertoire du frontend buildé (défaut : ../../web/dist)
    CONSO_FORCE_RESEED  1 = forcer la réinitialisation au démarrage (DROP +
                        import JSON si CONSO_SEED_JSON, sinon base vide)

EXEMPLE
    CONSO_SEED_JSON=tests/fixtures/seed.json conso-server"
    );
}

fn validate_args(args: &[String]) -> Result<(), String> {
    for a in args {
        if a == "-h" || a == "--help" {
            // déjà traité avant l'appel
        } else {
            return Err(format!("argument inconnu : '{a}'"));
        }
    }
    Ok(())
}
