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
    ("dim_control", "code", "seq_dim_control"),
    ("dim_control_set", "code", "seq_dim_control_set"),
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
    // code→id des 9 FK. Résout toutes les FK en une passe pour être compatible
    // avec le DDL courant, quelle que soit la vague d'origine.
    let create_mig = crate::schema::DDL_DIM_CONSOLIDATION
        .replace("dim_consolidation (", "dim_consolidation__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO dim_consolidation__mig
            (id, libelle, phase, exercice, perimeter_set, variant, presentation_currency,
             perimeter_period, rate_set, rate_period, ruleset_code,
             a_nouveau_consolidation_id, statut)
         SELECT c.id, c.libelle,
                (SELECT s.id  FROM dim_scenario_category s WHERE s.code     = c.phase),
                (SELECT pe.id FROM dim_period pe             WHERE pe.code   = c.exercice),
                (SELECT ps.id FROM dim_perimeter_set ps      WHERE ps.code   = c.perimeter_set),
                (SELECT v.id  FROM dim_variant v             WHERE v.code    = c.variant),
                (SELECT cu.id FROM dim_currency cu           WHERE cu.code_iso = c.presentation_currency),
                (SELECT pp.id FROM dim_period pp             WHERE pp.code   = c.perimeter_period),
                (SELECT r.id  FROM dim_rate_set r            WHERE r.code    = c.rate_set),
                (SELECT rp.id FROM dim_period rp             WHERE rp.code   = c.rate_period),
                (SELECT rs.id FROM dim_ruleset rs            WHERE rs.code   = c.ruleset_code),
                c.a_nouveau_consolidation_id, c.statut
         FROM dim_consolidation c;
         DROP TABLE dim_consolidation;
         ALTER TABLE dim_consolidation__mig RENAME TO dim_consolidation;",
    ))?;
    Ok(())
}

/// Migration in-place des **FK restantes** de `dim_consolidation` (chantier B1,
/// étape 3 — 2ᵉ vague) sur une base existante : convertit `exercice`,
/// `presentation_currency`, `perimeter_period`, `rate_period`, `ruleset_code`
/// du stockage **code (TEXT)** vers la **clé technique (id, INTEGER)**.
/// Idempotent (no-op si `exercice` est déjà entier).
///
/// À appeler **après** [`migrate_consolidation_fk_to_id`] (qui a déjà flippé
/// phase/perimeter_set/variant/rate_set) et après [`ensure_ids`].
/// Requiert `dim_period`, `dim_currency` et `dim_ruleset` avec leurs `id`.
pub fn migrate_consolidation_fk_to_id_v2(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_consolidation")? {
        return Ok(());
    }
    if column_is_int(con, "dim_consolidation", "exercice")? {
        return Ok(()); // déjà migrée
    }
    let create_mig = crate::schema::DDL_DIM_CONSOLIDATION
        .replace("dim_consolidation (", "dim_consolidation__mig2 (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO dim_consolidation__mig2
            (id, libelle, phase, exercice, perimeter_set, variant, presentation_currency,
             perimeter_period, rate_set, rate_period, ruleset_code,
             a_nouveau_consolidation_id, statut)
         SELECT c.id, c.libelle,
                c.phase,
                (SELECT p.id FROM dim_period p WHERE p.code = c.exercice),
                c.perimeter_set,
                c.variant,
                (SELECT cu.id FROM dim_currency cu WHERE cu.code_iso = c.presentation_currency),
                (SELECT pp.id FROM dim_period pp WHERE pp.code = c.perimeter_period),
                c.rate_set,
                (SELECT rp.id FROM dim_period rp WHERE rp.code = c.rate_period),
                (SELECT rs.id FROM dim_ruleset rs WHERE rs.code = c.ruleset_code),
                c.a_nouveau_consolidation_id, c.statut
         FROM dim_consolidation c;
         DROP TABLE dim_consolidation;
         ALTER TABLE dim_consolidation__mig2 RENAME TO dim_consolidation;",
    ))?;
    Ok(())
}

