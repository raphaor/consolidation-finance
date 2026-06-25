//! Pipeline de consolidation en 3 étapes.
//!
//! Chaque étape lit un niveau de stockage et produit le suivant. L'ordre A→C→D
//! correspond à la correspondance stockage ↔ traitement décrite dans
//! `docs/FLUX_CONSO.md` :
//!
//! ```text
//! A. Agrégation      stg_entry        → fact_entry [corporate]
//! C. Conversion      corporate        → fact_entry [converted]
//! D. Consolidation   converted        → fact_entry [consolidated]
//! ```
//!
//! L'étape B (reclassification de périmètre) a été **supprimée** : les
//! traitements de périmètre (F00→F01, miroir F98) passent par des règles au
//! niveau corporate (cf. docs/A_NOUVEAU.md §4). Le niveau `reclassified`
//! n'existe plus.
//!
//! Toute la logique est exprimée en SQL déclaratif (portage Rust direct via
//! duckdb-rs : une passe SQL par règle métier).

pub mod a_nouveau;
pub mod aggregate;
pub mod consolidate;
pub mod convert;
pub mod materialize_closures;
pub mod staging;

use duckdb::Connection;
use std::time::Instant;

/// Comptage des lignes par niveau de stockage après le pipeline.
///
/// 3 niveaux depuis la suppression de `reclassified` (cf. docs/A_NOUVEAU.md §4) :
/// `[corporate, converted, consolidated]`.
pub type LevelCounts = [usize; 3];

/// Paramètres d'un run de pipeline : une **consolidation** (plus un scénario).
///
/// Ces paramètres ne sont **plus** constructibles via `Default` : ils dépendent
/// de la consolidation choisie et de la config applicative. Utiliser
/// [`ConvertParams::load_params`] pour les hydrater depuis la base.
///
/// - `consolidation_id` : isole les résultats du run dans `fact_entry`.
/// - `phase` + `exercice` : sélectionnent la **remontée** dans `stg_entry`
///   (`WHERE phase = ? AND entry_period = ?`).
/// - `perimeter_set` + `perimeter_period` : périmètre explicite (ex-`entry_period`
///   implicite). `rate_set` + `rate_period` : table de taux explicite.
/// - `pivot_currency` : lu depuis `app_config.pivot_currency` (singleton
///   d'instance — invariant pour toute la durée de vie d'une base).
/// - Le taux N-1 (`close_n1`) vient de `sat_exchange_rate.taux_ouverture` porté
///   par `rate_period` — aucune période antérieure requise.
///
/// Cf. `docs/MODELE_DONNEES.md` (consolidation v3).
#[derive(Debug, Clone)]
pub struct ConvertParams {
    /// PK technique de la consolidation (isolation dans `fact_entry`).
    pub consolidation_id: i64,
    /// Phase de la remontée (ex `'REEL'`) — filtre `stg_entry.phase`.
    pub phase: String,
    /// Exercice N (filtre `stg_entry.entry_period` ; période d'ouverture/à-nouveau).
    pub exercice: String,
    /// Devise de présentation (cible de la conversion).
    pub presentation_currency: String,
    /// Devise pivot applicative (tous les taux stockés convertissent vers elle).
    pub pivot_currency: String,
    /// Jeu de périmètre du run (clé dans `sat_perimeter`).
    pub perimeter_set: String,
    /// Période du périmètre (défaut = exercice).
    pub perimeter_period: String,
    /// Jeu de taux à utiliser (clé dans `sat_exchange_rate`).
    pub rate_set: String,
    /// Période des taux (close_n / avg / ouverture) — défaut = exercice.
    pub rate_period: String,
    /// Consolidation d'à-nouveau (conso N-1 figée dont on reporte l'ouverture).
    /// `None` = pas d'à-nouveau (cf. docs/A_NOUVEAU.md §2.2 / §6).
    pub a_nouveau_consolidation_id: Option<i64>,
}

