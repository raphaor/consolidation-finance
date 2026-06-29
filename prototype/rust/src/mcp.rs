//! Serveur MCP (Model Context Protocol) intégré — mode `conso-server --mcp` (Q54).
//!
//! Expose le moteur de consolidation comme un ensemble d'**outils typés** qu'un
//! agent IA (opencode, Claude, etc.) peut découvrir et invoquer via stdio. Le
//! cœur métier est partagé avec le serveur HTTP (`conso_engine::reports`,
//! `masterdata`, `import`, `indicators`, `controls`) : aucun round-trip HTTP.
//!
//! Cf. [`docs/PLAN_Q54_API_MCP.md`](../../docs/PLAN_Q54_API_MCP.md) §4.

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::{
    handler::server::wrapper::Parameters, schemars::JsonSchema, tool, tool_router,
    ErrorData as McpError, ServiceExt,
};
use serde::Deserialize;

use crate::state::{lock_con, AppError, AppState};

/// Convertit une `AppError` (HTTP) en erreur MCP. Le code HTTP est conservé
/// dans le message pour le diagnostic ; les erreurs client (4xx) deviennent des
/// `invalid_params`, les autres des `internal_error`.
fn mcp_err(e: AppError) -> McpError {
    let msg = format!("[{}] {}", e.0.as_u16(), e.1);
    if e.0.is_client_error() {
        McpError::invalid_params(msg, None)
    } else {
        McpError::internal_error(msg, None)
    }
}

/// Sérialise une valeur en JSON compact pour le retour d'outil.
fn to_json<T: serde::Serialize>(v: T) -> Result<String, McpError> {
    serde_json::to_string(&v)
        .map_err(|e| McpError::internal_error(format!("sérialisation : {e}"), None))
}

/// Échappe une valeur JSON en champ CSV (RFC 4180) : entoure de guillemets si
/// nécessaire et double les guillemets internes. Sert à `import_entries` quand
/// l'agent fournit des lignes en JSON.
fn csv_field(v: &serde_json::Value) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    };
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

