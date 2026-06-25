//! Clés techniques (`id`) des dimensions — **étape 1** du chantier « codes
//! renommables » (option B1, cf. `docs/PLAN_RENOMMAGE_CODES.md`).
//!
//! Objectif de cette étape : **doter chaque dimension d'un `id` technique
//! immuable**, sans rien casser. À ce stade les `id` sont *alloués mais pas
//! encore consommés* : le `code` reste la clé primaire, toutes les FK et la table
//! de faits continuent de pointer sur les codes. Les étapes ultérieures (3–4)
//! basculeront les références vers ces `id`.
//!
//! # Pourquoi une étape post-création plutôt que d'amender chaque DDL
//!
//! Les `CREATE TABLE` de [`crate::schema`] restent inchangés : on ajoute la
//! colonne `id` + sa séquence + son unicité **après** création, via
//! [`ensure_ids`]. Le même chemin couvre :
//! - les bases **neuves** (appelé depuis `schema::create_schema`) ;
//! - les bases **existantes** (appelé au démarrage serveur) — migration
//!   in-place qui *préserve les objets présents* (on backfille les lignes
//!   existantes, on ne reseed pas).
//!
//! C'est le même patron que les autres migrations idempotentes du démarrage
//! (`coefficients::ensure_schema`, `custom_references::migrate_native`).
//!
//! `dim_consolidation` est **exclue** : elle a déjà une PK technique `id`
//! (cf. `schema::DDL_DIM_CONSOLIDATION`). `dim_custom_reference` (PK composite
//! sans code) est traitée plus tard (étape 5, objets dynamiques).

use duckdb::Connection;

/// Les dimensions dotées d'un `code` textuel, à enrichir d'un `id` technique.
///
/// Tuple `(table, colonne_code, séquence)`. Inclut les registres qui survivent au
/// reset (`dim_value_list`, `dim_characteristic`, `dim_custom_dimension`) : leur
/// `id` doit exister comme pour les autres dimensions.
pub const SURROGATE_DIMS: &[(&str, &str, &str)] = &[
    ("dim_scenario_category", "code", "seq_dim_scenario_category"),
    ("dim_rate_set", "code", "seq_dim_rate_set"),
    ("dim_perimeter_set", "code", "seq_dim_perimeter_set"),
    ("dim_variant", "code", "seq_dim_variant"),
    ("dim_entity", "code", "seq_dim_entity"),
    ("dim_period", "code", "seq_dim_period"),
    ("dim_account", "code", "seq_dim_account"),
    ("dim_sous_classe", "code", "seq_dim_sous_classe"),
    ("dim_flow", "code", "seq_dim_flow"),
    ("dim_flow_scheme", "code", "seq_dim_flow_scheme"),
    ("dim_currency", "code_iso", "seq_dim_currency"),
    ("dim_nature", "code", "seq_dim_nature"),
    ("dim_method", "code", "seq_dim_method"),
    ("dim_rule", "code", "seq_dim_rule"),
    ("dim_ruleset", "code", "seq_dim_ruleset"),
    ("dim_coefficient", "code", "seq_dim_coefficient"),
    ("dim_aggregate", "code", "seq_dim_aggregate"),
    ("dim_indicator", "code", "seq_dim_indicator"),
    ("dim_value_list", "code", "seq_dim_value_list"),
    ("dim_characteristic", "code", "seq_dim_characteristic"),
    ("dim_custom_dimension", "name", "seq_dim_custom_dimension"),
];

/// `true` si la table `table` possède une colonne `column`.
fn column_exists(con: &Connection, table: &str, column: &str) -> duckdb::Result<bool> {
    con.query_row(
        "SELECT COUNT(*) > 0 FROM information_schema.columns \
         WHERE table_schema = 'main' AND table_name = ? AND column_name = ?",
        [table, column],
        |r| r.get(0),
    )
}

/// `true` si la table existe (tolère les tables absentes selon l'état du schéma).
fn table_exists(con: &Connection, table: &str) -> duckdb::Result<bool> {
    con.query_row(
        "SELECT COUNT(*) > 0 FROM information_schema.tables \
         WHERE table_schema = 'main' AND table_name = ?",
        [table],
        |r| r.get(0),
    )
}