/// Migration in-place de `dim_entity` (chantier B1) : convertit
/// `devise_fonctionnelle` (TEXT code_iso → INTEGER dim_currency.id) et
/// `entite_parent` (TEXT code → INTEGER dim_entity.id auto-référence).
/// Idempotent (no-op si `devise_fonctionnelle` est déjà entier).
///
/// Requiert `dim_currency` avec ses `id` et `ensure_ids` sur `dim_entity`.
/// Le parent (M) doit avoir été résolu avant les filiales — garanti car on
/// lit la table d'origine entière (`dim_entity e`) et on résout avec
/// `WHERE code = e.entite_parent` (sous-requête sur la table originale).
pub fn migrate_entity_fk_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_entity")? {
        return Ok(());
    }
    if column_is_int(con, "dim_entity", "devise_fonctionnelle")? {
        return Ok(());
    }
    // `dim_entity__mig` est créée sans colonne `id` (DDL_DIM_ENTITY n'en a pas —
    // c'est `ensure_ids` qui la gère). On la recrée manuellement AVANT l'INSERT
    // pour conserver les ids existants : le self-join `entite_parent → ep.id`
    // lit la table **originale** dont les ids sont encore cohérents.
    let create_mig = crate::schema::DDL_DIM_ENTITY
        .replace("dim_entity (", "dim_entity__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         ALTER TABLE dim_entity__mig ADD COLUMN id INTEGER;
         INSERT INTO dim_entity__mig (code, libelle, devise_fonctionnelle, entite_parent, statut, id)
         SELECT e.code, e.libelle,
                (SELECT cu.id FROM dim_currency cu WHERE cu.code_iso = e.devise_fonctionnelle),
                (SELECT ep.id FROM dim_entity  ep WHERE ep.code      = e.entite_parent),
                e.statut,
                e.id
         FROM dim_entity e;
         DROP TABLE dim_entity;
         ALTER TABLE dim_entity__mig RENAME TO dim_entity;",
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
    // Détecte si methode est déjà entier (migration methode jouée avant) ou encore code.
    let methode_is_int = column_is_int(con, "sat_perimeter", "methode").unwrap_or(false);
    let methode_expr = if methode_is_int {
        "s.methode".to_string()
    } else {
        "(SELECT m.id FROM dim_method m WHERE m.code = s.methode)".to_string()
    };
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO sat_perimeter__mig
            (perimeter_set, entity, period, methode,
             pct_interet, pct_integration, entree, sortie)
         SELECT (SELECT p.id FROM dim_perimeter_set p WHERE p.code = s.perimeter_set),
                s.entity, s.period, {methode_expr},
                s.pct_interet, s.pct_integration, s.entree, s.sortie
         FROM sat_perimeter s;
         DROP TABLE sat_perimeter;
         ALTER TABLE sat_perimeter__mig RENAME TO sat_perimeter;",
    ))?;
    Ok(())
}

/// Migration in-place de `sat_flow_scheme_item.scheme` (option A, chantier B1)
/// sur une base **existante** : convertit la colonne du stockage **code (TEXT)**
/// vers la **clé technique (id, INTEGER)**. Idempotent (no-op si déjà entier).
///
/// `scheme` est dans la PK `(scheme, flow)` → **reconstruction de table** (cf.
/// [`migrate_sat_exchange_rate_fk_to_id`]). À appeler au démarrage serveur
/// **après** [`ensure_ids`] (`dim_flow_scheme` doit avoir son `id`). Seconde réf.
/// vers `dim_flow_scheme` (avec `dim_account.flow_scheme`) → rend la dimension
/// entièrement flippée (renommable).
pub fn migrate_sat_flow_scheme_item_scheme_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "sat_flow_scheme_item")? {
        return Ok(());
    }
    if column_is_int(con, "sat_flow_scheme_item", "scheme")? {
        return Ok(()); // déjà migrée
    }
    let create_mig = crate::schema::DDL_SAT_FLOW_SCHEME_ITEM
        .replace("sat_flow_scheme_item (", "sat_flow_scheme_item__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO sat_flow_scheme_item__mig
            (scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau)
         SELECT (SELECT fs.id FROM dim_flow_scheme fs WHERE fs.code = s.scheme),
                s.flow, s.taux_conversion, s.flux_ecart, s.flux_de_report, s.flux_a_nouveau
         FROM sat_flow_scheme_item s;
         DROP TABLE sat_flow_scheme_item;
         ALTER TABLE sat_flow_scheme_item__mig RENAME TO sat_flow_scheme_item;",
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

/// Migration B1 : bascule `dim_account.flow_scheme` du code (TEXT) vers la clé
/// technique (id, INTEGER) sur une base existante. Idempotent.
///
/// Comme `sous_classe`, c'est une colonne simple hors PK/UNIQUE : **add temp +
/// update + drop + rename**, qui préserve les colonnes custom runtime. La table
/// cible `dim_flow_scheme` doit déjà avoir son `id` (cf. [`ensure_ids`]).
pub fn migrate_account_flow_scheme_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_account")? {
        return Ok(());
    }
    if column_is_int(con, "dim_account", "flow_scheme")? {
        return Ok(()); // déjà migrée
    }
    con.execute_batch(
        "ALTER TABLE dim_account ADD COLUMN flow_scheme__b1 INTEGER;
         UPDATE dim_account SET flow_scheme__b1 = (
             SELECT fs.id FROM dim_flow_scheme fs WHERE fs.code = dim_account.flow_scheme
         ) WHERE dim_account.flow_scheme IS NOT NULL;
         ALTER TABLE dim_account DROP COLUMN flow_scheme;
         ALTER TABLE dim_account RENAME COLUMN flow_scheme__b1 TO flow_scheme;",
    )?;
    Ok(())
}

/// Migration B1 : bascule `sat_perimeter.methode` du code (TEXT) vers la clé
/// technique (id, INTEGER) sur une base existante. Idempotent.
///
/// `methode` est une colonne simple hors PK → **add temp + update + drop + rename**.
/// La table cible `dim_method` doit déjà avoir son `id` (cf. [`ensure_ids`]).
/// Rend la dimension `dim_method` entièrement flippée (renommable).
pub fn migrate_sat_perimeter_methode_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "sat_perimeter")? {
        return Ok(());
    }
    if column_is_int(con, "sat_perimeter", "methode")? {
        return Ok(()); // déjà migrée
    }
    con.execute_batch(
        "ALTER TABLE sat_perimeter ADD COLUMN methode__b1 INTEGER;
         UPDATE sat_perimeter SET methode__b1 = (
             SELECT m.id FROM dim_method m WHERE m.code = sat_perimeter.methode
         ) WHERE sat_perimeter.methode IS NOT NULL;
         ALTER TABLE sat_perimeter DROP COLUMN methode;
         ALTER TABLE sat_perimeter RENAME COLUMN methode__b1 TO methode;",
    )?;
    Ok(())
}

