//! Tests d'intégration du moteur de règles de consolidation.
//!
//! Ces tests valident l'exécution de `run_ruleset` sur une DuckDB en mémoire
//! (pattern identique à `tests/pipeline.rs`). Ils complètent `rules_test.py`
//! (test black-box HTTP sur le dataset golden) en :
//!
//! - **reproduisant** le scénario d'élimination interco sur le seed Rust
//!   (M/A/B), sans dépendre d'un binaire release ni d'un serveur démarré ;
//! - **couvrant des branches** que l'exemple interco n'exerce pas : coefficient
//!   `constant`, opérateur `IN`, idempotence intra-règle via les snapshots ;
//! - **rendant explicite** la reconstruction des clôtures F99 après règle.
//!
//! # Cohérence avec `rules_test.py`
//!
//! | Assertion Python (rules_test.py)          | Contrepartie Rust (ci-dessous)            |
//! |-------------------------------------------|-------------------------------------------|
//! | 7a.  lignes 2ELI au consolidé             | `run_ruleset_interco_genere_lignes_2eli`  |
//! | 7a'. tag analysis2 = RULE:code:seq        | `tag_analysis2_au_format_rule_code_seq`   |
//! | 7b.  solde interco extourné à 0           | `solde_interco_extourne_a_zero`           |
//! | 7c.  bilan agrégé inchangé                | `solde_interco_extourne_a_zero` (total)   |
//! | —    (idempotence inter-ruleset uniquement)| `idempotence_intra_reglet_via_snapshot`   |
//! | —    (pas de coefficient constant)         | `coefficient_constant_et_multiplicateur`  |
//! | —    (reconstruction F99 implicite)        | `reconstruction_f99_apres_regle`          |

use conso_engine::{
    create_schema,
    pipeline::{materialize_closures::materialize_closures, run_pipeline},
    run_ruleset, seed_all, ConvertParams,
};
use duckdb::Connection;

// ─────────────────────────────────────────────────────────────────────────────
//  Helpers locaux (SQL) — propres à ce fichier de test.
// ─────────────────────────────────────────────────────────────────────────────

/// Ouvre une connexion en mémoire, crée le schéma, charge le seed (groupe
/// M/A/B), lance le pipeline A→B→C→D. Renvoie la connexion dans l'état
/// consolidé (prête pour exécution d'un ruleset).
fn setup() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    let params = ConvertParams::load_params(&con, "REEL").expect("load_params");
    run_pipeline(&con, &params).expect("run_pipeline");
    con
}

/// Tolérance f64 (le moteur stocke en DECIMAL(18,2), la lecture se fait en f64).
const TOL: f64 = 0.01;

/// Insère des écritures interco au niveau `consolidated` (post-pipeline).
///
/// Le seed M/A/B n'a aucune écriture partenaire (toutes `partner = NULL`).
/// Pour exercer une règle d'élimination interco, on injecte 2 lignes au niveau
/// consolidated :
///   - M vend 100 EUR à A sur le compte 700 (F20, nature 0LIASS) ;
///   - A vend  80 EUR à M sur le compte 600 (F20, nature 0LIASS).
///
/// M et A sont toutes deux en méthode `globale` dans le seed → le scope croisé
/// `(entity.methode = globale) AND (partner.methode = globale)` les sélectionne.
fn seed_interco(con: &Connection) {
    con.execute_batch(
        "INSERT INTO fact_entry \
            (scenario, entity, entry_period, period, account, flow, currency, \
             nature, partner, share, analysis, analysis2, level, amount) \
         VALUES \
            ('REEL','M','2024','2024','700','F20','EUR','0LIASS','A',NULL,NULL,'S-M-INT','consolidated','100.00'), \
            ('REEL','A','2024','2024','600','F20','EUR','0LIASS','M',NULL,NULL,'S-A-INT','consolidated','80.00');",
    )
    .expect("seed_interco");
    // Les F20 interco ont été injectés après le pipeline : les F99 du niveau
    // consolidated ne les intègrent pas encore. On reconstruit les clôtures
    // pour partir d'un état cohérent (sinon la règle, en déclenchant
    // `materialize_closures`, ferait apparaître un Δ artificiel = montant des
    // F20 injectés, en plus de son propre effet).
    materialize_closures(con, "consolidated").expect("materialize_closures post-seed_interco");
}

