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

/// `true` si la colonne `(table, column)` est de type entier (déjà migrée).
fn column_is_int(con: &Connection, table: &str, column: &str) -> duckdb::Result<bool> {
    let data_type: String = con.query_row(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema = 'main' AND table_name = ? AND column_name = ?",
        [table, column],
        |row| row.get(0),
    )?;
    Ok(data_type.to_uppercase().contains("INT"))
}

/// Migration in-place des **FK à contrat code** de `dim_consolidation` (option A,
/// chantier B1, étape 3) sur une base **existante** : convertit `phase`,
/// `perimeter_set`, `variant`, `rate_set` du stockage **code (TEXT)** vers la
/// **clé technique (id, INTEGER)**. Idempotent (no-op si `variant` est déjà
/// entier).
///
/// Nécessaire car le changement de DDL (TEXT→INTEGER) ne s'applique qu'aux bases
/// **neuves** (`create_schema`) ; une base persistée garde ses colonnes TEXT.
/// À appeler au démarrage serveur **après** [`ensure_ids`] (les dimensions cibles
/// doivent déjà avoir leur `id`).
///
/// **Reconstruction de table** plutôt qu'`ALTER COLUMN` : DuckDB interdit une
/// sous-requête dans `ALTER … SET DATA TYPE … USING` (et le changement de type
/// d'une colonne prise dans la contrainte `UNIQUE` est délicat). On recrée donc
/// la table au nouveau schéma puis on réinjecte en résolvant les codes→ids,
/// `id` préservés.
pub fn migrate_consolidation_fk_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_consolidation")? {
        return Ok(());
    }
    if column_is_int(con, "dim_consolidation", "variant")? {
        return Ok(()); // déjà migrée
    }
    // Table neuve au schéma cible (FK en INTEGER), réinjection avec résolution
    // code→id des 4 FK flippées ; les autres colonnes sont reprises telles quelles.
    let create_mig = crate::schema::DDL_DIM_CONSOLIDATION
        .replace("dim_consolidation (", "dim_consolidation__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO dim_consolidation__mig
            (id, libelle, phase, exercice, perimeter_set, variant, presentation_currency,
             perimeter_period, rate_set, rate_period, ruleset_code,
             a_nouveau_consolidation_id, statut)
         SELECT c.id, c.libelle,
                (SELECT s.id FROM dim_scenario_category s WHERE s.code = c.phase),
                c.exercice,
                (SELECT p.id FROM dim_perimeter_set p WHERE p.code = c.perimeter_set),
                (SELECT v.id FROM dim_variant v WHERE v.code = c.variant),
                c.presentation_currency, c.perimeter_period,
                (SELECT r.id FROM dim_rate_set r WHERE r.code = c.rate_set),
                c.rate_period, c.ruleset_code, c.a_nouveau_consolidation_id, c.statut
         FROM dim_consolidation c;
         DROP TABLE dim_consolidation;
         ALTER TABLE dim_consolidation__mig RENAME TO dim_consolidation;",
    ))?;
    Ok(())
}

/// Migration in-place de `sat_exchange_rate.rate_set` (option A, chantier B1) sur
/// une base **existante** : convertit la colonne du stockage **code (TEXT)** vers
/// la **clé technique (id, INTEGER)**. Idempotent (no-op si `rate_set` est déjà
/// entier).
///
/// `rate_set` est dans la PK `(rate_set, currency_source, period)` →
/// **reconstruction de table** (cf. [`migrate_consolidation_fk_to_id`]) : DuckDB
/// interdit `ALTER … SET DATA TYPE … USING (sous-requête)` et le changement de
/// type d'une colonne de PK est délicat. À appeler au démarrage serveur **après**
/// [`ensure_ids`] (`dim_rate_set` doit déjà avoir son `id`).
pub fn migrate_sat_exchange_rate_fk_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "sat_exchange_rate")? {
        return Ok(());
    }
    if column_is_int(con, "sat_exchange_rate", "rate_set")? {
        return Ok(()); // déjà migrée
    }
    let create_mig = crate::schema::DDL_SAT_EXCHANGE_RATE
        .replace("sat_exchange_rate (", "sat_exchange_rate__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO sat_exchange_rate__mig
            (rate_set, currency_source, period, taux_close, taux_moyen, taux_ouverture)
         SELECT (SELECT r.id FROM dim_rate_set r WHERE r.code = s.rate_set),
                s.currency_source, s.period, s.taux_close, s.taux_moyen, s.taux_ouverture
         FROM sat_exchange_rate s;
         DROP TABLE sat_exchange_rate;
         ALTER TABLE sat_exchange_rate__mig RENAME TO sat_exchange_rate;",
    ))?;
    Ok(())
}

