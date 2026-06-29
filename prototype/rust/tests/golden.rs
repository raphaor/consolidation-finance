//! Tests **golden** — filet de régression pour la migration « codes renommables »
//! (option B1, cf. `docs/PLAN_RENOMMAGE_CODES.md`).
//!
//! Principe : on fige la sortie consolidée **exprimée en termes métier**
//! (codes + montants), pas en représentation physique. Quand `fact_entry`
//! passera des codes aux `id` (étape 4 du plan), seule la **projection**
//! [`BUSINESS_SELECT`] changera (passage en JOINs pour ré-résoudre les codes) :
//! le contenu attendu (`tests/snapshots/*.tsv`) doit rester **identique au
//! centime près**. C'est l'invariant qui garantit un pipeline iso-résultat à
//! travers toute la migration.
//!
//! ## Régénérer un snapshot
//! Après un changement **intentionnel** de la sortie, régénérer avec :
//! ```bash
//! UPDATE_SNAPSHOTS=1 cargo test --release --test golden
//! ```
//! puis **relire le diff** avant de committer (un snapshot qui bouge sans raison
//! métier = régression).

use conso_engine::{create_schema, pipeline::run_pipeline, seed_all, ConvertParams};
use duckdb::Connection;
use std::path::PathBuf;

/// Projection **métier** de `fact_entry` : une ligne = un fait, résolu en codes.
///
/// ⚠️ Point de bascule B1 : aujourd'hui les colonnes (`entity`, `account`, …)
/// **sont** les codes (PK textuelles). À l'étape 4, elles deviendront des `id` ;
/// il faudra alors remplacer ces colonnes par des `JOIN dim_x ON x.id = f.x_id`
/// projetant `x.code`. Le `SELECT` ci-dessous est le **seul** endroit à adapter ;
/// l'ordre des colonnes et le format des montants doivent rester identiques.
// B1 étape 4 : fact_entry stocke les 10 dims en INTEGER ids → JOINs pour
// re-projeter les codes métier. L'ordre et le format restent identiques.
const BUSINESS_SELECT: &str = "\
SELECT
    f.level,
    ph.code                          AS phase,
    de.code                          AS entity,
    ep.code                          AS entry_period,
    p.code                           AS period,
    da.code                          AS account,
    df.code                          AS flow,
    cu.code_iso                      AS currency,
    n.code                           AS nature,
    COALESCE(par.code, '')           AS partner,
    COALESCE(sh.code, '')            AS share,
    COALESCE(f.analysis, '')         AS analysis,
    COALESCE(f.analysis2, '')        AS analysis2,
    CAST(f.amount AS VARCHAR)        AS amount
FROM fact_entry f
JOIN dim_scenario_category ph ON ph.id = f.phase
JOIN dim_entity de             ON de.id = f.entity
JOIN dim_period ep             ON ep.id = f.entry_period
JOIN dim_period p              ON p.id  = f.period
JOIN dim_account da            ON da.id = f.account
JOIN dim_flow df               ON df.id = f.flow
JOIN dim_currency cu           ON cu.id = f.currency
JOIN dim_nature n              ON n.id  = f.nature
LEFT JOIN dim_entity par       ON par.id = f.partner
LEFT JOIN dim_entity sh        ON sh.id  = f.share
ORDER BY
    f.level, ph.code, de.code, ep.code, p.code, da.code, df.code, cu.code_iso,
    n.code, COALESCE(par.code, ''), COALESCE(sh.code, ''),
    COALESCE(f.analysis, ''), COALESCE(f.analysis2, ''), CAST(f.amount AS VARCHAR)";

/// Colonnes du snapshot, dans l'ordre du `SELECT` (en-tête TSV).
const SNAPSHOT_HEADER: &str = "level\tphase\tentity\tentry_period\tperiod\taccount\tflow\t\
currency\tnature\tpartner\tshare\tanalysis\tanalysis2\tamount";