/// Crée une règle dans `dim_rule`.
fn create_rule(con: &Connection, code: &str, libelle: &str, definition_json: &str) {
    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
        duckdb::params![code, libelle, definition_json],
    )
    .unwrap_or_else(|e| panic!("create_rule({code}) : {e}"));
}

/// Crée un ruleset à un seul item (ordre 1) référençant une règle.
fn create_ruleset_one(con: &Connection, rs_code: &str, libelle: &str, rule_code: &str) {
    con.execute(
        "INSERT INTO dim_ruleset (code, libelle) VALUES (?, ?)",
        duckdb::params![rs_code, libelle],
    )
    .unwrap_or_else(|e| panic!("create_ruleset({rs_code}) : {e}"));
    con.execute(
        "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) VALUES (?, 1, ?)",
        duckdb::params![rs_code, rule_code],
    )
    .unwrap_or_else(|e| panic!("create_ruleset_item({rs_code}) : {e}"));
}

/// SUM(amount) au niveau consolidated avec un filtre SQL libre (appliqué après
/// `level='consolidated'`). Renvoie 0.0 si aucune ligne.
fn sum_consol(con: &Connection, filter: &str) -> f64 {
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0) FROM fact_entry \
         WHERE level='consolidated' AND {filter}"
    );
    con.query_row(&sql, [], |r| r.get::<_, f64>(0))
        .unwrap_or_else(|e| panic!("sum_consol({filter}) : {e}"))
}

/// Nombre de lignes au niveau consolidated avec un filtre SQL libre.
fn count_consol(con: &Connection, filter: &str) -> i64 {
    let sql = format!(
        "SELECT COUNT(*) FROM fact_entry \
         WHERE level='consolidated' AND {filter}"
    );
    con.query_row(&sql, [], |r| r.get::<_, i64>(0))
        .unwrap_or_else(|e| panic!("count_consol({filter}) : {e}"))
}

/// Définition JSON d'une règle d'élimination interco à 2 opérations sur le
/// compte 700 (extourne + contrepartie). Plus courte que la règle 4-op du test
/// Python (qui couvre 700 + 600) — suffisante pour valider la mécanique.
fn elim_700_json() -> &'static str {
    r#"{
        "scope": [
            {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
            {"target": "partner", "dim": "methode", "op": "=", "val": "globale"}
        ],
        "operations": [
            {
                "seq": 1, "level": "consolidated",
                "selection": [
                    {"dim": "account", "op": "=", "val": "700"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "pct_integration"},
                "multiplicateur": -1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "inherit"}
                }
            },
            {
                "seq": 2, "level": "consolidated",
                "selection": [
                    {"dim": "account", "op": "=", "val": "700"},
                    {"dim": "partner", "op": "IS NOT NULL"}
                ],
                "coefficient": {"type": "pct_integration"},
                "multiplicateur": 1,
                "destination": {
                    "nature":  {"mode": "override", "value": "2ELI"},
                    "partner": {"mode": "null"}
                }
            }
        ]
    }"#
}

