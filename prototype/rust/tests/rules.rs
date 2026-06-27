//! Tests **mécaniques** de l'interpréteur de règles (`run_ruleset`).
//!
//! Objectif (Sujet 1 — près du moteur) : vérifier que, *étant donné une
//! définition de règle*, l'interpréteur écrit **exactement les bonnes
//! écritures**. On teste la mécanique, pas une politique comptable :
//!
//! - **sélection** : les bonnes lignes sont matchées (`=`, `IN`, `IS NOT NULL`),
//!   et il y a **une ligne de sortie par ligne source** (donc une par flux) ;
//! - **coefficient × multiplicateur** : `montant_sortie = source × coeff × mult`
//!   (constant, `pct_integration` lu depuis `sat_perimeter`) ;
//! - **destination** : `inherit` / `override` / `null` par dimension (compte,
//!   nature, partner) ;
//! - **isolation par snapshot** : une opération ne lit pas la sortie d'une autre ;
//! - **scope** : filtrage via `sat_perimeter` (méthode de l'entité).
//!
//! La justesse comptable d'une règle réelle (ex. élimination interco) relève de
//! la **recette/config** et se teste hors moteur (smoke tests Python).
//!
//! Méthode : on part du schéma + `seed_all` (registre des dimensions, flux,
//! comptes, périmètre M/A/B), on **vide `fact_entry`**, on injecte des lignes
//! contrôlées au niveau voulu, on exécute la règle, et on asserte les sorties.

use conso_engine::{create_schema, run_ruleset, seed_all};
use duckdb::Connection;

const TOL: f64 = 0.001;

/// Connexion prête pour un test mécanique : schéma + dimensions/périmètre seedés,
/// `fact_entry` vidé (on contrôle entièrement les lignes sources).
fn engine() -> Connection {
    let con = Connection::open_in_memory().expect("open_in_memory");
    create_schema(&con).expect("create_schema");
    seed_all(&con).expect("seed_all");
    con.execute("DELETE FROM fact_entry", [])
        .expect("clear fact_entry");
    // Natures synthétiques utilisées comme cibles d'`override` par les règles de
    // test. Sous B1, `fact_entry.nature` est une FK id (NOT NULL) : une valeur
    // d'override doit donc exister dans `dim_nature` (avant, nature était du TEXT
    // libre). On les seede ici (les natures « réelles » 0LIASS/1AJUST viennent du
    // seed).
    con.execute_batch(
        "INSERT INTO dim_nature (code, libelle, rules) VALUES
            ('TST','Test',NULL),('DONT','Dont',NULL),('MAIN','Main',NULL),
            ('2A','Test 2A',NULL),('2B','Test 2B',NULL),('2CPY','Test copie',NULL),
            ('MAPRF','Test map_ref',NULL),('SELN1','Test sélection N1',NULL),
            ('SELRF','Test sélection ref',NULL),('SCACTIF','Test sous-classe actif',NULL),
            ('FSBILAN','Test flow_scheme bilan',NULL);",
    )
    .expect("seed natures de test");
    // Sous B1, les colonnes dimensionnelles de `fact_entry` sont des ids INTEGER.
    // Cette vue rétablit les **codes** pour les assertions des tests (`ssum`/
    // `scount` la ciblent) : les dims à master data sont jointes sur l'id et
    // projetées en code ; `level`/`amount`/`analysis*` restent tels quels.
    con.execute_batch(
        "CREATE VIEW vfe AS
         SELECT f.consolidation_id, f.level, f.amount, f.analysis, f.analysis2,
                de.code  AS entity,
                da.code  AS account,
                dfl.code AS flow,
                dn.code  AS nature,
                dpa.code AS partner,
                dsh.code AS share,
                dc.code_iso AS currency,
                dph.code AS phase,
                dep.code AS entry_period,
                dpe.code AS period
         FROM fact_entry f
         LEFT JOIN dim_entity            de  ON de.id  = f.entity
         LEFT JOIN dim_account           da  ON da.id  = f.account
         LEFT JOIN dim_flow              dfl ON dfl.id = f.flow
         LEFT JOIN dim_nature            dn  ON dn.id  = f.nature
         LEFT JOIN dim_entity            dpa ON dpa.id = f.partner
         LEFT JOIN dim_entity            dsh ON dsh.id = f.share
         LEFT JOIN dim_currency          dc  ON dc.id  = f.currency
         LEFT JOIN dim_scenario_category dph ON dph.id = f.phase
         LEFT JOIN dim_period            dep ON dep.id = f.entry_period
         LEFT JOIN dim_period            dpe ON dpe.id = f.period;",
    )
    .expect("create view vfe");
    con
}

/// Résout l'id d'une consolidation par (phase, exercice).
fn cid(con: &Connection, phase: &str, exercice: &str) -> i64 {
    con.query_row(
        "SELECT id FROM dim_consolidation \
         WHERE phase = (SELECT id FROM dim_scenario_category WHERE code = ?) \
         AND exercice = (SELECT id FROM dim_period WHERE code = ?)",
        [phase, exercice],
        |r| r.get(0),
    )
    .expect("consolidation")
}