/// Migration B1 : bascule `dim_ruleset_item.ruleset_code` du code (TEXT) vers la
/// clé technique (id, INTEGER) sur une base existante. Idempotent.
///
/// `ruleset_code` fait partie de la PK composite → reconstruction de la table
/// (même patron que `sat_flow_scheme_item`). Rend la dimension `dim_ruleset`
/// entièrement flippée (renommable).
pub fn migrate_ruleset_item_fk_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_ruleset_item")? {
        return Ok(());
    }
    if column_is_int(con, "dim_ruleset_item", "ruleset_code")? {
        return Ok(()); // déjà migrée
    }
    let create_mig = crate::schema::DDL_DIM_RULESET_ITEM
        .replace("dim_ruleset_item (", "dim_ruleset_item__mig (");
    con.execute_batch(&format!(
        "{create_mig}
         INSERT INTO dim_ruleset_item__mig (ruleset_code, ordre, rule_code)
         SELECT (SELECT rs.id FROM dim_ruleset rs WHERE rs.code = i.ruleset_code),
                i.ordre, i.rule_code
         FROM dim_ruleset_item i;
         DROP TABLE dim_ruleset_item;
         ALTER TABLE dim_ruleset_item__mig RENAME TO dim_ruleset_item;",
    ))?;
    Ok(())
}

/// Dote `dim_characteristic_attribute` d'un `id` technique (séquence dédiée).
/// Idempotent. À appeler après [`ensure_ids`] (qui ne couvre pas ce registre car
/// il n'a pas de colonne `code` unique — sa clé est composite).
pub fn ensure_characteristic_attribute_ids(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_characteristic_attribute")? {
        return Ok(());
    }
    if column_exists(con, "dim_characteristic_attribute", "id")? {
        return Ok(()); // déjà migrée
    }
    let seq = "seq_dim_characteristic_attribute";
    con.execute(&format!("CREATE SEQUENCE IF NOT EXISTS {seq} START 1"), [])?;
    con.execute(
        "ALTER TABLE dim_characteristic_attribute ADD COLUMN id INTEGER",
        [],
    )?;
    con.execute(
        &format!("UPDATE dim_characteristic_attribute SET id = nextval('{seq}')"),
        [],
    )?;
    con.execute(
        &format!(
            "ALTER TABLE dim_characteristic_attribute ALTER COLUMN id \
             SET DEFAULT nextval('{seq}')"
        ),
        [],
    )?;
    Ok(())
}

/// Migration B1 étape 5 : renomme les tables de valeurs N1 de `car_<code>` →
/// `car_<id>` pour chaque caractéristique. Idempotent : skip si `car_<id>` existe
/// déjà ou si `car_<code>` n'existe pas. À appeler après [`ensure_ids`].
pub fn migrate_characteristic_tables_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_characteristic")? {
        return Ok(());
    }
    let chars: Vec<(String, i64)> = {
        let mut stmt = con.prepare("SELECT code, id FROM dim_characteristic")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<duckdb::Result<_>>()?
    };
    for (code, id) in chars {
        let old_name = format!("car_{code}");
        let new_name = format!("car_{id}");
        if table_exists(con, &new_name)? {
            continue; // déjà renommée
        }
        if !table_exists(con, &old_name)? {
            continue; // jamais créée ou déjà renommée
        }
        con.execute(&format!("ALTER TABLE {old_name} RENAME TO {new_name}"), [])?;
    }
    Ok(())
}

/// Migration B1 étape 5 : renomme les tables de listes de valeurs de
/// `lst_<code>` → `lst_<id>`. Idempotent. À appeler après [`ensure_ids`].
pub fn migrate_value_list_tables_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_value_list")? {
        return Ok(());
    }
    let lists: Vec<(String, i64)> = {
        let mut stmt = con.prepare("SELECT code, id FROM dim_value_list")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.collect::<duckdb::Result<_>>()?
    };
    for (code, id) in lists {
        let old_name = format!("lst_{code}");
        let new_name = format!("lst_{id}");
        if table_exists(con, &new_name)? {
            continue; // déjà renommée
        }
        if !table_exists(con, &old_name)? {
            continue;
        }
        con.execute(&format!("ALTER TABLE {old_name} RENAME TO {new_name}"), [])?;
    }
    Ok(())
}