// ─────────────────────────────────────────────────────────────────────────────
//  1. Exécution d'un ruleset d'élimination interco (2 ops sur 700)
//     — miroir de rules_test.py §7a/7c sur le seed Rust.
//
//     La règle sélectionne les lignes interco (partner NOT NULL) du compte 700
//     au niveau consolidated, extourne (op 1, partner hérité) et pose une
//     contrepartie (op 2, partner vidé). Toutes deux en nature 2ELI.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn run_ruleset_interco_genere_lignes_2eli() {
    let con = setup();
    seed_interco(&con);
    create_rule(&con, "ELI_700", "Élimination interco 700", elim_700_json());
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "ELI_700");

    // Avant règle : pas de 2ELI au consolidé.
    assert_eq!(count_consol(&con, "nature='2ELI'"), 0, "baseline sans 2ELI");

    let report = run_ruleset(&con, "RS_TEST").expect("run_ruleset");
    assert_eq!(report.ruleset, "RS_TEST");
    assert!(report.total_generated > 0, "doit générer des lignes : {report:?}");
    // Exactement 2 lignes générées (1 par opération, 1 ligne source sur 700/M→A).
    assert_eq!(
        report.total_generated, 2,
        "2 ops × 1 ligne source = 2 lignes générées, eu {}",
        report.total_generated
    );

    // Après règle : des lignes 2ELI sont présentes.
    let n_2eli = count_consol(&con, "nature='2ELI'");
    assert!(n_2eli >= 2, "au moins 2 lignes 2ELI (extourne + contrepartie), eu {n_2eli}");
}

// ─────────────────────────────────────────────────────────────────────────────
//  2. Tag analysis2 = "RULE:<code>:<seq>" sur les lignes générées
//     — miroir de rules_test.py §7a'.
//
//     Les F99 reconstruits par materialize_closures portent analysis2=NULL
//     (le tag n'est appliqué qu'aux lignes directement générées par la règle).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn tag_analysis2_au_format_rule_code_seq() {
    let con = setup();
    seed_interco(&con);
    create_rule(&con, "ELI_700", "Élimination interco 700", elim_700_json());
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "ELI_700");

    run_ruleset(&con, "RS_TEST").expect("run_ruleset");

    // Toutes les lignes 2ELI non-F99 portent analysis2 = 'RULE:ELI_700:<seq>'.
    let bad: Vec<(String, Option<String>)> = con
        .prepare(
            "SELECT flow, analysis2 FROM fact_entry \
             WHERE level='consolidated' AND nature='2ELI' AND flow <> 'F99'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|(_, a2)| !matches!(a2, Some(s) if s.starts_with("RULE:ELI_700:")))
        .collect();
    assert!(bad.is_empty(), "analysis2 non conformes (doivent commencer par 'RULE:ELI_700:') : {bad:?}");

    // Les F99 2ELI reconstruits portent analysis2=NULL (régénérés par materialize).
    let bad_f99: i64 = con
        .query_row(
            "SELECT COUNT(*) FROM fact_entry \
             WHERE level='consolidated' AND nature='2ELI' AND flow='F99' \
               AND analysis2 IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bad_f99, 0, "les F99 2ELI reconstruits doivent avoir analysis2=NULL");
}

// ─────────────────────────────────────────────────────────────────────────────
//  3. Solde interco extourné à 0 — miroir de rules_test.py §7b/7c.
//
//     Après exécution, la somme des montants au niveau consolidated avec
//     partner IS NOT NULL doit être ≈ 0 (extourne). Le total consolidated
//     (toutes lignes) doit être inchangé (équilibre : pour chaque -X généré,
//     un +X est aussi généré).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn solde_interco_extourne_a_zero() {
    let con = setup();
    seed_interco(&con);

    let total_before = sum_consol(&con, "1=1");
    let interco_before = sum_consol(&con, "partner IS NOT NULL");
    // Interco injectée : 100 (M→A) + 80 (A→M) = 180. La règle 700 ne couvre que
    // le compte 700 → seule l'interco 100 (M→A) sera extournée. On le vérifie
    // ci-dessous (solde 700/M partner NOT NULL = 0).
    assert!((interco_before - 180.0).abs() < TOL, "baseline interco = 180, eu {interco_before}");

    create_rule(&con, "ELI_700", "Élimination interco 700", elim_700_json());
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "ELI_700");
    run_ruleset(&con, "RS_TEST").expect("run_ruleset");

    // Solde 700 partner NOT NULL extourné à 0 (op 1 extourne les 100 initiaux).
    let interco_700 = sum_consol(&con, "account='700' AND partner IS NOT NULL");
    assert!(
        interco_700.abs() < TOL,
        "solde interco 700 (partner NOT NULL) doit être ~0 après extourne, eu {interco_700}"
    );

    // Total consolidated inchangé (extourne −100 + contrepartie +100 = 0 net).
    let total_after = sum_consol(&con, "1=1");
    assert!(
        (total_after - total_before).abs() < TOL,
        "total consolidated doit être inchangé par la règle (équilibrée) : avant={total_before}, après={total_after}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  4. Idempotence intra-règle via snapshot — non couvert par rules_test.py.
//
//     Une règle avec 2 opérations identiques (même sélection) sur le même
//     niveau doit générer exactement 2×N lignes (N = lignes matchant la
//     sélection à l'état initial), pas 3×N (cas d'une cascade sans snapshot,
//     où l'op 2 lirait les écritures générées par l'op 1).
//
//     Le moteur crée un `CREATE TEMP TABLE _rule_snap_<level>` avant l'exécution
//     des opérations : toutes les opérations lisent le snapshot, jamais l'état
//     en cours de modification.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn idempotence_intra_reglet_via_snapshot() {
    let con = setup();
    seed_interco(&con);

    // Règle à 2 opérations identiques (seq 1 et 2) : copie à l'identique de
    // l'interco 700 (coefficient 1, multiplicateur 1, nature surchargée).
    let json = r#"{
        "scope": [
            {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
            {"target": "partner", "dim": "methode", "op": "=", "val": "globale"}
        ],
        "operations": [
            {
                "seq": 1, "level": "consolidated",
                "selection": [{"dim": "account", "op": "=", "val": "700"},
                              {"dim": "partner", "op": "IS NOT NULL"}],
                "destination": {"nature": {"mode": "override", "value": "2CPY"}}
            },
            {
                "seq": 2, "level": "consolidated",
                "selection": [{"dim": "account", "op": "=", "val": "700"},
                              {"dim": "partner", "op": "IS NOT NULL"}],
                "destination": {"nature": {"mode": "override", "value": "2CPY"}}
            }
        ]
    }"#;
    create_rule(&con, "CPY_2OPS", "Copie 2 ops identiques", json);
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "CPY_2OPS");

    let report = run_ruleset(&con, "RS_TEST").expect("run_ruleset");

    // La sélection matche exactement 1 ligne source (M→A 700 100 EUR).
    // Avec snapshot : chaque op génère 1 ligne → total = 2.
    // Sans snapshot (cascade intra-règle) : op 1 génère 1 ligne, op 2 lirait
    // l'originale + la générée de l'op 1 → 2 lignes → total = 3.
    assert_eq!(
        report.total_generated, 2,
        "snapshot doit isoler les opérations : attendu 2 lignes (2 ops × 1 source), eu {}",
        report.total_generated
    );

    // Vérification par requête : exactement 2 lignes 2CPY non-F99 sur 700.
    let n = count_consol(&con, "nature='2CPY' AND flow<>'F99'");
    assert_eq!(n, 2, "2 lignes 2CPY générées (hors F99 reconstruits), eu {n}");
}