/// Injecte une ligne source dans `fact_entry`. Phase/exercice/période/devise
/// fixés (REEL/2024/2024/EUR) pour matcher le périmètre seedé. Le
/// `consolidation_id` pointe vers la consolidation REEL seedée (id déterministe)
/// — nécessaire pour que les JOINs de périmètre (et N-1 via à-nouveau) résolvent
/// le bon `perimeter_set` depuis `dim_consolidation`.
#[allow(clippy::too_many_arguments)]
fn put(
    con: &Connection,
    entity: &str,
    account: &str,
    flow: &str,
    partner: Option<&str>,
    nature: &str,
    amount: f64,
    level: &str,
) {
    con.execute(
        "INSERT INTO fact_entry \
            (consolidation_id, phase, entity, entry_period, period, account, flow, currency, \
             nature, partner, share, analysis, analysis2, level, amount) \
         SELECT (SELECT id FROM dim_consolidation \
                 WHERE phase = (SELECT id FROM dim_scenario_category WHERE code='REEL') \
                   AND exercice = (SELECT id FROM dim_period WHERE code='2024')), \
                (SELECT id FROM dim_scenario_category WHERE code='REEL'), \
                (SELECT id FROM dim_entity WHERE code=?), \
                (SELECT id FROM dim_period WHERE code='2024'), \
                (SELECT id FROM dim_period WHERE code='2024'), \
                (SELECT id FROM dim_account WHERE code=?), \
                (SELECT id FROM dim_flow WHERE code=?), \
                (SELECT id FROM dim_currency WHERE code_iso='EUR'), \
                (SELECT id FROM dim_nature WHERE code=?), \
                (SELECT id FROM dim_entity WHERE code=?), \
                NULL, NULL, NULL, ?, ?",
        duckdb::params![entity, account, flow, nature, partner, level, amount],
    )
    .unwrap_or_else(|e| panic!("put({entity},{account},{flow}): {e}"));
}

/// Crée une règle dans `dim_rule`.
fn create_rule(con: &Connection, code: &str, definition_json: &str) {
    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES (?, ?, ?)",
        duckdb::params![code, code, definition_json],
    )
    .unwrap_or_else(|e| panic!("create_rule({code}): {e}"));
}

/// Crée un ruleset à un seul item (ordre 1) référençant une règle, et l'exécute.
fn run_one(con: &Connection, rule_code: &str) -> usize {
    con.execute(
        "INSERT INTO dim_ruleset (code, libelle) VALUES ('RS', 'rs')",
        [],
    )
    .expect("create ruleset");
    con.execute(
        "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) \
         VALUES ((SELECT id FROM dim_ruleset WHERE code = 'RS'), 1, ?)",
        duckdb::params![rule_code],
    )
    .expect("create ruleset item");
    run_ruleset(con, "RS", None).expect("run_ruleset").total_generated
}

/// SUM(amount) sur `fact_entry` filtré par une clause SQL libre.
fn ssum(con: &Connection, filter: &str) -> f64 {
    // `vfe` = vue code-aware sur `fact_entry` (cf. `engine`) : les filtres des
    // tests citent les codes des dimensions, pas les ids de stockage.
    let sql = format!("SELECT COALESCE(SUM(amount), 0) FROM vfe WHERE {filter}");
    con.query_row(&sql, [], |r| r.get::<_, f64>(0))
        .unwrap_or_else(|e| panic!("ssum({filter}): {e}"))
}