/// Migration B1 étape 4 : bascule les 10 colonnes dimensionnelles de `fact_entry`
/// du stockage **code (TEXT)** vers la **clé technique (id, INTEGER)**. Idempotent.
///
/// `fact_entry` est une **donnée dérivée** (produite par le pipeline) : on peut la
/// reconstruire à zéro sans perte métier. La migration **tronque** la table et la
/// recrée au schéma cible ; l'utilisateur doit relancer le pipeline après cette
/// migration.
///
/// Met également à jour la vue `v_flow_behavior` vers sa définition id-aware
/// (expose `flux_*_id` en INTEGER à la place des codes TEXT).
///
/// À appeler au démarrage serveur **après** [`ensure_ids`] (toutes les dimensions
/// doivent déjà avoir leur `id`).
pub fn migrate_fact_entry_to_ids(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "fact_entry")? {
        return Ok(());
    }
    if column_is_int(con, "fact_entry", "entity")? {
        return Ok(()); // déjà migrée
    }
    // Reconstruction au schéma cible (INTEGER). Les données sont dérivées →
    // truncate propre, pas de réinjection. L'utilisateur relance le pipeline.
    let create_mig = crate::schema::DDL_FACT_ENTRY
        .replace("fact_entry (", "fact_entry__b1 (");
    con.execute_batch(&format!(
        "{create_mig}
         DROP TABLE fact_entry;
         ALTER TABLE fact_entry__b1 RENAME TO fact_entry;"
    ))?;
    // Réapplique les colonnes custom (colonnes physiques x{id}, B1 étape 10).
    let customs = crate::dimensions::load_customs(con).unwrap_or_default();
    for d in &customs {
        let _ = con.execute(&format!("ALTER TABLE fact_entry ADD COLUMN {} TEXT", d.col), []);
    }
    // Mise à jour de la vue v_flow_behavior → définition id-aware.
    let _ = con.execute_batch("DROP VIEW IF EXISTS v_flow_behavior");
    con.execute_batch(crate::schema::DDL_V_FLOW_BEHAVIOR)?;
    Ok(())
}

/// Migration B1 étape 8 : bascule `app_config.pivot_currency` (code TEXT) vers
/// `app_config.pivot_currency_id` (id INTEGER). Idempotent.
///
/// Après cette migration :
/// - `ConvertParams::load_params` préfère `pivot_currency_id` (résolution id→code) ;
/// - `scan_json_blockers` n'a plus à bloquer le renommage d'une devise : l'id est
///   immunisé au renommage du code. La `currency` devient pleinement renommable.
pub fn migrate_pivot_currency_to_id(con: &Connection) -> duckdb::Result<()> {
    // Déjà fait ?
    let done: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM app_config WHERE key = 'pivot_currency_id'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if done {
        return Ok(());
    }
    // Lire le code courant.
    let code: Option<String> = con
        .query_row(
            "SELECT value FROM app_config WHERE key = 'pivot_currency'",
            [],
            |r| r.get(0),
        )
        .ok();
    let Some(code) = code else {
        return Ok(()); // pas de pivot_currency configuré
    };
    // Résoudre en id.
    let id: i64 = con.query_row(
        "SELECT id FROM dim_currency WHERE code_iso = ?",
        [&code],
        |r| r.get(0),
    )?;
    con.execute(
        "INSERT INTO app_config (key, value) VALUES ('pivot_currency_id', ?)",
        [id.to_string()],
    )?;
    Ok(())
}

/// Migration B1 étape 9 : renomme les colonnes N2 des tables de valeurs de
/// `<attr_name>` → `c<attr_id>`. Idempotent.
///
/// Après cette migration, les colonnes physiques sur `car_<char_id>` ne dépendent
/// plus du code (nom mutable) de l'attribut : un renommage de code attribut ne
/// nécessite plus d'`ALTER TABLE RENAME COLUMN`.
pub fn migrate_attribute_columns_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_characteristic_attribute")? {
        return Ok(());
    }
    let mut stmt = con.prepare(
        "SELECT dc.id, dca.id, dca.name \
         FROM dim_characteristic_attribute dca \
         JOIN dim_characteristic dc ON dc.code = dca.characteristic_code \
         ORDER BY dc.id, dca.name",
    )?;
    let rows: Vec<(i64, i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .flatten()
        .collect();
    for (char_id, attr_id, attr_name) in rows {
        let car_table = crate::characteristics::value_table(char_id);
        if !table_exists(con, &car_table)? {
            continue; // table absente (état partiel)
        }
        let new_col = format!("c{attr_id}");
        if column_exists(con, &car_table, &new_col)? {
            continue; // déjà migré
        }
        if !column_exists(con, &car_table, &attr_name)? {
            continue; // colonne source absente (état incohérent, on ignore)
        }
        con.execute(
            &format!("ALTER TABLE {car_table} RENAME COLUMN \"{attr_name}\" TO \"{new_col}\""),
            [],
        )?;
    }
    Ok(())
}

/// Migration B1 étape 10 : renomme les colonnes custom `<name>` → `x<id>` sur
/// `fact_entry` et `stg_entry`. Idempotent.
///
/// Après `migrate_fact_entry_to_ids` (étape 4), les colonnes custom de
/// `fact_entry` portent encore le nom API si la table a été reconstruite avec
/// l'ancien code. Cette migration les passe au nom physique stable. Idem pour
/// `stg_entry`.
///
/// À appeler au démarrage serveur **après** [`migrate_fact_entry_to_ids`] et
/// [`ensure_ids`] (`dim_custom_dimension` doit déjà avoir ses `id`).
pub fn migrate_custom_dimension_columns_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_custom_dimension")? {
        return Ok(());
    }
    let customs: Vec<(String, i64)> = {
        let mut stmt =
            con.prepare("SELECT name, id FROM dim_custom_dimension ORDER BY name")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<duckdb::Result<_>>()?
    };
    for (name, id) in customs {
        let new_col = format!("x{id}");
        for table in &["fact_entry", "stg_entry"] {
            if !table_exists(con, table)? {
                continue;
            }
            if column_exists(con, table, &new_col)? {
                continue; // déjà migré
            }
            if !column_exists(con, table, &name)? {
                continue; // colonne absente (base fraîche ou déjà renommée)
            }
            con.execute(
                &format!("ALTER TABLE {table} RENAME COLUMN \"{name}\" TO \"{new_col}\""),
                [],
            )?;
        }
    }
    Ok(())
}

