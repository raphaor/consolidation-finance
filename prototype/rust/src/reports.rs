//! Rapports & exécution pipeline — cœur métier partagé entre le serveur HTTP
//! (binaire `conso-server`) et le serveur MCP (Q54). Extraction des handlers
//! Axum pour éviter la duplication HTTP↔MCP : les handlers deviennent des
//! wrappers fins qui extraient la requête, appellent ces fonctions pures
//! (`&Connection` + params) et sérialisent le résultat.

use duckdb::{params_from_iter, types::Value as DbValue, Connection};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::money::Money;
use crate::rules::{run_ruleset_at_level, RuleResult, RulesetReport};
use crate::state::{db_err, AppError};
use crate::validate;
use crate::{dimensions, ConvertParams, run_pipeline, run_pipeline_with_hook};

// ─────────────────────────────────────────────────────────────────────────────
//  DTO
// ─────────────────────────────────────────────────────────────────────────────

/// Ligne de bilan / compte de résultat : montant agrégé par
/// (compte, flux, nature) au niveau demandé. `amount` sérialisé en nombre JSON.
#[derive(Serialize)]
pub struct BilanRow {
    pub account: String,
    pub flow: String,
    pub nature: String,
    /// Sens comptable dérivé de `dim_sous_classe.sens` : `"C"` / `"D"` / `"?"`.
    pub sens: String,
    pub amount: Decimal,
}

/// Avertissement de cohérence de l'à-nouveau (non bloquant).
#[derive(Serialize)]
pub struct CoherenceWarning {
    pub kind: String,
    pub entity: String,
    pub detail: String,
}

/// Réponse d'un run de consolidation : nombre de lignes produites à chaque
/// étape du pipeline + rapport ruleset + avertissements à-nouveau.
#[derive(Serialize)]
pub struct PipelineResult {
    pub corporate: usize,
    pub converted: usize,
    pub consolidated: usize,
    pub consolidation: i64,
    pub ruleset: Option<String>,
    pub ruleset_report: Option<RulesetReport>,
    pub a_nouveau_warnings: Vec<CoherenceWarning>,
}

/// Paramètres de requête partagés par le bilan et le compte de résultat.
#[derive(Deserialize, Default, Clone)]
pub struct BilanQuery {
    #[serde(default = "default_level")]
    pub level: String,
    #[serde(default)]
    pub consolidation: Option<i64>,
    #[serde(default)]
    pub entity: Option<String>,
    #[serde(default)]
    pub entry_period: Option<String>,
    #[serde(default)]
    pub period: Option<String>,
    #[serde(default)]
    pub nature: Option<String>,
}