/// COUNT(*) sur `fact_entry` (via la vue `vfe`) filtré.
fn scount(con: &Connection, filter: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM vfe WHERE {filter}");
    con.query_row(&sql, [], |r| r.get::<_, i64>(0))
        .unwrap_or_else(|e| panic!("scount({filter}): {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
//  1. Sélection + multiplicateur : une ligne de sortie, montant négocié, flux
//     et dimensions non surchargées hérités.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn op_genere_une_ligne_par_source_montant_et_heritage() {
    let con = engine();
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":-1,
             "destination":{"nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 1, "1 ligne source matchée → 1 sortie");

    // Sortie : compte 200 (hérité), flux F20 (hérité), partner NULL (hérité),
    // nature TST (override), montant 100 × 1 × (−1) = −100.
    assert!(
        (ssum(&con, "level='converted' AND nature='TST'") - (-100.0)).abs() < TOL,
        "montant = source × coeff × mult"
    );
    assert_eq!(
        scount(
            &con,
            "level='converted' AND nature='TST' AND account='200' AND flow='F20' AND partner IS NULL"
        ),
        1,
        "flux et compte hérités, partner non surchargé reste NULL"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  2. Une sortie PAR ligne source : sans filtre de flux, chaque flux matché
//     produit sa propre sortie (mécanique pure — pas une politique sur F99).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn une_sortie_par_flux_matche() {
    let con = engine();
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted");
    put(&con, "M", "200", "F99", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":-1,
             "destination":{"nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    assert_eq!(
        run_one(&con, "R"),
        2,
        "2 lignes sources (F20, F99) → 2 sorties"
    );

    assert!((ssum(&con, "nature='TST' AND flow='F20'") - (-100.0)).abs() < TOL);
    assert!((ssum(&con, "nature='TST' AND flow='F99'") - (-100.0)).abs() < TOL);
}

// ─────────────────────────────────────────────────────────────────────────────
//  3. Destination override : compte + nature redirigés ; pas de sortie sur le
//     compte source.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn destination_override_compte_et_nature() {
    let con = engine();
    put(&con, "M", "468", "F20", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"468"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"account":{"mode":"override","value":"471L"},
                            "nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 1);

    assert!((ssum(&con, "nature='TST' AND account='471L' AND flow='F20'") - 100.0).abs() < TOL);
    assert_eq!(
        scount(&con, "nature='TST' AND account='468'"),
        0,
        "aucune sortie sur le compte source (compte redirigé)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  4. Destination partner : `inherit` conserve le partenaire, `null` le vide.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn destination_partner_inherit_et_null() {
    let con = engine();
    put(
        &con,
        "M",
        "200",
        "F20",
        Some("A"),
        "0LIASS",
        100.0,
        "converted",
    );

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"DONT"},
                            "partner":{"mode":"inherit"}}},
            {"seq":2,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"MAIN"},
                            "partner":{"mode":"null"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 2);

    assert_eq!(
        scount(&con, "nature='DONT' AND partner='A'"),
        1,
        "partner inherit conserve A"
    );
    assert_eq!(
        scount(&con, "nature='MAIN' AND partner IS NULL"),
        1,
        "partner null vide le partenaire"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  5. Coefficient constant × multiplicateur : montant = source × coeff × mult.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn coefficient_constant_et_multiplicateur() {
    let con = engine();
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":0.5},"multiplicateur":2,
             "destination":{"nature":{"mode":"override","value":"2A"}}},
            {"seq":2,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":0.25},"multiplicateur":-1,
             "destination":{"nature":{"mode":"override","value":"2B"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 2);

    assert!(
        (ssum(&con, "nature='2A'") - 100.0).abs() < TOL,
        "100 × 0,5 × 2 = 100"
    );
    assert!(
        (ssum(&con, "nature='2B'") - (-25.0)).abs() < TOL,
        "100 × 0,25 × −1 = −25"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  6. Coefficient `pct_integration` : lu depuis `sat_perimeter` (ici forcé à 0,5
//     sur B). montant = source × 0,5.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn coefficient_pct_integration_lit_le_perimetre() {
    let con = engine();
    con.execute(
        "UPDATE sat_perimeter SET pct_integration = 0.5 WHERE entity = 'B' AND perimeter_set = (SELECT id FROM dim_perimeter_set WHERE code = 'PERIM_REEL')",
        [],
    )
    .expect("set pct_integration B");
    put(&con, "B", "200", "F20", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"pct_integration"},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 1);

    assert!(
        (ssum(&con, "nature='TST' AND entity='B'") - 50.0).abs() < TOL,
        "100 × pct_integration(0,5) = 50"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  7. Isolation par snapshot : 2 opérations identiques sur la même sélection
//     produisent 2×N lignes (pas 3×N) — l'op 2 lit le snapshot initial, pas la
//     sortie de l'op 1.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn snapshot_isole_les_operations() {
    let con = engine();
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted");

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"2CPY"}}},
            {"seq":2,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"2CPY"}}}]}"#,
    );
    // 1 source × 2 ops = 2 (et non 3 : l'op 2 ne voit pas la sortie de l'op 1).
    assert_eq!(run_one(&con, "R"), 2);
    assert_eq!(scount(&con, "nature='2CPY'"), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
//  8. Sélection `IN` + `IS NOT NULL` : matche le bon sous-ensemble.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selection_in_et_is_not_null() {
    let con = engine();
    put(
        &con,
        "M",
        "200",
        "F20",
        Some("A"),
        "0LIASS",
        100.0,
        "converted",
    ); // match
    put(
        &con,
        "M",
        "300",
        "F20",
        Some("A"),
        "0LIASS",
        100.0,
        "converted",
    ); // hors IN
    put(&con, "M", "705", "F20", None, "0LIASS", 100.0, "converted"); // partner NULL

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"IN","val":["200","705"]},
                          {"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    // Seul 200 (dans IN ET partner non NULL) matche : 705 a partner NULL, 300 hors IN.
    assert_eq!(run_one(&con, "R"), 1);
    assert_eq!(scount(&con, "nature='TST' AND account='200'"), 1);
    assert_eq!(scount(&con, "nature='TST' AND account IN ('300','705')"), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
//  9. Scope : filtrage par la méthode de l'entité (via `sat_perimeter`). Seules
//     les lignes des entités `globale` produisent une sortie.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn scope_filtre_sur_methode_entite() {
    let con = engine();
    con.execute(
        "UPDATE sat_perimeter SET methode = (SELECT id FROM dim_method WHERE code = 'proportionnelle') \
         WHERE entity = 'B' AND perimeter_set = (SELECT id FROM dim_perimeter_set WHERE code = 'PERIM_REEL')",
        [],
    )
    .expect("set methode B");
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted"); // M globale → match
    put(&con, "B", "200", "F20", None, "0LIASS", 100.0, "converted"); // B proportionnelle → exclue

    create_rule(
        &con,
        "R",
        r#"{"scope":[{"target":"entity","dim":"methode","op":"=","val":"globale"}],
            "operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","op":"=","val":"200"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"TST"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 1, "seule l'entité globale produit");
    assert!((ssum(&con, "nature='TST' AND entity='M'") - 100.0).abs() < TOL);
    assert_eq!(scount(&con, "nature='TST' AND entity='B'"), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
//  10. Destination `map` : traversée N1→N2. Les comptes classés par une
//      caractéristique sont redirigés vers ses attributs (compte de liaison +
//      nature). INNER JOIN : un compte non classé ne génère rien. Multi-cible :
//      account ET nature sont mappés depuis la même caractéristique.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn destination_map_traverse_caracteristique() {
    use conso_engine::characteristics::{add_attribute, create_characteristic};

    let con = engine();

    // N1 « comportement » sur les comptes + 2 attributs N2 (vers comptes / natures).
    create_characteristic(&con, "comportement", "Comportement", "account").unwrap();
    add_attribute(
        &con,
        "comportement",
        "compte_destination",
        "Compte de liaison",
        "account",
    )
    .unwrap();
    add_attribute(
        &con,
        "comportement",
        "nat",
        "Nature d'élimination",
        "nature",
    )
    .unwrap();

    // Valeur + affectation en SQL direct (le CRUD est testé côté `characteristics`).
    // Après B1 étape 9, les colonnes physiques sont c{attr_id}, pas les noms d'attributs.
    let char_id = conso_engine::characteristics::id_of(&con, "comportement").unwrap();
    let car_table = conso_engine::characteristics::value_table(char_id);
    let col_cd = conso_engine::characteristics::attr_col_for(&con, "comportement", "compte_destination").unwrap();
    let col_nat = conso_engine::characteristics::attr_col_for(&con, "comportement", "nat").unwrap();
    con.execute(
        &format!(
            "INSERT INTO {car_table} (code, libelle, \"{col_cd}\", \"{col_nat}\") \
             VALUES ('VENTES_IC', 'Ventes interco', '471L', '1AJUST')"
        ),
        [],
    )
    .unwrap();
    con.execute(
        "UPDATE dim_account SET comportement = 'VENTES_IC' WHERE code = '468'",
        [],
    )
    .unwrap();

    put(&con, "M", "468", "F20", None, "0LIASS", 100.0, "converted"); // classé → mappé
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted"); // non classé → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"flow","op":"=","val":"F20"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":-1,
             "destination":{
                "account":{"mode":"map","via":"comportement","attr":"compte_destination"},
                "nature":{"mode":"map","via":"comportement","attr":"nat"}}}]}"#,
    );

    // Seul 468 (classé) génère ; 200 non classé est exclu par l'INNER JOIN.
    assert_eq!(run_one(&con, "R"), 1, "seul le compte classé est mappé");
    // Sortie : compte 471L + nature 1AJUST (mappés), flux F20 hérité, montant −100.
    assert_eq!(
        scount(&con, "account='471L' AND nature='1AJUST' AND flow='F20'"),
        1,
        "account et nature mappés depuis la caractéristique"
    );
    assert!((ssum(&con, "account='471L' AND nature='1AJUST'") - (-100.0)).abs() < TOL);
    // Rien depuis le compte non classé (200).
    assert_eq!(
        scount(&con, "nature='1AJUST' AND flow='F20'"),
        1,
        "une seule sortie mappée (le non-classé n'a rien produit)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  11. Destination `map_ref` : traversée d'une référence directe (patron B).
//      Le compte source est redirigé vers son `compte_parent` (auto-référence).
//      INNER JOIN : un compte sans parent ne génère rien.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn destination_map_ref_traverse_reference_directe() {
    use conso_engine::custom_references::{create, assign};

    let con = engine();

    // Référence directe `compte_parent` sur account → account (hiérarchie).
    create(&con, "account", "compte_parent", "account").unwrap();
    // 705 → 700 (a un parent) ; 200 n'a pas de parent.
    assign(&con, "account", "compte_parent", "705", Some("700")).unwrap();

    put(&con, "M", "705", "F20", None, "0LIASS", 100.0, "converted"); // a un parent → mappé
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted"); // sans parent → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"flow","op":"=","val":"F20"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"account":{"mode":"map_ref","ref":"compte_parent"},
                            "nature":{"mode":"override","value":"MAPRF"}}}]}"#,
    );

    // Seul 705 (qui a un parent) génère ; 200 sans parent est exclu.
    assert_eq!(run_one(&con, "R"), 1, "seul le compte avec parent est mappé");
    // Sortie : account = 700 (parent de 705), nature MAPRF, flux F20 hérité.
    assert_eq!(
        scount(&con, "account='700' AND nature='MAPRF' AND flow='F20'"),
        1,
        "account redirigé vers son compte_parent"
    );
    assert!((ssum(&con, "account='700' AND nature='MAPRF'") - 100.0).abs() < TOL);
}