impl ConvertParams {
    /// Charge les paramètres d'un run depuis `dim_consolidation` + `app_config`.
    ///
    /// Identifie la consolidation par sa PK technique `id`. Le pivot par défaut
    /// est `'EUR'` si `app_config` est vide (robustesse — mais le seed
    /// l'insère toujours). Aucune période N-1 n'est requise : le taux `close_n1`
    /// est lu via `sat_exchange_rate.taux_ouverture` porté par `rate_period`.
    pub fn load_params(con: &duckdb::Connection, consolidation_id: i64) -> duckdb::Result<Self> {
        let (
            phase,
            exercice,
            presentation_currency,
            perimeter_set,
            perimeter_period,
            rate_set,
            rate_period,
            a_nouveau_consolidation_id,
        ): (String, String, String, String, String, String, String, Option<i64>) = con.query_row(
            // phase / perimeter_set / rate_set sont stockés en clé technique (id,
            // chantier B1) : résolus id→code par JOIN pour que le pipeline reste
            // code-based (jointures sur satellites en codes inchangées).
            "SELECT sc.code, c.exercice, c.presentation_currency,
                    ps.code, c.perimeter_period,
                    rs.code, c.rate_period,
                    c.a_nouveau_consolidation_id
             FROM dim_consolidation c
             LEFT JOIN dim_scenario_category sc ON sc.id = c.phase
             LEFT JOIN dim_perimeter_set ps ON ps.id = c.perimeter_set
             LEFT JOIN dim_rate_set rs ON rs.id = c.rate_set
             WHERE c.id = ?",
            [consolidation_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                ))
            },
        )?;

        let pivot_currency: String = con.query_row(
            "SELECT COALESCE((SELECT value FROM app_config WHERE key = 'pivot_currency'), 'EUR')",
            [],
            |r| r.get::<_, String>(0),
        )?;

        Ok(Self {
            consolidation_id,
            phase,
            exercice,
            presentation_currency,
            pivot_currency,
            perimeter_set,
            perimeter_period,
            rate_set,
            rate_period,
            a_nouveau_consolidation_id,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Trait Step — uniformisation des étapes du pipeline
// ─────────────────────────────────────────────────────────────────────────────

/// Un trait pour unifier les étapes du pipeline (A→C→D).
///
/// Chaque étape : (1) exécute sa transformation principale ([`Step::run`]),
/// (2) injecte d'éventuels flux de staging (préfixe 2/3/4), (3) reconstruit
/// les clôtures. L'orchestration commune est dans [`run_steps`].
///
/// Le SQL réellement exécuté est strictement le même qu'avant le refactor :
/// les impls délèguent aux fonctions `step_a`/`step_c`/`step_d`
/// existantes, dans le même ordre.
pub trait Step: Send + Sync {
    /// Nom court pour les logs ("agrégation", "reclassification"…).
    fn name(&self) -> &'static str;
    /// Niveau produit en sortie ("corporate", "converted"…).
    fn output_level(&self) -> &'static str;
    /// Préfixe de staging injecté après cette étape ("2", "3", "4", ou "" si aucun).
    fn staging_prefix(&self) -> &'static str {
        ""
    }
    /// Exécute la transformation principale (sans staging ni clôtures).
    fn run(&self, con: &Connection, params: &ConvertParams) -> duckdb::Result<()>;
}

/// Étape A — agrégation `stg_entry` → `fact_entry [corporate]`.
pub struct AggregateStep;
impl Step for AggregateStep {
    fn name(&self) -> &'static str {
        "agrégation"
    }
    fn output_level(&self) -> &'static str {
        "corporate"
    }
    fn run(&self, con: &Connection, params: &ConvertParams) -> duckdb::Result<()> {
        aggregate::step_a(con, params).map(|_| ())
    }
}