fn default_level() -> String {
    "consolidated".to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Filtres fact_entry
// ─────────────────────────────────────────────────────────────────────────────

/// Construit le fragment SQL et les paramètres pour les filtres optionnels
/// `consolidation` (PK de la conso, isolation d'un run dans `fact_entry`),
/// `entity`, `entry_period`, `period` et `nature`. Les codes TEXT sont résolus
/// en ids via sous-requêtes (colonnes INTEGER après étape 4 B1). Renvoie une
/// chaîne préfixée par " AND ..." prête à concaténer après un WHERE existant.
pub fn build_filters_fe(
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
//  Bilan & compte de résultat
// ─────────────────────────────────────────────────────────────────────────────

/// Cœur partagé du bilan (`classe = "bilan"`) et du compte de résultat
/// (`classe = "resultat"`). Regroupe par (compte, flux, nature) au niveau
/// demandé, en excluant les lignes « dont » (dimensions analytiques renseignées).
fn report_by_class(
    con: &Connection,
    q: &BilanQuery,
    classe: &str,
) -> Result<Vec<BilanRow>, AppError> {
    let dims = dimensions::load_all(con).map_err(db_err)?;
    let of_which: String = dimensions::analytical_cols(&dims)
        .iter()
        .map(|c| format!(" AND e.{c} IS NULL"))
        .collect();
    let (fsql, fparams) = build_filters_fe(&q.consolidation, &q.entity, &q.entry_period, &q.period, &q.nature);
    let sql = format!(
        "SELECT a.code AS account, df.code AS flow, n.code AS nature,
                COALESCE(sc.sens, '?') AS sens, SUM(e.amount) AS amount
         FROM fact_entry e
         JOIN dim_account a ON a.id = e.account
         JOIN dim_flow df ON df.id = e.flow
         JOIN dim_nature n ON n.id = e.nature
         LEFT JOIN dim_sous_classe sc ON sc.id = a.sous_classe
         WHERE e.level = ? AND a.classe = '{classe}' {fsql}{of_which}
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
    Ok(out)
}

/// Bilan par flux (comptes de classe `bilan`).
pub fn get_bilan(con: &Connection, q: &BilanQuery) -> Result<Vec<BilanRow>, AppError> {
    report_by_class(con, q, "bilan")
}

/// Compte de résultat par flux (comptes de classe `resultat`).
pub fn get_compte_resultat(con: &Connection, q: &BilanQuery) -> Result<Vec<BilanRow>, AppError> {
    report_by_class(con, q, "resultat")
}

// ─────────────────────────────────────────────────────────────────────────────
//  Exécution du pipeline (consolidation)
// ─────────────────────────────────────────────────────────────────────────────

/// Résout l'id de consolidation : explicite si fourni, sinon la 1ère de statut
/// `'ouvert'` (ou NULL). Public pour que le MCP et l'handler HTTP partagent la
/// même règle de défaut.
pub fn resolve_consolidation_id(
    con: &Connection,
    explicit: Option<i64>,
) -> Result<i64, AppError> {
    if let Some(id) = explicit {
        return Ok(id);
    }
    con.query_row(
        "SELECT id FROM dim_consolidation \
         WHERE statut = 'ouvert' OR statut IS NULL \
         ORDER BY id LIMIT 1",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| {
        AppError::bad_request(format!(
            "aucune consolidation 'ouvert' trouvée (précisez consolidation_id) : {e}"
        ))
    })
}

/// Lance le pipeline 3 étapes sur la consolidation `consolidation_id` (si
/// `None`, la 1ère `'ouvert'`), puis exécute le ruleset référencé par la
/// consolidation (intercalé par niveau via un hook), et collecte les
/// avertissements de cohérence de l'à-nouveau. Cœur du `POST /api/run`.
pub fn run_consolidation(
    con: &Connection,
    consolidation_id: Option<i64>,
) -> Result<PipelineResult, AppError> {
    // 1. Résolution de la consolidation.
    let consolidation_id = resolve_consolidation_id(con, consolidation_id)?;

    // 2. Chargement des params depuis dim_consolidation + app_config.
    let params = ConvertParams::load_params(con, consolidation_id).map_err(db_err)?;

    // 3. Lecture du ruleset_code (NULL si la consolidation n'en référence pas).
    let ruleset_code: Option<String> = con
        .query_row(
            "SELECT rs.code FROM dim_consolidation c \
             LEFT JOIN dim_ruleset rs ON rs.id = c.ruleset_code \
             WHERE c.id = ?",
            [consolidation_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .map_err(db_err)?;

    // 4. Vider les résultats du pipeline de la consolidation courante.
    con.execute(
        "DELETE FROM fact_entry WHERE consolidation_id = ?",
        [consolidation_id],
    )
    .map_err(db_err)?;

    // 5. Pipeline. Si ruleset, intercalage de ses règles par niveau (hook).
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
            run_pipeline_with_hook(con, &params, &mut hook)
                .map_err(db_err)?
                .counts()
        }
        None => run_pipeline(con, &params).map_err(db_err)?.counts(),
    };

    // 5b. À-nouveau : contrôle de cohérence non bloquant.
    let mut a_nouveau_warnings: Vec<CoherenceWarning> = Vec::new();
    if let Some(a_nouveau_id) = params.a_nouveau_consolidation_id {
        match validate::check_a_nouveau_coherence(
            con,
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

    Ok(PipelineResult {
        corporate: counts[0],
        converted: counts[1],
        consolidated: counts[2],
        consolidation: consolidation_id,
        ruleset: ruleset_code,
        ruleset_report,
        a_nouveau_warnings,
    })
}