// ─────────────────────────────────────────────────────────────────────────────
//  12. Sélection par caractéristique N1 (`via`) : filtre les lignes dont le
//      membre est classé dans une valeur N1 donnée. INNER JOIN : un compte non
//      classé dans la caractéristique n'est pas sélectionné, même s'il serait
//      matché par les autres conditions.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selection_via_n1_filtre_par_valeur_de_caracteristique() {
    use conso_engine::characteristics::create_characteristic;

    let con = engine();

    // N1 « regroupement » sur les comptes. Valeurs : PROD (ventes), CHGES (achats).
    create_characteristic(&con, "regroupement", "Regroupement", "account").unwrap();
    let reg_id = conso_engine::characteristics::id_of(&con, "regroupement").unwrap();
    let reg_table = conso_engine::characteristics::value_table(reg_id);
    con.execute(
        &format!(
            "INSERT INTO {reg_table} (code, libelle) VALUES \
             ('PROD', 'Produits'), ('CHGES', 'Charges')"
        ),
        [],
    )
    .unwrap();
    // 700 et 705 classés PROD ; 600 classé CHGES ; 300 non classé.
    con.execute(
        "UPDATE dim_account SET regroupement = CASE \
            WHEN code IN ('700','705') THEN 'PROD' \
            WHEN code = '600' THEN 'CHGES' END \
         WHERE code IN ('700','705','600')",
        [],
    )
    .unwrap();

    put(&con, "M", "700", "F20", None, "0LIASS", 100.0, "converted"); // PROD → match
    put(&con, "M", "705", "F20", None, "0LIASS", 100.0, "converted"); // PROD → match
    put(&con, "M", "600", "F20", None, "0LIASS", 100.0, "converted"); // CHGES → exclu
    put(&con, "M", "300", "F20", None, "0LIASS", 100.0, "converted"); // non classé → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","via":"regroupement","op":"=","val":"PROD"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"SELN1"}}}]}"#,
    );

    // Seuls 700 et 705 (classés PROD) sont matchés.
    assert_eq!(run_one(&con, "R"), 2, "2 comptes PROD sélectionnés");
    assert_eq!(scount(&con, "nature='SELN1' AND account IN ('700','705')"), 2);
    assert_eq!(scount(&con, "nature='SELN1' AND account='600'"), 0, "CHGES exclu");
    assert_eq!(
        scount(&con, "nature='SELN1' AND account='300'"),
        0,
        "non classé exclu par l'INNER JOIN"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  13. Sélection par référence directe (`ref`, patron B) : filtre les lignes
//      dont le membre a une valeur de référence donnée. Ex : comptes dont le
//      `compte_parent` = 700. INNER JOIN : un compte sans parent n'est pas
//      sélectionné.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selection_via_ref_filtre_par_reference_directe() {
    use conso_engine::custom_references::{assign, create};

    let con = engine();

    // Référence directe `compte_parent` sur account → account.
    create(&con, "account", "compte_parent", "account").unwrap();
    // 705 → 700 ; 700 → 100 ; 600 n'a pas de parent.
    assign(&con, "account", "compte_parent", "705", Some("700")).unwrap();
    assign(&con, "account", "compte_parent", "700", Some("100")).unwrap();

    put(&con, "M", "705", "F20", None, "0LIASS", 100.0, "converted"); // parent=700 → match
    put(&con, "M", "700", "F20", None, "0LIASS", 100.0, "converted"); // parent=100 → exclu
    put(&con, "M", "600", "F20", None, "0LIASS", 100.0, "converted"); // sans parent → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","ref":"compte_parent","op":"=","val":"700"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"SELRF"}}}]}"#,
    );

    // Seul 705 (dont le parent est 700) est matché.
    assert_eq!(run_one(&con, "R"), 1, "un seul compte a parent=700");
    assert_eq!(scount(&con, "nature='SELRF' AND account='705'"), 1);
    assert_eq!(scount(&con, "nature='SELRF' AND account='700'"), 0);
    assert_eq!(
        scount(&con, "nature='SELRF' AND account='600'"),
        0,
        "sans parent → exclu par l'INNER JOIN"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  13b. Sélection par FK native ri() (id-aware) : `sous_classe` est stockée en
//      id (B1). La sélection `ref: sous_classe` doit joindre la cible sur l'id
//      et filtrer sur le code utilisateur — 1er cas id-aware post-flip.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn selection_via_sous_classe_id_aware() {
    let con = engine();

    // 200 = actif, 100 = passif (cf. seed ACCOUNTS).
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted"); // actif → match
    put(&con, "M", "100", "F20", None, "0LIASS", 100.0, "converted"); // passif → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","ref":"sous_classe","op":"=","val":"actif"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"SCACTIF"}}}]}"#,
    );

    assert_eq!(run_one(&con, "R"), 1, "un seul compte est de sous_classe actif");
    assert_eq!(scount(&con, "nature='SCACTIF' AND account='200'"), 1);
    assert_eq!(scount(&con, "nature='SCACTIF' AND account='100'"), 0, "passif exclu");
}