/// Migration in-place de `sat_perimeter.perimeter_set` (option A, chantier B1)
/// sur une base **existante** : convertit la colonne du stockage **code (TEXT)**
/// vers la **clé technique (id, INTEGER)**. Idempotent (no-op si déjà entier).
///
/// `perimeter_set` est dans la PK `(perimeter_set, entity, period)` →
/// **reconstruction de table** (cf. [`migrate_consolidation_fk_to_id`]). À appeler
/// au démarrage serveur **après** [`ensure_ids`] (`dim_perimeter_set` doit avoir
/// son `id`). Rend la dimension `perimeter_set` entièrement flippée (renommable).
pub fn migrate_sat_perimeter_fk_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "sat_perimeter")? {
        return Ok(());
    }
    if column_is_int(con, "sat_perimeter", "perimeter_set")? {
        return Ok(()); // déjà migrée
    }
    let create_mig = crate::schema::DDL_SAT_PERIMETER
        .replace("sat_perimeter (", "sat_perimeter__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO sat_perimeter__mig
            (perimeter_set, entity, period, methode,
             pct_interet, pct_integration, entree, sortie)
         SELECT (SELECT p.id FROM dim_perimeter_set p WHERE p.code = s.perimeter_set),
                s.entity, s.period, s.methode,
                s.pct_interet, s.pct_integration, s.entree, s.sortie
         FROM sat_perimeter s;
         DROP TABLE sat_perimeter;
         ALTER TABLE sat_perimeter__mig RENAME TO sat_perimeter;",
    ))?;
    Ok(())
}

/// Migration Q44 : ajoute la colonne `sens` à `dim_sous_classe` sur une base
/// existante et **backfille** les codes natifs (traduit une fois l'ancien dur
/// `SENS_CASE` en données). Idempotent. Après cela, `sens` est user-driven.
pub fn ensure_sous_classe_sens(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_sous_classe")? {
        return Ok(());
    }
    if column_exists(con, "dim_sous_classe", "sens")? {
        return Ok(()); // déjà migrée
    }
    con.execute("ALTER TABLE dim_sous_classe ADD COLUMN sens TEXT", [])?;
    // Backfill unique des codes natifs (passif/produits → C, actif/charges → D),
    // équivalent à l'ancien `SENS_CASE` de server.rs. Les autres sous-classes
    // (utilisateur) restent NULL jusqu'à saisie.
    con.execute_batch(
        "UPDATE dim_sous_classe SET sens = 'C' WHERE code IN ('passif', 'produits');
         UPDATE dim_sous_classe SET sens = 'D' WHERE code IN ('actif', 'charges');",
    )?;
    Ok(())
}