/// Dote chaque dimension de [`SURROGATE_DIMS`] d'une colonne `id` technique,
/// peuplée par une séquence dédiée, unique. **Idempotent** : ne fait rien sur une
/// table déjà migrée. Préserve les lignes existantes (backfill `nextval`).
///
/// Étapes par table (seulement si la colonne `id` manque) :
/// 1. `CREATE SEQUENCE IF NOT EXISTS` ;
/// 2. `ALTER TABLE … ADD COLUMN id INTEGER` ;
/// 3. backfill des lignes existantes (`UPDATE … SET id = nextval(seq)`) ;
/// 4. `ALTER COLUMN id SET DEFAULT nextval(seq)` (inserts futurs auto).
///
/// **Unicité** : pas d'index unique explicite à ce stade. DuckDB interdit
/// `ALTER … DROP COLUMN` tant qu'un index utilisateur existe sur la table — or
/// les caractéristiques et références directes droppent des colonnes sur ces
/// dimensions. L'unicité de l'`id` est garantie par la **séquence** (jamais
/// d'écriture manuelle d'id) ; la contrainte PK sur `id` sera posée à l'étape 3/4
/// lors de la reconstruction des tables (bascule des FK vers les `id`).
pub fn ensure_ids(con: &Connection) -> duckdb::Result<()> {
    for &(table, _code_col, seq) in SURROGATE_DIMS {
        if !table_exists(con, table)? {
            continue; // table pas encore créée (état de schéma partiel)
        }
        con.execute(&format!("CREATE SEQUENCE IF NOT EXISTS {seq} START 1"), [])?;
        if column_exists(con, table, "id")? {
            continue; // déjà migrée
        }
        con.execute(&format!("ALTER TABLE {table} ADD COLUMN id INTEGER"), [])?;
        // Backfill des lignes existantes (ordre indifférent : l'id est opaque).
        con.execute(&format!("UPDATE {table} SET id = nextval('{seq}')"), [])?;
        // Inserts futurs : id auto-généré.
        con.execute(
            &format!("ALTER TABLE {table} ALTER COLUMN id SET DEFAULT nextval('{seq}')"),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::create_schema;

    fn setup() -> Connection {
        let con = Connection::open_in_memory().expect("open_in_memory");
        create_schema(&con).expect("create_schema");
        con
    }

    #[test]
    fn chaque_dimension_a_une_colonne_id() {
        let con = setup();
        for &(table, _, _) in SURROGATE_DIMS {
            assert!(
                column_exists(&con, table, "id").unwrap(),
                "{table} doit avoir une colonne id après create_schema"
            );
        }
    }

    #[test]
    fn les_lignes_seedees_recoivent_un_id_unique_non_null() {
        let con = setup();
        crate::seed_all(&con).expect("seed_all");
        // dim_account est peuplée par le seed : ses ids doivent être non nuls et
        // distincts.
        let (total, distincts, non_nuls): (i64, i64, i64) = con
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT id), COUNT(id) FROM dim_account",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(total > 0, "dim_account peuplée par le seed");
        assert_eq!(total, distincts, "ids distincts");
        assert_eq!(total, non_nuls, "aucun id NULL");
    }

    #[test]
    fn insert_apres_migration_recoit_un_id_par_defaut() {
        let con = setup();
        // Insert sans fournir d'id : le DEFAULT nextval doit le générer.
        con.execute(
            "INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('XAF','Franc CFA',0)",
            [],
        )
        .unwrap();
        let id: Option<i64> = con
            .query_row(
                "SELECT id FROM dim_currency WHERE code_iso = 'XAF'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(id.is_some(), "l'insert doit recevoir un id auto");
    }

    #[test]
    fn ensure_ids_est_idempotent() {
        let con = setup();
        // create_schema a déjà appelé ensure_ids ; un second appel ne doit pas
        // échouer ni dupliquer la colonne.
        ensure_ids(&con).expect("ensure_ids #2");
        ensure_ids(&con).expect("ensure_ids #3");
        // Toujours une seule colonne id.
        let n: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM information_schema.columns \
                 WHERE table_name = 'dim_account' AND column_name = 'id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "une seule colonne id sur dim_account");
    }

    #[test]
    fn migration_in_place_backfille_une_table_existante() {
        // Simule une base **antérieure** : table dim_flow sans id (on la recrée
        // à la main sans passer par ensure_ids), avec des lignes présentes.
        let con = Connection::open_in_memory().expect("open_in_memory");
        con.execute_batch(
            "CREATE TABLE dim_flow (code TEXT PRIMARY KEY, libelle TEXT);
             INSERT INTO dim_flow VALUES ('F00','Ouverture'),('F99','Clôture');",
        )
        .unwrap();
        assert!(!column_exists(&con, "dim_flow", "id").unwrap());

        ensure_ids(&con).expect("ensure_ids migration");

        // Les 2 lignes existantes ont reçu un id unique non nul.
        let (total, distincts, non_nuls): (i64, i64, i64) = con
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT id), COUNT(id) FROM dim_flow",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(distincts, 2, "ids distincts après backfill");
        assert_eq!(non_nuls, 2, "aucun id NULL après backfill");
    }
}