#[test]
fn selection_via_flow_scheme_id_aware() {
    let con = engine();

    // Seed : les comptes bilan portent flow_scheme=BILAN, les comptes resultat
    // flow_scheme=RESULTAT. 200 = bilan → match ; 600 = charges (resultat) → exclu.
    put(&con, "M", "200", "F20", None, "0LIASS", 100.0, "converted"); // bilan → match
    put(&con, "M", "600", "F20", None, "0LIASS", 100.0, "converted"); // resultat → exclu

    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"converted",
             "selection":[{"dim":"account","ref":"flow_scheme","op":"=","val":"BILAN"}],
             "coefficient":{"type":"constant","value":1},"multiplicateur":1,
             "destination":{"nature":{"mode":"override","value":"FSBILAN"}}}]}"#,
    );

    assert_eq!(run_one(&con, "R"), 1, "un seul compte est de flow_scheme BILAN");
    assert_eq!(scount(&con, "nature='FSBILAN' AND account='200'"), 1);
    assert_eq!(scount(&con, "nature='FSBILAN' AND account='600'"), 0, "resultat exclu");
}

// ─────────────────────────────────────────────────────────────────────────────
//  Coefficients d'élimination IC corporate (N / N-1 / Var).
//
//  Vérifie la mécanique : facteur = Min(1, INTEG_PA / INTEG_EN), taux N-1 lu via
//  le périmètre du scénario d'à-nouveau. Source : ligne corporate M→B.
//   - N   : INTEG_EN(M)=1.0, INTEG_PA(B)=0.5 → Min(1, 0.5) = 0.5
//   - N-1 : INTEG_EN(M)=1.0, INTEG_PA(B)=0.4 → Min(1, 0.4) = 0.4
//   - Var : 0.5 − 0.4 = 0.1
// ─────────────────────────────────────────────────────────────────────────────