/// Étape C — conversion multi-devises `corporate` → `fact_entry [converted]`.
pub struct ConvertStep;
impl Step for ConvertStep {
    fn name(&self) -> &'static str {
        "conversion"
    }
    fn output_level(&self) -> &'static str {
        "converted"
    }
    // Pas de staging post-étape : le préfixe 2 est consommé DANS step_c (UNION,
    // en devise fonctionnelle, pour subir conversion + écarts). Cf. step_c.
    fn run(&self, con: &Connection, params: &ConvertParams) -> duckdb::Result<()> {
        convert::step_c(con, params).map(|_| ())
    }
}

/// Étape D — consolidation `converted` → `fact_entry [consolidated]`.
pub struct ConsolidateStep;
impl Step for ConsolidateStep {
    fn name(&self) -> &'static str {
        "consolidation"
    }
    fn output_level(&self) -> &'static str {
        "consolidated"
    }
    // Préfixe 4 = injection post-étape au consolidé (APRÈS le × pct, tel quel).
    // Le préfixe 3 (AVANT le × pct) est consommé DANS step_d (UNION). Cf. step_d.
    fn staging_prefix(&self) -> &'static str {
        "4"
    }
    fn run(&self, con: &Connection, params: &ConvertParams) -> duckdb::Result<()> {
        consolidate::step_d(con, params).map(|_| ())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Rapport d'exécution
// ─────────────────────────────────────────────────────────────────────────────

/// Temps d'exécution mesuré pour une étape du pipeline.
#[derive(Debug, Clone)]
pub struct StepTiming {
    /// Niveau de stockage produit (`corporate`, `converted`, …).
    pub level: &'static str,
    /// Nombre de lignes produites à ce niveau.
    pub rows: usize,
    /// Durée de l'étape, en millisecondes.
    pub ms: f64,
}

/// Rapport d'exécution du pipeline avec timings par étape.
#[derive(Debug, Clone)]
pub struct PipelineReport {
    /// Une entrée par étape, dans l'ordre A→C→D (B/reclassification supprimée).
    pub steps: [StepTiming; 3],
    /// Durée totale A→D (wall-clock), en millisecondes.
    pub total_ms: f64,
}

impl PipelineReport {
    /// Nombre de lignes par niveau `[corporate, converted, consolidated]`.
    pub fn counts(&self) -> LevelCounts {
        [self.steps[0].rows, self.steps[1].rows, self.steps[2].rows]
    }

    /// Durée totale en secondes.
    pub fn total_sec(&self) -> f64 {
        self.total_ms / 1000.0
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Orchestration
// ─────────────────────────────────────────────────────────────────────────────

/// Enchaîne les étapes fournies et renvoie le rapport d'exécution (avec timings
/// par étape).
///
/// Pour chaque étape :
/// 1. Exécute la transformation principale ([`Step::run`]).
/// 2. Si [`Step::staging_prefix`] est non vide : injection des flux de staging
///    ([`staging::inject_by_prefix`]).
/// 3. Reconstruction autoritaire des clôtures
///    ([`materialize_closures::materialize_closures`]) **après chaque niveau**,
///    corporate inclus (devenu un point de traitement).
/// 4. Comptage des lignes produites + mesure du temps écoulé.
///
/// Le tableau renvoyé contient toujours exactement 3 entrées (les 3 étapes
/// du pipeline A→C→D) — `try_into` panic sinon (cf. [`run_pipeline`]).
fn run_steps(
    con: &Connection,
    params: &ConvertParams,
    steps: &[Box<dyn Step>],
    after_level: &mut dyn FnMut(&Connection, &str) -> duckdb::Result<()>,
) -> duckdb::Result<PipelineReport> {
    let wall = Instant::now();
    let mut timings: Vec<StepTiming> = Vec::with_capacity(steps.len());
    for step in steps {
        let t = Instant::now();
        step.run(con, params)?;
        if !step.staging_prefix().is_empty() {
            staging::inject_by_prefix(con, params, step.output_level(), step.staging_prefix())?;
        }
        // À-nouveau : colle le solde de clôture du snapshot N-1 sur le flux
        // d'ouverture (F00) au niveau produit (corporate / consolidated), AVANT la
        // reconstruction des clôtures. No-op si le scénario n'a pas d'à-nouveau.
        // Cf. docs/A_NOUVEAU.md §3.
        a_nouveau::carry(con, params, step.output_level())?;
        // Reconstruction autoritaire des clôtures après CHAQUE niveau — y compris
        // `corporate`, qui devient un point de traitement (injection à-nouveau +
        // règles de périmètre, cf. docs/A_NOUVEAU.md §4). Avant la suppression de
        // l'étape B, les clôtures n'apparaissaient qu'à partir de `reclassified`.
        materialize_closures::materialize_closures(con, step.output_level())?;
        // Hook post-niveau : permet d'injecter des écritures (ex. règles de
        // consolidation au niveau produit) AVANT que l'étape suivante ne
        // consomme ce niveau. Une règle au niveau `converted` est ainsi
        // propagée vers `consolidated` par l'étape D, comme une écriture
        // manuelle. Sans hook (pipeline natif seul), c'est un no-op.
        after_level(con, step.output_level())?;
        let rows = count_level(con, step.output_level())?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        timings.push(StepTiming {
            level: step.output_level(),
            rows,
            ms,
        });
    }
    let total_ms = wall.elapsed().as_secs_f64() * 1000.0;
    let steps_arr: [StepTiming; 3] = timings
        .try_into()
        .expect("run_steps attend exactement 3 étapes");
    Ok(PipelineReport {
        steps: steps_arr,
        total_ms,
    })
}

/// Enchaîne les 3 étapes et renvoie le rapport d'exécution (avec timings).
///
/// Ordre A→C→D :
/// ```text
/// A. Agrégation      stg_entry        → fact_entry [corporate]
/// C. Conversion      corporate        → fact_entry [converted]
/// D. Consolidation   converted        → fact_entry [consolidated]
/// ```
///
/// Après chacune des étapes (corporate, converted, consolidated), on matérialise
/// les flux de clôture (flux auto-référentiels de `dim_flow.flux_de_report`) =
/// Σ des flux qui y reportent — en écrasant la clôture portée par l'étape (les
/// clôtures transitent comme n'importe quel flux, puis sont reconstruites de
/// façon autoritaire à chaque niveau). Le validateur [`crate::validate`] compare
/// ensuite la clôture stockée à cette somme (data-driven).
///
/// Pour récupérer uniquement les comptes par niveau : [`PipelineReport::counts`].
pub fn run_pipeline(con: &Connection, params: &ConvertParams) -> duckdb::Result<PipelineReport> {
    run_pipeline_with_hook(con, params, &mut |_con, _level| Ok(()))
}

/// Comme [`run_pipeline`], mais appelle `after_level(con, level)` après chaque
/// étape (une fois ses clôtures matérialisées) et avant l'étape suivante.
///
/// Sert à intercaler les **règles de consolidation** au bon niveau : le serveur
/// passe un hook qui exécute les règles ciblant le niveau produit. Une règle au
/// niveau `converted` est donc injectée juste après l'étape C, puis l'étape D la
/// consolide normalement (propagation identique à une écriture manuelle).
pub fn run_pipeline_with_hook(
    con: &Connection,
    params: &ConvertParams,
    after_level: &mut dyn FnMut(&Connection, &str) -> duckdb::Result<()>,
) -> duckdb::Result<PipelineReport> {
    let steps: Vec<Box<dyn Step>> = vec![
        Box::new(AggregateStep),
        Box::new(ConvertStep),
        Box::new(ConsolidateStep),
    ];
    run_steps(con, params, &steps, after_level)
}

/// Compte les lignes d'un niveau de stockage donné.
fn count_level(con: &Connection, level: &str) -> duckdb::Result<usize> {
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry WHERE level = ?",
        [level],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