/// Ouvre une connexion en mémoire, crée le schéma, charge le seed, lance le
/// pipeline pour la consolidation REEL 2024. (Miroir de `pipeline.rs::setup`.)
fn setup() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    let reel_id: i64 = con
        .query_row(
            "SELECT id FROM dim_consolidation \
             WHERE phase = (SELECT id FROM dim_scenario_category WHERE code='REEL') \
               AND exercice = (SELECT id FROM dim_period WHERE code='2024')",
            [],
            |r| r.get(0),
        )
        .expect("REEL consolidation");
    let params = ConvertParams::load_params(&con, reel_id).expect("load_params");
    run_pipeline(&con, &params).expect("run_pipeline");
    con
}

/// Sérialise la projection métier en TSV canonique (en-tête + lignes triées).
fn render_business_tsv(con: &Connection) -> String {
    let mut stmt = con.prepare(BUSINESS_SELECT).expect("prepare BUSINESS_SELECT");
    let ncols = SNAPSHOT_HEADER.split('\t').count();
    let rows: Vec<String> = stmt
        .query_map([], move |row| {
            let mut cells = Vec::with_capacity(ncols);
            for i in 0..ncols {
                // Toutes les colonnes projetées sont textuelles (codes, amount casté).
                let v: Option<String> = row.get(i)?;
                cells.push(v.unwrap_or_default());
            }
            Ok(cells.join("\t"))
        })
        .expect("query_map")
        .map(|r| r.expect("row"))
        .collect();

    let mut out = String::new();
    out.push_str(SNAPSHOT_HEADER);
    out.push('\n');
    for line in rows {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Chemin d'un snapshot (`tests/snapshots/<name>`).
fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join(name)
}

/// Compare `actual` au snapshot `name`. En mode `UPDATE_SNAPSHOTS`, (ré)écrit le
/// fichier. Sinon, échoue avec un diff sur la première ligne divergente.
///
/// La comparaison normalise les fins de ligne (`\r\n` → `\n`) : sous Windows avec
/// `core.autocrlf=true`, le snapshot est checké-out en CRLF alors que le run
/// produit du LF pur — le contenu est identique, seuls les EOL diffèrent.
fn assert_snapshot(name: &str, actual: &str) {
    let normalize = |s: &str| s.replace("\r\n", "\n").replace('\r', "\n");
    let actual = normalize(actual);
    let path = snapshot_path(name);
    if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create snapshots dir");
        std::fs::write(&path, &actual).expect("write snapshot");
        eprintln!("snapshot mis à jour : {}", path.display());
        return;
    }
    let expected = normalize(&std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "snapshot absent : {}\nGénère-le avec : UPDATE_SNAPSHOTS=1 cargo test --release --test golden",
            path.display()
        )
    }));
    if actual == expected {
        return;
    }
    // Diff minimal : première ligne divergente + comptages.
    let a_lines: Vec<&str> = actual.lines().collect();
    let e_lines: Vec<&str> = expected.lines().collect();
    let mut first_diff = None;
    for (i, (a, e)) in a_lines.iter().zip(e_lines.iter()).enumerate() {
        if a != e {
            first_diff = Some((i + 1, *e, *a));
            break;
        }
    }
    let detail = match first_diff {
        Some((ln, e, a)) => format!("première divergence ligne {ln} :\n  attendu : {e}\n  obtenu  : {a}"),
        None => format!(
            "longueurs différentes : attendu {} lignes, obtenu {} lignes",
            e_lines.len(),
            a_lines.len()
        ),
    };
    panic!(
        "snapshot {name} divergent ({} vs {} lignes).\n{detail}\n\
         Si le changement est INTENTIONNEL : UPDATE_SNAPSHOTS=1 cargo test --release --test golden, puis relis le diff.",
        a_lines.len(),
        e_lines.len()
    );
}

/// Snapshot de référence de la sortie consolidée complète sur le seed (3 niveaux).
#[test]
fn golden_fact_entry_seed() {
    let con = setup();
    let actual = render_business_tsv(&con);
    assert_snapshot("fact_entry_seed.tsv", &actual);
}