/// Force le `pct_integration` (= pct_interet) d'une entité dans le périmètre N
/// courant (`PERIM_REEL`, 2024) — `seed_all` met toutes les entités à 1.0.
fn set_pct_n(con: &Connection, entity: &str, pct: f64) {
    con.execute(
        "UPDATE sat_perimeter SET pct_integration = ?, pct_interet = ? \
         WHERE perimeter_set = (SELECT id FROM dim_perimeter_set WHERE code = 'PERIM_REEL') AND entity = ? AND period = '2024'",
        duckdb::params![pct, pct, entity],
    )
    .unwrap_or_else(|e| panic!("set_pct_n({entity}): {e}"));
}

/// Branche une consolidation d'à-nouveau `REEL_N1` (périmètre `PSET_N1`,
/// exercice 2023) sur `REEL`, et seede le périmètre N-1 : M=1.0, B=`pct_b_n1`.
fn setup_a_nouveau_perimeter(con: &Connection, pct_b_n1: f64) {
    con.execute(
        "INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PSET_N1', 'Périmètre N-1')",
        [],
    )
    .expect("perimeter_set N-1");
    con.execute(
        "INSERT INTO dim_period (code, libelle) VALUES ('2023', 'Exercice 2023') ON CONFLICT DO NOTHING",
        [],
    )
    .expect("période 2023");
    con.execute(
        "INSERT INTO dim_consolidation \
            (id, libelle, phase, exercice, perimeter_set, variant, presentation_currency, \
             perimeter_period, rate_set, rate_period, ruleset_code, a_nouveau_consolidation_id, statut) \
         VALUES (nextval('seq_consolidation'), 'Réel 2023', \
                 (SELECT id FROM dim_scenario_category WHERE code = 'REEL'),
                 (SELECT id FROM dim_period WHERE code = '2023'),
                 (SELECT id FROM dim_perimeter_set WHERE code = 'PSET_N1'), \
                 (SELECT id FROM dim_variant WHERE code = 'BASE'),
                 (SELECT id FROM dim_currency WHERE code_iso = 'EUR'),
                 (SELECT id FROM dim_period WHERE code = '2023'),
                 (SELECT id FROM dim_rate_set WHERE code = 'RATES'),
                 (SELECT id FROM dim_period WHERE code = '2023'),
                 NULL, NULL, 'verrouillé')",
        [],
    )
    .expect("consolidation à-nouveau");
    // Rattache REEL_N1 à REEL (a_nouveau_consolidation_id).
    let reel_id = cid(con, "REEL", "2024");
    let reel_n1_id = cid(con, "REEL", "2023");
    con.execute(
        "UPDATE dim_consolidation SET a_nouveau_consolidation_id = ? WHERE id = ?",
        duckdb::params![reel_n1_id, reel_id],
    )
    .expect("lien à-nouveau");
    // Périmètre N-1 : M intégrée à 1.0, B à `pct_b_n1`.
    for (entity, pct) in [("M", 1.0_f64), ("B", pct_b_n1)] {
        con.execute(
            "INSERT INTO sat_perimeter \
                (perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie) \
             VALUES ((SELECT id FROM dim_perimeter_set WHERE code = 'PSET_N1'), ?, '2023', \
                     (SELECT id FROM dim_method WHERE code = 'globale'), ?, ?, false, false)",
            duckdb::params![entity, pct, pct],
        )
        .expect("sat_perimeter N-1");
    }
}