/// Migration B1 : bascule `dim_account.sous_classe` du code (TEXT) vers la clé
/// technique (id, INTEGER) sur une base existante. Idempotent.
///
/// Contrairement aux satellites (PK → reconstruction), `sous_classe` est une
/// colonne simple hors PK/UNIQUE : on utilise **add temp + update + drop +
/// rename**, qui préserve les colonnes custom runtime (références directes
/// patron B ajoutées par `custom_references::reapply`). La table cible
/// `dim_sous_classe` doit déjà avoir son `id` (cf. [`ensure_ids`]).
pub fn migrate_account_sous_classe_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_account")? {
        return Ok(());
    }
    if column_is_int(con, "dim_account", "sous_classe")? {
        return Ok(()); // déjà migrée
    }
    con.execute_batch(
        "ALTER TABLE dim_account ADD COLUMN sous_classe__b1 INTEGER;
         UPDATE dim_account SET sous_classe__b1 = (
             SELECT sc.id FROM dim_sous_classe sc WHERE sc.code = dim_account.sous_classe
         ) WHERE dim_account.sous_classe IS NOT NULL;
         ALTER TABLE dim_account DROP COLUMN sous_classe;
         ALTER TABLE dim_account RENAME COLUMN sous_classe__b1 TO sous_classe;",
    )?;
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
    fn migrate_consolidation_fk_to_id_convertit_les_codes_en_id() {
        // Simule une base **existante** (ancien schéma) : dim_consolidation avec
        // ses FK en TEXT (codes). On vérifie que la migration la reconstruit avec
        // les FK en id, en préservant l'id de la consolidation et la contrainte
        // UNIQUE.
        let con = setup(); // schéma neuf (sert pour les dimensions cibles + seq)
        // Dimensions cibles avec leurs id (ensure_ids l'a fait au create_schema).
        con.execute_batch(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES ('REEL','Réel');
             INSERT INTO dim_variant (code, libelle) VALUES ('BASE','Base');
             INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PERIM_REEL','Périmètre');
             INSERT INTO dim_rate_set (code, libelle) VALUES ('RATES','Taux réels');",
        )
        .unwrap();

        // Rétrograde dim_consolidation à l'ancien schéma (FK en TEXT) avec une
        // ligne en codes.
        con.execute_batch(
            "DROP TABLE dim_consolidation;
             CREATE TABLE dim_consolidation (
                id INTEGER PRIMARY KEY, libelle TEXT, phase TEXT, exercice TEXT,
                perimeter_set TEXT, variant TEXT, presentation_currency TEXT,
                perimeter_period TEXT, rate_set TEXT, rate_period TEXT, ruleset_code TEXT,
                a_nouveau_consolidation_id INTEGER, statut TEXT,
                UNIQUE (phase, exercice, perimeter_set, variant, presentation_currency));
             INSERT INTO dim_consolidation VALUES
                (1,'Réel','REEL','2024','PERIM_REEL','BASE','EUR','2024','RATES','2024',
                 NULL, NULL, 'ouvert');",
        )
        .unwrap();

        migrate_consolidation_fk_to_id(&con).expect("migration");

        // Les 4 FK sont désormais des entiers = id de leur cible ; id préservé.
        let (phase, perim, variant, rate, id): (i64, i64, i64, i64, i64) = con
            .query_row(
                "SELECT phase, perimeter_set, variant, rate_set, id FROM dim_consolidation",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("FK migrées en entiers");
        assert_eq!(id, 1, "id préservé");
        assert_eq!(
            phase,
            crate::resolve::resolve_id(&con, "dim_scenario_category", "REEL").unwrap().unwrap()
        );
        assert_eq!(
            perim,
            crate::resolve::resolve_id(&con, "dim_perimeter_set", "PERIM_REEL").unwrap().unwrap()
        );
        assert_eq!(
            variant,
            crate::resolve::resolve_id(&con, "dim_variant", "BASE").unwrap().unwrap()
        );
        assert_eq!(
            rate,
            crate::resolve::resolve_id(&con, "dim_rate_set", "RATES").unwrap().unwrap()
        );
        // exercice (non flippé) reste un code.
        let exercice: String = con
            .query_row("SELECT exercice FROM dim_consolidation", [], |r| r.get(0))
            .unwrap();
        assert_eq!(exercice, "2024");

        // Idempotent : second passage no-op (variant déjà entier).
        migrate_consolidation_fk_to_id(&con).expect("migration #2");
    }

    #[test]
    fn migrate_sat_exchange_rate_fk_to_id_convertit_les_codes_en_id() {
        // Simule une base **existante** : sat_exchange_rate avec rate_set en TEXT
        // (codes). On vérifie que la migration la reconstruit avec rate_set en id.
        let con = setup(); // schéma neuf (sert pour dim_rate_set + seq + id)
        con.execute_batch(
            "INSERT INTO dim_rate_set (code, libelle) VALUES ('RATES','Taux réels');",
        )
        .unwrap();

        // Rétrograde sat_exchange_rate à l'ancien schéma (rate_set TEXT, code).
        con.execute_batch(
            "DROP TABLE sat_exchange_rate;
             CREATE TABLE sat_exchange_rate (
                rate_set TEXT, currency_source TEXT, period TEXT,
                taux_close DECIMAL(18,8), taux_moyen DECIMAL(18,8), taux_ouverture DECIMAL(18,8),
                PRIMARY KEY (rate_set, currency_source, period)
             );
             INSERT INTO sat_exchange_rate VALUES
                ('RATES','USD','2024', 0.9, 0.95, 0.92);",
        )
        .unwrap();

        migrate_sat_exchange_rate_fk_to_id(&con).expect("migration");

        // rate_set est désormais l'id de 'RATES' ; la ligne est préservée.
        let (rate_id, src): (i64, String) = con
            .query_row(
                "SELECT rate_set, currency_source FROM sat_exchange_rate",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("ligne présente après migration");
        assert_eq!(src, "USD", "autres colonnes préservées");
        assert_eq!(
            rate_id,
            crate::resolve::resolve_id(&con, "dim_rate_set", "RATES")
                .unwrap()
                .unwrap()
        );

        // Idempotent : second passage no-op (rate_set déjà entier).
        migrate_sat_exchange_rate_fk_to_id(&con).expect("migration #2");
    }

    #[test]
    fn migrate_sat_perimeter_fk_to_id_convertit_les_codes_en_id() {
        // Simule une base existante : sat_perimeter avec perimeter_set en TEXT.
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PERIM_REEL','Périmètre');",
        )
        .unwrap();

        con.execute_batch(
            "DROP TABLE sat_perimeter;
             CREATE TABLE sat_perimeter (
                perimeter_set TEXT, entity TEXT, period TEXT, methode TEXT,
                pct_interet DECIMAL(10,4), pct_integration DECIMAL(10,4),
                entree BOOLEAN DEFAULT FALSE, sortie BOOLEAN DEFAULT FALSE,
                PRIMARY KEY (perimeter_set, entity, period)
             );
             INSERT INTO sat_perimeter VALUES
                ('PERIM_REEL','E1','2024','globale',1.0,1.0,FALSE,FALSE);",
        )
        .unwrap();

        migrate_sat_perimeter_fk_to_id(&con).expect("migration");

        let (perim_id, entity): (i64, String) = con
            .query_row(
                "SELECT perimeter_set, entity FROM sat_perimeter",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("ligne présente après migration");
        assert_eq!(entity, "E1", "autres colonnes préservées");
        assert_eq!(
            perim_id,
            crate::resolve::resolve_id(&con, "dim_perimeter_set", "PERIM_REEL")
                .unwrap()
                .unwrap()
        );

        // Idempotent.
        migrate_sat_perimeter_fk_to_id(&con).expect("migration #2");
    }

    #[test]
    fn ensure_sous_classe_sens_backfille_les_codes_natifs() {
        // Base existante : dim_sous_classe sans la colonne `sens`.
        let con = Connection::open_in_memory().unwrap();
        con.execute_batch(
            "CREATE TABLE dim_sous_classe (code TEXT PRIMARY KEY, libelle TEXT, classe TEXT);
             INSERT INTO dim_sous_classe VALUES
                ('actif','Actif','bilan'), ('passif','Passif','bilan'),
                ('charges','Charges','resultat'), ('produits','Produits','resultat'),
                ('CUSTOM','Perso','bilan');", // utilisateur → reste NULL
        )
        .unwrap();

        ensure_sous_classe_sens(&con).expect("migration sens");

        let sens_of = |code: &str| -> String {
            con.query_row(
                "SELECT COALESCE(sens,'?') FROM dim_sous_classe WHERE code=?",
                [code],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(sens_of("actif"), "D");
        assert_eq!(sens_of("charges"), "D");
        assert_eq!(sens_of("passif"), "C");
        assert_eq!(sens_of("produits"), "C");
        assert_eq!(sens_of("CUSTOM"), "?", "sous-classe utilisateur non backfillée");

        // Idempotent.
        ensure_sous_classe_sens(&con).expect("migration #2");
    }

    /// Q44 : le sens user-driven (`dim_sous_classe.sens`) reproduit exactement
    /// l'ancien `SENS_CASE` codé en dur, sur tous les comptes seedés. Sous B1,
    /// `dim_account.sous_classe` est un id — on résout le code via JOIN sur id,
    /// puis on compare la colonne `sens` au CASE historique appliqué au code.
    #[test]
    fn sens_data_driven_equivaut_au_case_historique() {
        let con = setup();
        crate::seed_all(&con).expect("seed_all");
        let diff: i64 = con
            .query_row(
                "SELECT COUNT(*) FROM (
                    SELECT a.code,
                           sc.sens AS sens_new,
                           CASE sc.code
                               WHEN 'passif' THEN 'C' WHEN 'produits' THEN 'C'
                               WHEN 'actif'  THEN 'D' WHEN 'charges' THEN 'D'
                               ELSE '?' END AS sens_old
                    FROM dim_account a
                    LEFT JOIN dim_sous_classe sc ON sc.id = a.sous_classe
                ) WHERE sens_new IS DISTINCT FROM sens_old",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            diff, 0,
            "le sens user-driven doit coïncider avec le CASE historique sur tout le seed"
        );
    }

    #[test]
    fn migrate_account_sous_classe_to_id_convertit_les_codes() {
        // Base existante : dim_account avec sous_classe TEXT (codes), + une colonne
        // custom runtime (patron B) qui doit survivre à la migration.
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_sous_classe (code, libelle, classe) VALUES
                ('actif','Actif','bilan'),('passif','Passif','bilan');
             ALTER TABLE dim_account ADD COLUMN compte_parent TEXT;
             DROP TABLE dim_account;
             CREATE TABLE dim_account (
                code TEXT PRIMARY KEY, libelle TEXT, classe TEXT,
                sous_classe TEXT, flow_scheme TEXT, compte_parent TEXT
             );
             INSERT INTO dim_account VALUES
                ('10','Cap','bilan','passif',NULL,'ROOT'),
                ('101','Capital','bilan','actif',NULL,'10');",
        )
        .unwrap();

        migrate_account_sous_classe_to_id(&con).expect("migration");

        let (sc, parent): (i64, String) = con
            .query_row(
                "SELECT sous_classe, compte_parent FROM dim_account WHERE code='101'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent, "10", "colonne custom préservée");
        assert_eq!(
            sc,
            crate::resolve::resolve_id(&con, "dim_sous_classe", "actif")
                .unwrap()
                .unwrap(),
            "sous_classe résolu en id"
        );
        // Idempotent.
        migrate_account_sous_classe_to_id(&con).expect("migration #2");
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