/// Convertit un tableau JSON d'objets en CSV (header = union ordonnée des clés).
fn json_rows_to_csv(rows: &[serde_json::Value]) -> Result<String, McpError> {
    if rows.is_empty() {
        return Err(McpError::invalid_params(
            "rows_json : tableau vide".to_string(),
            None,
        ));
    }
    let mut keys: Vec<String> = Vec::new();
    for r in rows {
        let o = r.as_object().ok_or_else(|| {
            McpError::invalid_params(
                "rows_json : chaque ligne doit être un objet".to_string(),
                None,
            )
        })?;
        for k in o.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    let mut out = String::new();
    out.push_str(&keys.join(","));
    out.push('\n');
    for r in rows {
        let o = r.as_object().unwrap();
        let vals: Vec<String> = keys
            .iter()
            .map(|k| csv_field(o.get(k).unwrap_or(&serde_json::Value::Null)))
            .collect();
        out.push_str(&vals.join(","));
        out.push('\n');
    }
    Ok(out)
}

// ───────────────────────────── Paramètres d'outils ─────────────────────────────

#[derive(Deserialize, JsonSchema)]
struct ListMasterDataParams {
    #[schemars(description = "Nom d'API de la table (ex: accounts, entities, flows, rates, perimeter, consolidations). Voir describe_model.")]
    table: String,
    #[serde(default)]
    #[schemars(description = "Recherche plein-texte (insensible à la casse) sur le libellé")]
    search: Option<String>,
    #[serde(default)]
    #[schemars(description = "Filtres exacts {colonne: valeur} (ex: {\"classe\":\"bilan\"}). Les FK code-contrat sont résolues.")]
    filters: HashMap<String, String>,
    #[serde(default)]
    #[schemars(description = "Pagination : nombre de lignes (défaut: toutes)")]
    limit: Option<i64>,
    #[serde(default)]
    #[schemars(description = "Pagination : décalage (défaut: 0)")]
    offset: Option<i64>,
    #[serde(default)]
    #[schemars(description = "Ajoute les colonnes {fk}_libelle pour les FK (défaut: false)")]
    enrich: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct UpsertMasterDataParams {
    #[schemars(description = "Nom d'API de la table cible")]
    table: String,
    #[schemars(description = "Tableau JSON d'objets à insérer/mettre à jour (chacun porte ses colonnes, PK incluse pour les tables à PK code ; id optionnel pour les tables auto-PK comme consolidations).")]
    rows_json: String,
}

#[derive(Deserialize, JsonSchema)]
struct ImportEntriesParams {
    #[serde(default)]
    #[schemars(description = "Contenu CSV complet (avec header) des écritures à importer. Colonnes requises: phase,entity,entry_period,period,account,flow,currency,nature,amount.")]
    csv: Option<String>,
    #[serde(default)]
    #[schemars(description = "Alternative à csv : un tableau JSON d'objets (mêmes clés que le CSV). Sera converti en CSV en interne.")]
    rows_json: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GetEntriesParams {
    #[serde(default)]
    #[schemars(description = "Niveau : raw (saisie stg_entry), corporate, converted, consolidated (défaut)")]
    level: Option<String>,
    #[serde(default)]
    consolidation_id: Option<i64>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    #[schemars(description = "Nombre de lignes (défaut: 100)")]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct RunConsolidationParams {
    #[serde(default)]
    #[schemars(description = "id de la consolidation (PK dim_consolidation). Si omis, utilise la 1ère de statut 'ouvert'. Voir describe_model pour la liste.")]
    consolidation_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct RunControlsParams {
    #[serde(default)]
    #[schemars(description = "Code du control-set à exécuter. Si omis, retourne la liste des control-sets disponibles.")]
    set_code: Option<String>,
    #[serde(default)]
    consolidation_id: Option<i64>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ReportParams {
    #[serde(default)]
    consolidation_id: Option<i64>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entry_period: Option<String>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    nature: Option<String>,
    #[serde(default)]
    #[schemars(description = "Niveau fact_entry (défaut: consolidated)")]
    level: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GetIndicatorParams {
    #[serde(default)]
    #[schemars(description = "Code d'un indicateur existant (dim_indicator). Si fourni, son expression et son grain sont utilisés.")]
    code: Option<String>,
    #[serde(default)]
    #[schemars(description = "Expression de formule ad-hoc (ex: 'SAFE_DIV([resultat]; [ca])'). Ignoré si code est fourni.")]
    expression: Option<String>,
    #[schemars(description = "id de la consolidation cible (obligatoire)")]
    consolidation_id: i64,
    #[serde(default)]
    #[schemars(description = "Grain de restitution (tableau de dimensions, ex: [\"entity\"]). Défaut: celui de l'indicateur ou vide.")]
    grain: Option<Vec<String>>,
}

fn report_query(p: &ReportParams) -> crate::reports::BilanQuery {
    crate::reports::BilanQuery {
        level: p.level.clone().unwrap_or_else(|| "consolidated".into()),
        consolidation: p.consolidation_id,
        entity: p.entity.clone(),
        entry_period: p.entry_period.clone(),
        period: p.period.clone(),
        nature: p.nature.clone(),
    }
}

// ───────────────────────────── Serveur ─────────────────────────────────────────

/// Serveur MCP de consolidation. Porte l'`AppState` (connexion DuckDB partagée)
/// et expose un outil par cas d'usage agent (saisie, run conso, contrôles,
/// rapports bilan/P&L, indicateurs, lecture master data).
#[derive(Clone)]
pub struct ConsoMcp {
    state: Arc<AppState>,
}

#[tool_router(server_handler)]
impl ConsoMcp {
    // ── Modèle & master data ────────────────────────────────────────────────

    /// Outil de premier appel : décrit le modèle (tables master data, champs de
    /// saisie, catalogue de codes, consolidations) pour que l'agent construise
    /// des écritures valides sans tâtonner.
    #[tool(description = "Décrit le modèle de données de consolidation : tables master data navigables (avec colonnes et PK), champs de saisie stg_entry, catalogue de codes (flux, natures, classes, devises, méthodes, phases, échantillons entités/périodes) et consolidations existantes. À appeler en premier pour comprendre comment saisir des écritures valides et cibler une consolidation.")]
    fn describe_model(&self) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        to_json(crate::reports::describe_model(&con).map_err(mcp_err)?)
    }

    /// Lecture master data paginée/filtrée/recherchée.
    #[tool(description = "Liste les lignes d'une table master data (accounts, entities, flows, periods, currencies, rates, perimeter, consolidations, etc.). Supporte la recherche plein-texte (search), les filtres exacts par colonne, la pagination (limit/offset) et l'enrichissement ({fk}_libelle). Retourne un objet {total, rows}.")]
    fn list_master_data(
        &self,
        Parameters(p): Parameters<ListMasterDataParams>,
    ) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let (rows, total) = crate::masterdata::md_list(
            &con,
            &p.table,
            crate::masterdata::MdListOptions {
                search: p.search,
                filters: p.filters,
                limit: p.limit,
                offset: p.offset.unwrap_or(0),
                enrich: p.enrich.unwrap_or(false),
            },
        )
        .map_err(mcp_err)?;
        to_json(serde_json::json!({ "total": total, "rows": rows }))
    }

    /// Upsert master data en masse (insert-or-update par PK).
    #[tool(description = "Insère ou met à jour (upsert) en masse des lignes d'une table master data. Fournir rows_json = un tableau JSON d'objets (PK incluse). Validation all-or-nothing (champs inconnus + intégrité référentielle) puis transaction. Retourne {inserted, updated}.")]
    fn upsert_master_data(
        &self,
        Parameters(p): Parameters<UpsertMasterDataParams>,
    ) -> Result<String, McpError> {
        let rows: Vec<serde_json::Value> = serde_json::from_str(&p.rows_json).map_err(|e| {
            McpError::invalid_params(
                format!("rows_json n'est pas un tableau JSON valide : {e}"),
                None,
            )
        })?;
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let (inserted, updated) =
            crate::masterdata::md_bulk_upsert(&con, &p.table, rows).map_err(mcp_err)?;
        to_json(serde_json::json!({ "inserted": inserted, "updated": updated }))
    }

    /// Import d'écritures (append dans stg_entry).
    #[tool(description = "Importe des écritures de saisie (append dans stg_entry). Fournir csv (texte CSV avec header) ou rows_json (tableau JSON d'objets). Valide le header + la cohérence référentielle avant insertion. Retourne {imported}. Après import, lancer run_consolidation pour matérialiser le pipeline.")]
    fn import_entries(
        &self,
        Parameters(p): Parameters<ImportEntriesParams>,
    ) -> Result<String, McpError> {
        let csv = match (p.csv, p.rows_json) {
            (Some(c), _) if !c.trim().is_empty() => c,
            (_, Some(rows)) => {
                let parsed: Vec<serde_json::Value> = serde_json::from_str(&rows).map_err(|e| {
                    McpError::invalid_params(
                        format!("rows_json n'est pas un tableau JSON valide : {e}"),
                        None,
                    )
                })?;
                json_rows_to_csv(&parsed)?
            }
            _ => {
                return Err(McpError::invalid_params(
                    "fournir 'csv' ou 'rows_json'".to_string(),
                    None,
                ))
            }
        };
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let imported =
            crate::import::import_entries_csv(&con, csv.as_bytes()).map_err(mcp_err)?;
        to_json(serde_json::json!({ "imported": imported }))
    }

    /// Lecture des écritures (raw ou fact_entry).
    #[tool(description = "Lit les écritures : niveau 'raw' (saisie stg_entry) ou un niveau fact_entry (corporate/converted/consolidated). Filtres par consolidation, entité, phase, période, nature, source. Pagination limit/offset. Retourne un tableau de lignes.")]
    fn get_entries(
        &self,
        Parameters(p): Parameters<GetEntriesParams>,
    ) -> Result<String, McpError> {
        let q = crate::reports::EntriesQuery {
            level: p.level.unwrap_or_else(|| "consolidated".into()),
            limit: p.limit.unwrap_or(100),
            offset: p.offset.unwrap_or(0),
            consolidation: p.consolidation_id,
            phase: p.phase,
            entity: p.entity,
            entry_period: p.entry_period,
            period: p.period,
            nature: p.nature,
            source: p.source,
        };
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let rows = crate::reports::get_entries(&con, &q).map_err(mcp_err)?;
        to_json(rows)
    }

    // ── Exécution ───────────────────────────────────────────────────────────

    /// Lance le pipeline de consolidation (3 étapes + ruleset + contrôle à-nouveau).
    #[tool(description = "Déclenche le pipeline de consolidation (agrégation → conversion → consolidation) sur une consolidation, puis le ruleset référencé et le contrôle de cohérence à-nouveau. consolidation_id optionnel (défaut : 1ère 'ouvert'). Retourne {corporate, converted, consolidated, consolidation, ruleset, ruleset_report, a_nouveau_warnings}.")]
    fn run_consolidation(
        &self,
        Parameters(p): Parameters<RunConsolidationParams>,
    ) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let res = crate::reports::run_consolidation(&con, p.consolidation_id).map_err(mcp_err)?;
        to_json(res)
    }

    /// Exécute un control-set (ou liste les disponibles).
    #[tool(description = "Exécute un jeu de contrôles de données (control-set) sur une consolidation/phase et retourne le rapport (statut Pass/Warn/Error/NoData par contrôle et niveau). Si set_code est omis, retourne la liste des control-sets disponibles pour découverte.")]
    fn run_controls(
        &self,
        Parameters(p): Parameters<RunControlsParams>,
    ) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        match p.set_code {
            Some(code) => {
                let params = crate::controls::RunParams {
                    consolidation_id: p.consolidation_id,
                    phase: p.phase,
                    entry_period: p.entry_period,
                };
                let report =
                    crate::controls::run_control_set(&con, &code, &params).map_err(|e| {
                        McpError::internal_error(format!("exécution contrôles : {e}"), None)
                    })?;
                to_json(report)
            }
            None => {
                let rows = crate::masterdata::run_query(
                    &con,
                    "SELECT code, libelle FROM dim_control_set ORDER BY code",
                    Vec::new(),
                )
                .map_err(mcp_err)?;
                to_json(serde_json::json!({ "control_sets": rows, "hint": "Précisez set_code pour exécuter un jeu." }))
            }
        }
    }

    // ── Rapports ────────────────────────────────────────────────────────────

    /// Bilan par flux (comptes de classe bilan).
    #[tool(description = "Bilan consolidé par flux : montants agrégés par (compte, flux, nature) pour les comptes de classe 'bilan' (actif/passif/capitaux propres). Filtres par consolidation, entité, périodes, nature. Retourne un tableau de {account, flow, nature, sens, amount}.")]
    fn get_bilan(&self, Parameters(p): Parameters<ReportParams>) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let rows = crate::reports::get_bilan(&con, &report_query(&p)).map_err(mcp_err)?;
        to_json(rows)
    }

    /// Compte de résultat par flux.
    #[tool(description = "Compte de résultat consolidé par flux : montants agrégés pour les comptes de classe 'resultat' (produits/charges). Mêmes filtres que get_bilan.")]
    fn get_compte_resultat(
        &self,
        Parameters(p): Parameters<ReportParams>,
    ) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let rows = crate::reports::get_compte_resultat(&con, &report_query(&p)).map_err(mcp_err)?;
        to_json(rows)
    }

    // ── Indicateurs ─────────────────────────────────────────────────────────

    /// Calcule un indicateur (code existant ou formule ad-hoc).
    #[tool(description = "Calcule un indicateur/KPI sur une consolidation. Fournir 'code' (indicateur existant) ou 'expression' (formule ad-hoc). consolidation_id obligatoire. grain optionnel (regroupement par dimensions). Retourne un tableau de {grain, value}.")]
    fn get_indicator(
        &self,
        Parameters(p): Parameters<GetIndicatorParams>,
    ) -> Result<String, McpError> {
        let con = lock_con(&self.state).map_err(mcp_err)?;
        let (expression, grain) = match &p.code {
            Some(code) => {
                let row = crate::masterdata::run_query(
                    &con,
                    "SELECT expression, grain FROM dim_indicator WHERE code = ?",
                    vec![duckdb::types::Value::Text(code.clone())],
                )
                .map_err(mcp_err)?;
                let row = row.into_iter().next().ok_or_else(|| {
                    McpError::invalid_params(
                        format!("indicateur '{code}' introuvable"),
                        None,
                    )
                })?;
                let expr = row
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let stored_grain: Vec<String> = row
                    .get("grain")
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                (expr, p.grain.unwrap_or(stored_grain))
            }
            None => {
                let expr = p.expression.clone().ok_or_else(|| {
                    McpError::invalid_params(
                        "fournir 'code' ou 'expression'".to_string(),
                        None,
                    )
                })?;
                (expr, p.grain.unwrap_or_default())
            }
        };
        let rows = crate::indicators::run_indicator(&con, &expression, &grain, p.consolidation_id)
            .map_err(|e| McpError::internal_error(format!("indicateur : {e}"), None))?;
        to_json(rows)
    }
}

/// Lance le serveur MCP sur stdin/stdout. Bloque jusqu'à déconnexion du client
/// (opencode). Appelé par `conso-server --mcp` (cf. `src/bin/server.rs`).
pub async fn run_stdio(state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error>> {
    let service = ConsoMcp { state }
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