#[test]
fn coefficient_elim_ic_corp_n_n1_var() {
    let con = engine();
    setup_a_nouveau_perimeter(&con, 0.4);
    // INTEG N : M=1.0 (seedé), B forcé à 0.5 (seed_all met tout à 1.0).
    set_pct_n(&con, "B", 0.5);
    // Ligne interco corporate M→B, 1000.
    put(&con, "M", "700", "F20", Some("B"), "0LIASS", 1000.0, "corporate");

    // Une règle, 3 opérations (même snapshot), chaque coefficient taggé sur
    // `analysis` (dimension libre) pour distinguer les sorties.
    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"corporate",
             "selection":[{"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"elim_ic_corp_n"},"multiplicateur":1,
             "destination":{"analysis":{"mode":"override","value":"N"}}},
            {"seq":2,"level":"corporate",
             "selection":[{"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"elim_ic_corp_n1"},"multiplicateur":1,
             "destination":{"analysis":{"mode":"override","value":"N1"}}},
            {"seq":3,"level":"corporate",
             "selection":[{"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"elim_ic_corp_var"},"multiplicateur":1,
             "destination":{"analysis":{"mode":"override","value":"VAR"}}}]}"#,
    );
    assert_eq!(run_one(&con, "R"), 3, "3 opérations → 3 lignes");

    assert!(
        (ssum(&con, "analysis='N'") - 500.0).abs() < TOL,
        "N : 1000 × Min(1, 0.5/1.0) = 500"
    );
    assert!(
        (ssum(&con, "analysis='N1'") - 400.0).abs() < TOL,
        "N-1 : 1000 × Min(1, 0.4/1.0) = 400"
    );
    assert!(
        (ssum(&con, "analysis='VAR'") - 100.0).abs() < TOL,
        "Var : 1000 × (0.5 − 0.4) = 100"
    );
}

/// Sans scénario d'à-nouveau, le taux N-1 dégrade à 0 : `N1 = 0`, `Var = N`.
#[test]
fn coefficient_elim_ic_corp_sans_a_nouveau_n1_nul() {
    let con = engine();
    // Pas de setup_a_nouveau_perimeter : REEL.a_nouveau_consolidation_id reste NULL.
    set_pct_n(&con, "B", 0.5);
    put(&con, "M", "700", "F20", Some("B"), "0LIASS", 1000.0, "corporate");
    create_rule(
        &con,
        "R",
        r#"{"scope":[],"operations":[
            {"seq":1,"level":"corporate",
             "selection":[{"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"elim_ic_corp_n1"},"multiplicateur":1,
             "destination":{"analysis":{"mode":"override","value":"N1"}}},
            {"seq":2,"level":"corporate",
             "selection":[{"dim":"partner","op":"IS NOT NULL"}],
             "coefficient":{"type":"elim_ic_corp_var"},"multiplicateur":1,
             "destination":{"analysis":{"mode":"override","value":"VAR"}}}]}"#,
    );
    run_one(&con, "R");
    assert!(
        ssum(&con, "analysis='N1'").abs() < TOL,
        "pas d'à-nouveau → taux N-1 = 0 → N1 = 0"
    );
    assert!(
        (ssum(&con, "analysis='VAR'") - 500.0).abs() < TOL,
        "Var = N − 0 = 500"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  6b. Règle stockée avec `via` en id entier (post-migration JSON étape 6b) :
//      le chemin d'exécution doit dénormaliser avant parsing.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn via_id_selection_fonctionne_apres_normalisation() {
    use conso_engine::characteristics::create_characteristic;
    use conso_engine::json_migration::{denormalize_rule_definition, normalize_rule_definition};

    let con = engine();

    create_characteristic(&con, "regroupement", "R", "account").unwrap();
    let reg_id = conso_engine::characteristics::id_of(&con, "regroupement").unwrap();
    let reg_table = conso_engine::characteristics::value_table(reg_id);
    con.execute(
        &format!("INSERT INTO {reg_table} (code, libelle) VALUES ('PROD', 'Produits')"),
        [],
    )
    .unwrap();
    con.execute(
        "UPDATE dim_account SET regroupement = 'PROD' WHERE code IN ('700','705')",
        [],
    )
    .unwrap();
    put(&con, "M", "700", "F20", None, "0LIASS", 100.0, "converted"); // classé PROD → match
    put(&con, "M", "600", "F20", None, "0LIASS", 100.0, "converted"); // non classé → exclu

    // Normaliser la définition avant stockage (simule ce que fait le serveur).
    let def_code = r#"{"scope":[],"operations":[
        {"seq":1,"level":"converted",
         "selection":[{"dim":"account","via":"regroupement","op":"=","val":"PROD"}],
         "coefficient":{"type":"constant","value":1},"multiplicateur":1,
         "destination":{"nature":{"mode":"override","value":"SELN1"}}}]}"#;
    let def_id = normalize_rule_definition(&con, def_code).unwrap();

    // Vérifier que `via` est devenu un entier après normalisation.
    let v: serde_json::Value = serde_json::from_str(&def_id).unwrap();
    assert_eq!(
        v["operations"][0]["selection"][0]["via"].as_i64(),
        Some(reg_id),
        "via doit être l'id après normalisation"
    );

    // Vérifier que la dénormalisation redonne le code.
    let def_back = denormalize_rule_definition(&con, &def_id).unwrap();
    assert!(
        def_back.contains("\"via\":\"regroupement\""),
        "via doit être le code après dénormalisation : {def_back}"
    );

    // Stocker la règle sous forme normalisée (ids) et exécuter.
    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES ('RVIA', 'test_via', ?)",
        duckdb::params![def_id],
    )
    .unwrap();
    let n = run_one(&con, "RVIA");
    assert_eq!(n, 1, "règle avec via=id doit exécuter (seul le compte classé PROD)");
    assert_eq!(scount(&con, "nature='SELN1' AND account='700'"), 1, "700 sélectionné");
    assert_eq!(scount(&con, "nature='SELN1' AND account='600'"), 0, "600 exclu");
}