/// Migration B1 étape 11 : s'assure que `dim_custom_reference` possède une
/// colonne `id` avec sa séquence, et backfille les lignes existantes.
/// Idempotent (skip si la colonne existe déjà).
///
/// Les refs natives reçoivent un `id` comme les customs ; cela ne change pas
/// leur colonne physique sur la master data hôte (qui reste le nom d'origine).
pub fn ensure_custom_reference_ids(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_custom_reference")? {
        return Ok(());
    }
    if column_exists(con, "dim_custom_reference", "id")? {
        return Ok(()); // déjà migré
    }
    con.execute_batch(
        "CREATE SEQUENCE IF NOT EXISTS seq_dim_custom_reference START 1 INCREMENT 1;",
    )?;
    con.execute(
        "ALTER TABLE dim_custom_reference ADD COLUMN id INTEGER",
        [],
    )?;
    con.execute(
        "UPDATE dim_custom_reference SET id = nextval('seq_dim_custom_reference') WHERE id IS NULL",
        [],
    )?;
    Ok(())
}

/// Migration B1 étape 11 : renomme les colonnes custom `<column_name>` → `r<id>`
/// sur la master data hôte (`dim_<host>`), pour chaque référence non-native.
/// Idempotent.
///
/// Les références natives (colonne du DDL statique comme `sous_classe`,
/// `entite_parent`) gardent leur nom d'origine — elles ne sont pas renommées.
///
/// À appeler **après** `ensure_custom_reference_ids`.
pub fn migrate_custom_reference_columns_to_id(con: &Connection) -> duckdb::Result<()> {
    if !table_exists(con, "dim_custom_reference")? {
        return Ok(());
    }
    let custom_refs: Vec<(String, String, i64)> = {
        let mut stmt = con.prepare(
            "SELECT host_dimension, column_name, id \
             FROM dim_custom_reference \
             WHERE native = FALSE AND id IS NOT NULL \
             ORDER BY host_dimension, column_name",
        )?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<duckdb::Result<_>>()?
    };
    for (host_dim, col_name, id) in custom_refs {
        let phys_col = format!("r{id}");
        // Résoudre la table hôte via le registre statique des master data.
        let host_table = match crate::references::dimension_master(&host_dim) {
            Some((t, _)) => t,
            None => continue,
        };
        if !table_exists(con, host_table)? {
            continue;
        }
        if column_exists(con, host_table, &phys_col)? {
            continue; // déjà migré
        }
        if !column_exists(con, host_table, &col_name)? {
            continue; // absente (reset récent ou déjà renommée)
        }
        con.execute(
            &format!(
                "ALTER TABLE {host_table} RENAME COLUMN \"{col_name}\" TO \"{phys_col}\""
            ),
            [],
        )?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Étape 13 : PK `id` réelle sur les dimensions
// ─────────────────────────────────────────────────────────────────────────────

/// Retourne le nom de la colonne « code » pour une table donnée.
fn code_col_for(table: &str) -> &str {
    for &(t, c, _) in SURROGATE_DIMS {
        if t == table {
            return c;
        }
    }
    "code"
}

/// Reconstruit chaque dimension de [`SURROGATE_DIMS`] avec `id … PRIMARY KEY`
/// et `code … UNIQUE NOT NULL`. **Idempotent** : saute les tables déjà migrées
/// (celles où `id` est déjà la PK).
///
/// Patron : CREATE TABLE __mig (id PK, code UNIQUE, …) → INSERT … SELECT →
/// DROP old → RENAME __mig → SET DEFAULT nextval → CHECKPOINT.
///
/// Appelée au démarrage du serveur, après `ensure_ids`.
pub fn migrate_dims_pk_to_id(con: &duckdb::Connection) -> duckdb::Result<()> {
    for &(table, _code_col, seq) in SURROGATE_DIMS {
        if !table_exists(con, table)? {
            continue;
        }
        // Idempotence : si id est déjà la PK, rien à faire.
        if id_is_pk(con, table)? {
            continue;
        }
        // Collecter les colonnes actuelles, en excluant `id` (ajouté par ensure_ids).
        let all_cols = table_columns(con, table)?;
        let cols: Vec<String> = all_cols.into_iter().filter(|c| c != "id").collect();
        if cols.is_empty() {
            continue;
        }
        let col_list = cols.join(", ");
        let mig = format!("{table}__mig");

        // Construire le CREATE avec id PK + code UNIQUE + les autres colonnes.
        let code_col = code_col_for(table);
        let mut create_cols = format!("id INTEGER PRIMARY KEY");
        for c in &cols {
            if c == code_col {
                create_cols.push_str(&format!(", {c} TEXT UNIQUE NOT NULL"));
            } else {
                let dtype = column_type(con, table, c).unwrap_or_default();
                create_cols.push_str(&format!(", {c} {dtype}"));
            }
        }

        // Séquence : créer si absente (idempotent).
        con.execute(&format!("CREATE SEQUENCE IF NOT EXISTS {seq} START 1"), [])?;

        // Exécuter la reconstruction en une seule transaction.
        con.execute_batch(&format!(
            "CREATE TABLE {mig} ({create_cols});
             INSERT INTO {mig} (id, {col_list}) SELECT id, {col_list} FROM {table};
             DROP TABLE {table};
             ALTER TABLE {mig} RENAME TO {table};
             ALTER TABLE {table} ALTER COLUMN id SET DEFAULT nextval('{seq}');"
        ))?;
    }
    con.execute("CHECKPOINT", [])?;
    Ok(())
}

/// Vérifie si la colonne `id` est la PRIMARY KEY de la table.
fn id_is_pk(con: &duckdb::Connection, table: &str) -> duckdb::Result<bool> {
    // DuckDB : pragma_table_info retourne pk=true/false par colonne.
    let is_pk: bool = con.query_row(
        &format!(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('{table}') \
             WHERE name = 'id' AND pk"
        ),
        [],
        |r| r.get(0),
    )?;
    Ok(is_pk)
}

/// Liste les colonnes d'une table (ordre ordinal).
fn table_columns(con: &duckdb::Connection, table: &str) -> duckdb::Result<Vec<String>> {
    let mut stmt = con.prepare(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'main' AND table_name = ? \
         ORDER BY ordinal_position",
    )?;
    let cols: Vec<String> = stmt
        .query_map(duckdb::params![table], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(cols)
}

/// Retourne le type SQL d'une colonne.
fn column_type(con: &duckdb::Connection, table: &str, col: &str) -> duckdb::Result<String> {
    con.query_row(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema = 'main' AND table_name = ? AND column_name = ?",
        duckdb::params![table, col],
        |r| r.get(0),
    )
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
             INSERT INTO dim_rate_set (code, libelle) VALUES ('RATES','Taux réels');
             INSERT INTO dim_period (code, libelle) VALUES ('2024','Exercice 2024');
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);",
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

        // Les 9 FK sont désormais des entiers = id de leur cible ; id préservé.
        let (phase, perim, variant, rate, exercice, pres, id): (i64, i64, i64, i64, i64, i64, i64) = con
            .query_row(
                "SELECT phase, perimeter_set, variant, rate_set, exercice, \
                        presentation_currency, id FROM dim_consolidation",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .expect("FK migrées en entiers");
        assert_eq!(id, 1, "id préservé");
        assert_eq!(phase,    crate::resolve::resolve_id(&con, "dim_scenario_category", "REEL").unwrap().unwrap());
        assert_eq!(perim,    crate::resolve::resolve_id(&con, "dim_perimeter_set", "PERIM_REEL").unwrap().unwrap());
        assert_eq!(variant,  crate::resolve::resolve_id(&con, "dim_variant", "BASE").unwrap().unwrap());
        assert_eq!(rate,     crate::resolve::resolve_id(&con, "dim_rate_set", "RATES").unwrap().unwrap());
        assert_eq!(exercice, crate::resolve::resolve_id(&con, "dim_period", "2024").unwrap().unwrap());
        assert_eq!(pres,     crate::resolve::resolve_id(&con, "dim_currency", "EUR").unwrap().unwrap());

        // Idempotent : second passage no-op (variant déjà entier).
        migrate_consolidation_fk_to_id(&con).expect("migration #2");
    }

    #[test]
    fn migrate_consolidation_fk_to_id_v2_convertit_les_codes_restants() {
        // Simule une base en état intermédiaire : après v1 (phase/variant/perimeter_set/
        // rate_set en INTEGER) mais avant v2 (exercice/presentation_currency/etc. encore TEXT).
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_scenario_category (code, libelle) VALUES ('REEL','Réel');
             INSERT INTO dim_variant (code, libelle) VALUES ('BASE','Base');
             INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PERIM_REEL','Périmètre');
             INSERT INTO dim_rate_set (code, libelle) VALUES ('RATES','Taux réels');
             INSERT INTO dim_period (code, libelle) VALUES ('2024','Exercice 2024');
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);",
        )
        .unwrap();

        // État intermédiaire : phase/variant/perimeter_set/rate_set en id (v1 faite),
        // exercice/presentation_currency/perimeter_period/rate_period en TEXT (v2 à faire).
        con.execute_batch(
            "DROP TABLE dim_consolidation;
             CREATE TABLE dim_consolidation (
                id INTEGER PRIMARY KEY, libelle TEXT,
                phase INTEGER, exercice TEXT,
                perimeter_set INTEGER, variant INTEGER,
                presentation_currency TEXT, perimeter_period TEXT,
                rate_set INTEGER, rate_period TEXT, ruleset_code TEXT,
                a_nouveau_consolidation_id INTEGER, statut TEXT,
                UNIQUE (phase, exercice, perimeter_set, variant, presentation_currency));
             INSERT INTO dim_consolidation VALUES
                (1,'Réel',
                 (SELECT id FROM dim_scenario_category WHERE code='REEL'), '2024',
                 (SELECT id FROM dim_perimeter_set WHERE code='PERIM_REEL'),
                 (SELECT id FROM dim_variant WHERE code='BASE'),
                 'EUR','2024',
                 (SELECT id FROM dim_rate_set WHERE code='RATES'),
                 '2024', NULL, NULL, 'ouvert');",
        )
        .unwrap();

        migrate_consolidation_fk_to_id_v2(&con).expect("migration v2");

        let (ex, pres, pp, rp, ruleset, id): (i64, i64, i64, i64, Option<i64>, i64) = con
            .query_row(
                "SELECT exercice, presentation_currency, perimeter_period, \
                        rate_period, ruleset_code, id FROM dim_consolidation",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .expect("FK migrées en entiers");
        assert_eq!(id, 1, "id préservé");
        assert_eq!(ex, crate::resolve::resolve_id(&con, "dim_period", "2024").unwrap().unwrap());
        assert_eq!(pres, crate::resolve::resolve_id(&con, "dim_currency", "EUR").unwrap().unwrap());
        assert_eq!(pp, crate::resolve::resolve_id(&con, "dim_period", "2024").unwrap().unwrap());
        assert_eq!(rp, crate::resolve::resolve_id(&con, "dim_period", "2024").unwrap().unwrap());
        assert!(ruleset.is_none(), "ruleset_code NULL préservé");

        // Idempotent.
        migrate_consolidation_fk_to_id_v2(&con).expect("migration v2 #2");
    }

    #[test]
    fn migrate_entity_fk_to_id_convertit_les_codes_en_id() {
        // Simule une base existante : dim_entity avec devise_fonctionnelle et
        // entite_parent en TEXT. Vérifie que la migration convertit les deux en id.
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('EUR','Euro',2);
             INSERT INTO dim_currency (code_iso, libelle, decimales) VALUES ('USD','Dollar',2);",
        )
        .unwrap();

        // Rétrograde dim_entity à l'ancien schéma (TEXT) + id (ensure_ids l'avait ajouté).
        con.execute_batch(
            "DROP TABLE dim_entity;
             CREATE TABLE dim_entity (
                code TEXT PRIMARY KEY, libelle TEXT,
                devise_fonctionnelle TEXT, entite_parent TEXT, statut TEXT);
             ALTER TABLE dim_entity ADD COLUMN id INTEGER DEFAULT nextval('seq_dim_entity');
             INSERT INTO dim_entity (code,libelle,devise_fonctionnelle,entite_parent,statut)
                VALUES ('M','Mère','EUR',NULL,'actif');
             INSERT INTO dim_entity (code,libelle,devise_fonctionnelle,entite_parent,statut)
                VALUES ('A','Filiale A','USD','M','actif');",
        )
        .unwrap();

        migrate_entity_fk_to_id(&con).expect("migration entity");

        let (df_m, ep_m, df_a, ep_a): (i64, Option<i64>, i64, Option<i64>) = {
            let m_id: i64 = con.query_row("SELECT id FROM dim_entity WHERE code='M'", [], |r| r.get(0)).unwrap();
            let (df_m, ep_m) = con.query_row("SELECT devise_fonctionnelle, entite_parent FROM dim_entity WHERE code='M'", [], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,Option<i64>>(1)?))).unwrap();
            let (df_a, ep_a) = con.query_row("SELECT devise_fonctionnelle, entite_parent FROM dim_entity WHERE code='A'", [], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,Option<i64>>(1)?))).unwrap();
            let _ = m_id;
            (df_m, ep_m, df_a, ep_a)
        };
        assert_eq!(df_m, crate::resolve::resolve_id(&con, "dim_currency", "EUR").unwrap().unwrap());
        assert!(ep_m.is_none(), "parent de M est NULL");
        assert_eq!(df_a, crate::resolve::resolve_id(&con, "dim_currency", "USD").unwrap().unwrap());
        assert_eq!(ep_a, Some(crate::resolve::resolve_id(&con, "dim_entity", "M").unwrap().unwrap()));

        // Idempotent.
        migrate_entity_fk_to_id(&con).expect("migration entity #2");
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
            "INSERT INTO dim_perimeter_set (code, libelle) VALUES ('PERIM_REEL','Périmètre');
             INSERT INTO dim_method (code, libelle) VALUES ('globale','Intégration globale');",
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
    fn migrate_sat_perimeter_methode_to_id_convertit_les_codes() {
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_method (code, libelle) VALUES ('globale','Globale'),('prop','Prop');",
        )
        .unwrap();
        // Simule une base existante avec methode en TEXT.
        con.execute_batch(
            "DROP TABLE sat_perimeter;
             CREATE TABLE sat_perimeter (
                perimeter_set INTEGER, entity TEXT, period TEXT, methode TEXT,
                pct_interet DECIMAL(10,4), pct_integration DECIMAL(10,4),
                entree BOOLEAN DEFAULT FALSE, sortie BOOLEAN DEFAULT FALSE,
                PRIMARY KEY (perimeter_set, entity, period)
             );
             INSERT INTO sat_perimeter VALUES (1,'E1','2024','globale',1.0,1.0,FALSE,FALSE);
             INSERT INTO sat_perimeter VALUES (1,'E2','2024','prop',0.5,0.5,FALSE,FALSE);",
        )
        .unwrap();

        migrate_sat_perimeter_methode_to_id(&con).expect("migration");

        let methode_e1: i64 = con
            .query_row(
                "SELECT methode FROM sat_perimeter WHERE entity = 'E1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let globale_id = crate::resolve::resolve_id(&con, "dim_method", "globale")
            .unwrap()
            .unwrap();
        assert_eq!(methode_e1, globale_id, "methode E1 migrée en id");

        let methode_e2: i64 = con
            .query_row(
                "SELECT methode FROM sat_perimeter WHERE entity = 'E2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let prop_id = crate::resolve::resolve_id(&con, "dim_method", "prop").unwrap().unwrap();
        assert_eq!(methode_e2, prop_id, "methode E2 migrée en id");

        // Idempotent.
        migrate_sat_perimeter_methode_to_id(&con).expect("migration #2");
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
    fn migrate_account_flow_scheme_to_id_convertit_les_codes() {
        // Base existante : dim_account avec flow_scheme TEXT (codes), + une colonne
        // custom runtime (patron B) qui doit survivre à la migration.
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_flow_scheme (code, libelle) VALUES
                ('BILAN','Schéma bilan'),('RESULTAT','Schéma résultat');
             ALTER TABLE dim_account ADD COLUMN compte_parent TEXT;
             DROP TABLE dim_account;
             CREATE TABLE dim_account (
                code TEXT PRIMARY KEY, libelle TEXT, classe TEXT,
                sous_classe TEXT, flow_scheme TEXT, compte_parent TEXT
             );
             INSERT INTO dim_account VALUES
                ('10','Cap','bilan',NULL,'BILAN','ROOT'),
                ('101','Capital','bilan',NULL,NULL,'10');",
        )
        .unwrap();

        migrate_account_flow_scheme_to_id(&con).expect("migration");

        let (fs, parent): (Option<i64>, String) = con
            .query_row(
                "SELECT flow_scheme, compte_parent FROM dim_account WHERE code='10'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent, "ROOT", "colonne custom préservée");
        assert_eq!(
            fs,
            Some(
                crate::resolve::resolve_id(&con, "dim_flow_scheme", "BILAN")
                    .unwrap()
                    .unwrap()
            ),
            "flow_scheme résolu en id"
        );
        // La ligne à flow_scheme NULL reste NULL (option b : pas d'id inventé).
        let fs_null: Option<i64> = con
            .query_row(
                "SELECT flow_scheme FROM dim_account WHERE code='101'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(fs_null.is_none(), "flow_scheme NULL préservé");
        // Idempotent.
        migrate_account_flow_scheme_to_id(&con).expect("migration #2");
    }

    #[test]
    fn migrate_sat_flow_scheme_item_scheme_to_id_convertit_les_codes_en_id() {
        // Simule une base existante : sat_flow_scheme_item avec scheme en TEXT
        // (codes). On vérifie que la migration la reconstruit avec scheme en id.
        let con = setup();
        con.execute_batch(
            "INSERT INTO dim_flow_scheme (code, libelle) VALUES ('BILAN','Schéma bilan');",
        )
        .unwrap();

        // Rétrograde sat_flow_scheme_item à l'ancien schéma (scheme TEXT, code).
        con.execute_batch(
            "DROP TABLE sat_flow_scheme_item;
             CREATE TABLE sat_flow_scheme_item (
                scheme TEXT, flow TEXT, taux_conversion TEXT,
                flux_ecart TEXT, flux_de_report TEXT, flux_a_nouveau TEXT,
                PRIMARY KEY (scheme, flow)
             );
             INSERT INTO sat_flow_scheme_item VALUES
                ('BILAN','F99','close_n',NULL,'F99',NULL);",
        )
        .unwrap();

        migrate_sat_flow_scheme_item_scheme_to_id(&con).expect("migration");

        let (scheme_id, flow): (i64, String) = con
            .query_row(
                "SELECT scheme, flow FROM sat_flow_scheme_item",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("ligne présente après migration");
        assert_eq!(flow, "F99", "autres colonnes préservées");
        assert_eq!(
            scheme_id,
            crate::resolve::resolve_id(&con, "dim_flow_scheme", "BILAN")
                .unwrap()
                .unwrap(),
            "scheme résolu en id"
        );
        // Idempotent.
        migrate_sat_flow_scheme_item_scheme_to_id(&con).expect("migration #2");
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

    #[test]
    fn migrate_dims_pk_to_id_reconstruit_avec_id_pk() {
        let con = setup();
        crate::seed_all(&con).expect("seed_all");

        // Avant migration : code est PK (ensure_ids a ajouté id mais sans PK).
        assert!(!id_is_pk(&con, "dim_account").unwrap(),
            "avant étape 13, id ne doit pas être PK");

        migrate_dims_pk_to_id(&con).expect("migrate_dims_pk_to_id");

        // Après migration : id est PK sur toutes les dimensions.
        for &(table, _, _) in SURROGATE_DIMS {
            if !table_exists(&con, table).unwrap() {
                continue;
            }
            assert!(id_is_pk(&con, table).unwrap(),
                "{table} doit avoir id comme PK après étape 13");
        }

        // Les données sont préservées.
        let n: i64 = con
            .query_row("SELECT COUNT(*) FROM dim_account", [], |r| r.get(0))
            .unwrap();
        assert!(n > 0, "les comptes doivent être préservés après reconstruction");

        // Idempotent : re-appel ne fait rien.
        migrate_dims_pk_to_id(&con).expect("migrate_dims_pk_to_id idempotent");
    }
}