// ─────────────────────────────────────────────────────────────────────────────
//  5. Coefficient constant × multiplicateur — branche non couverte par
//     l'exemple interco (qui utilise pct_integration = 1.0 sur le seed).
//
//     Une opération coefficient=Constant(0.5), multiplicateur=2 sur une ligne
//     source de montant 100 doit produire une ligne de montant 100 × 0.5 × 2 = 100.
//     Une opération coefficient=Constant(0.25), multiplicateur=-1 produit
//     100 × 0.25 × -1 = -25.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn coefficient_constant_et_multiplicateur() {
    let con = setup();
    seed_interco(&con);

    // Deux opérations indépendantes sur la même sélection (interco 700/M→A = 100).
    let json = r#"{
        "operations": [
            {
                "seq": 1, "level": "consolidated",
                "selection": [{"dim": "account", "op": "=", "val": "700"},
                              {"dim": "partner", "op": "IS NOT NULL"}],
                "coefficient": {"type": "constant", "value": 0.5},
                "multiplicateur": 2,
                "destination": {"nature": {"mode": "override", "value": "2A"}}
            },
            {
                "seq": 2, "level": "consolidated",
                "selection": [{"dim": "account", "op": "=", "val": "700"},
                              {"dim": "partner", "op": "IS NOT NULL"}],
                "coefficient": {"type": "constant", "value": 0.25},
                "multiplicateur": -1,
                "destination": {"nature": {"mode": "override", "value": "2B"}}
            }
        ]
    }"#;
    create_rule(&con, "CST", "Coefficients constants", json);
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "CST");

    let report = run_ruleset(&con, "RS_TEST").expect("run_ruleset");
    assert_eq!(report.total_generated, 2, "2 ops × 1 source = 2 lignes");

    // Op 1 : 100 × 0.5 × 2 = 100.
    let amt_a = sum_consol(&con, "nature='2A' AND flow<>'F99'");
    assert!(
        (amt_a - 100.0).abs() < TOL,
        "op 1 : 100 × 0.5 × 2 doit donner 100, eu {amt_a}"
    );
    // Op 2 : 100 × 0.25 × -1 = -25.
    let amt_b = sum_consol(&con, "nature='2B' AND flow<>'F99'");
    assert!(
        (amt_b - (-25.0)).abs() < TOL,
        "op 2 : 100 × 0.25 × -1 doit donner -25, eu {amt_b}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  6. Reconstruction F99 après règle — explicite ce que rules_test.py vérifie
//     indirectement (via l'absence de fuite).
//
//     Une règle génère un flux constitutif (F20) en nature 2CLO au niveau
//     consolidated. Après `materialize_closures`, le F99 correspondant doit
//     être reconstruit à ce même grain (scenario, entity, entry_period,
//     period, account, currency, nature) en intégrant le flux généré.
//
//     Concrètement : F99@700/M/2CLO = 100 (le F20 2CLO généré), car la nature
//     est une dimension `Active` qui entre dans le grain de reconstruction.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn reconstruction_f99_apres_regle() {
    let con = setup();
    seed_interco(&con);

    // Avant règle : aucun F99 en nature 2CLO au niveau consolidated.
    let f99_2clo_before = sum_consol(&con, "nature='2CLO' AND flow='F99'");
    assert!(f99_2clo_before.abs() < TOL, "baseline : pas de F99 2CLO");

    // Règle : copie l'interco 700 en F20 nature 2CLO partner vidé (1 ligne, +100).
    let json = r#"{
        "operations": [
            {
                "seq": 1, "level": "consolidated",
                "selection": [{"dim": "account", "op": "=", "val": "700"},
                              {"dim": "partner", "op": "IS NOT NULL"}],
                "destination": {
                    "nature":  {"mode": "override", "value": "2CLO"},
                    "partner": {"mode": "null"}
                }
            }
        ]
    }"#;
    create_rule(&con, "CLO", "Génère F20 2CLO", json);
    create_ruleset_one(&con, "RS_TEST", "Ruleset test", "CLO");

    run_ruleset(&con, "RS_TEST").expect("run_ruleset");

    // Le F20 2CLO a bien été généré (partner NULL, account 700).
    let f20_2clo = sum_consol(&con, "nature='2CLO' AND flow='F20'");
    assert!(
        (f20_2clo - 100.0).abs() < TOL,
        "F20 2CLO généré = 100, eu {f20_2clo}"
    );

    // Le F99 2CLO a été reconstruit par materialize_closures après la règle.
    // Puisque la nature est Active (entre dans le grain), F99@2CLO est un grain
    // distinct de F99@0LIASS, et sa valeur = Σ(F20 2CLO) = 100.
    let f99_2clo_after = sum_consol(&con, "nature='2CLO' AND flow='F99'");
    assert!(
        (f99_2clo_after - 100.0).abs() < TOL,
        "F99 2CLO reconstruit après règle = 100 (intégration du F20 généré), eu {f99_2clo_after}"
    );
}