#[test]
fn via_id_destination_map_fonctionne_apres_normalisation() {
    use conso_engine::characteristics::{add_attribute, create_characteristic};
    use conso_engine::json_migration::normalize_rule_definition;

    let con = engine();

    create_characteristic(&con, "comportement", "C", "account").unwrap();
    add_attribute(&con, "comportement", "compte_destination", "Cpt dest", "account").unwrap();
    add_attribute(&con, "comportement", "nat", "Nature", "nature").unwrap();
    let char_id = conso_engine::characteristics::id_of(&con, "comportement").unwrap();
    let car_table = conso_engine::characteristics::value_table(char_id);
    let col_cd = conso_engine::characteristics::attr_col_for(&con, "comportement", "compte_destination").unwrap();
    let col_nat = conso_engine::characteristics::attr_col_for(&con, "comportement", "nat").unwrap();
    con.execute(
        &format!(
            "INSERT INTO {car_table} (code, libelle, \"{col_cd}\", \"{col_nat}\") \
             VALUES ('VENTES_IC', 'V', '471L', '1AJUST')"
        ),
        [],
    )
    .unwrap();
    con.execute(
        "UPDATE dim_account SET comportement = 'VENTES_IC' WHERE code = '468'",
        [],
    )
    .unwrap();
    put(&con, "M", "468", "F20", None, "0LIASS", 100.0, "converted");

    // Normaliser la définition (via code → id dans destination.map).
    let def_code = r#"{"scope":[],"operations":[
        {"seq":1,"level":"converted",
         "selection":[{"dim":"flow","op":"=","val":"F20"}],
         "coefficient":{"type":"constant","value":1},"multiplicateur":-1,
         "destination":{
            "account":{"mode":"map","via":"comportement","attr":"compte_destination"},
            "nature":{"mode":"map","via":"comportement","attr":"nat"}}}]}"#;
    let def_id = normalize_rule_definition(&con, def_code).unwrap();
    let v: serde_json::Value = serde_json::from_str(&def_id).unwrap();
    assert_eq!(
        v["operations"][0]["destination"]["account"]["via"].as_i64(),
        Some(char_id),
        "via dans destination.map doit être l'id"
    );

    con.execute(
        "INSERT INTO dim_rule (code, libelle, definition) VALUES ('RMAP', 'test_map', ?)",
        duckdb::params![def_id],
    )
    .unwrap();
    let n = run_one(&con, "RMAP");
    assert_eq!(n, 1, "destination map avec via=id doit exécuter");
    assert_eq!(scount(&con, "account='471L' AND nature='1AJUST'"), 1, "compte + nature mappés");
}
