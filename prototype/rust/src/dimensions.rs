//! Registre central des dimensions.
//!
//! Décrit les dimensions connues du moteur (built-in) et celles ajoutées par
//! l'utilisateur (custom). Trois catégories suffisent à dériver toutes les
//! règles de propagation / pilotage / nullabilité / grain de clôture :
//!
//! - [`DimCategory::Fixed`]      : propagées, non pilotables, non nullables,
//!                                 dans le grain des clôtures.
//! - [`DimCategory::Active`]     : propagées, pilotables, non nullables,
//!                                 dans le grain des clôtures.
//! - [`DimCategory::Analytical`] : propagées, pilotables, nullables,
//!                                 hors grain des clôtures.
//!
//! Les dimensions custom sont toujours `Analytical` (et donc nullables).
//!
//! Le registre ne contient QUE les vraies dimensions : `level` (niveau de
//! stockage) et `amount` (mesure agrégée) restent gérés séparément.

use duckdb::Connection;

// ─────────────────────────────────────────────────────────────────────────────
//  Catégories et définitions
// ─────────────────────────────────────────────────────────────────────────────

/// Catégorie d'une dimension — porte les règles de dérivation ci-dessous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimCategory {
    Fixed,
    Active,
    Analytical,
}

/// Définition d'une dimension.
#[derive(Debug, Clone)]
pub struct DimDef {
    /// Nom technique de la colonne (`scenario`, `partner`, …).
    pub name: String,
    /// Catégorie (Fixed / Active / Analytical).
    pub category: DimCategory,
    /// `true` si ajoutée par l'utilisateur (custom).
    pub custom: bool,
    /// Libellé UI.
    pub label: String,
}

impl DimDef {
    /// Une dimension du registre est toujours propagée (ni `level`, ni
    /// `amount` n'apparaissent ici).
    pub fn propagated(&self) -> bool {
        true
    }

    /// Pilotable = Active ou Analytical (les Fixed sont figées).
    pub fn pilotable(&self) -> bool {
        matches!(self.category, DimCategory::Active | DimCategory::Analytical)
    }

    /// Nullable = Analytical uniquement (les customs le sont par construction).
    pub fn nullable(&self) -> bool {
        matches!(self.category, DimCategory::Analytical)
    }

    /// Appartient au grain de reconstruction des clôtures = Fixed ou Active.
    pub fn in_closure_grain(&self) -> bool {
        matches!(self.category, DimCategory::Fixed | DimCategory::Active)
    }
}

/// Liste des dimensions built-in, dans l'ordre canonique des colonnes de
/// `fact_entry` / `stg_entry` (ordre du CSV d'entrée).
///
/// IMPORTANT : cet ordre est figé — il correspond à l'ordre des colonnes dans
/// `fact_entry` et garantit que le SQL généré pour les 12 colonnes builtin
/// reste identique au SQL statique historique (test golden).
pub fn builtin_dims() -> Vec<DimDef> {
    vec![
        // Fixed
        DimDef { name: "scenario".into(),     category: DimCategory::Fixed,      custom: false, label: "Définition de consolidation".into() },
        // Active
        DimDef { name: "entity".into(),       category: DimCategory::Active,     custom: false, label: "Entité".into() },
        // Fixed
        DimDef { name: "entry_period".into(), category: DimCategory::Fixed,      custom: false, label: "Exercice".into() },
        DimDef { name: "period".into(),       category: DimCategory::Fixed,      custom: false, label: "Période".into() },
        // Active
        DimDef { name: "account".into(),      category: DimCategory::Active,     custom: false, label: "Compte".into() },
        DimDef { name: "flow".into(),         category: DimCategory::Active,     custom: false, label: "Flux".into() },
        // Fixed
        DimDef { name: "currency".into(),     category: DimCategory::Fixed,      custom: false, label: "Devise".into() },
        // Active
        DimDef { name: "nature".into(),       category: DimCategory::Active,     custom: false, label: "Nature".into() },
        // Analytical
        DimDef { name: "partner".into(),      category: DimCategory::Analytical, custom: false, label: "Partenaire".into() },
        DimDef { name: "share".into(),        category: DimCategory::Analytical, custom: false, label: "Quote-part".into() },
        DimDef { name: "analysis".into(),     category: DimCategory::Analytical, custom: false, label: "Analyse 1".into() },
        DimDef { name: "analysis2".into(),    category: DimCategory::Analytical, custom: false, label: "Analyse 2".into() },
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
//  Chargement runtime
// ─────────────────────────────────────────────────────────────────────────────

/// Charge toutes les dimensions : built-in + custom (depuis `dim_custom_dimension`).
pub fn load_all(con: &Connection) -> Result<Vec<DimDef>, duckdb::Error> {
    let mut dims = builtin_dims();
    let customs = load_customs(con)?;
    dims.extend(customs);
    Ok(dims)
}

/// Charge uniquement les dimensions custom depuis `dim_custom_dimension`.
///
/// Retourne `Vec::new()` si la table n'existe pas encore (cas du premier
/// démarrage avant création du schéma).
pub fn load_customs(con: &Connection) -> Result<Vec<DimDef>, duckdb::Error> {
    // On teste d'abord l'existence de la table : au tout premier démarrage,
    // `create_schema` l'appelle AVANT d'avoir exécuté le DDL.
    let exists: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = 'dim_custom_dimension'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt = con.prepare("SELECT name, label FROM dim_custom_dimension ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(DimDef {
            name: row.get::<_, String>(0)?,
            category: DimCategory::Analytical,
            custom: true,
            label: row.get::<_, String>(1)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Sélecteurs
// ─────────────────────────────────────────────────────────────────────────────

/// Retourne la liste des noms propagés (toutes les dims du registre).
pub fn propagated_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().map(|d| d.name.as_str()).collect()
}

/// Retourne la liste des noms pilotables (Active + Analytical).
pub fn pilotable_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().filter(|d| d.pilotable()).map(|d| d.name.as_str()).collect()
}

/// Retourne les noms appartenant au grain de reconstruction des clôtures
/// (Fixed + Active).
pub fn closure_grain_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().filter(|d| d.in_closure_grain()).map(|d| d.name.as_str()).collect()
}

/// Retourne les noms des dimensions analytiques (catégorie `Analytical`).
///
/// Ces dimensions portent un *« dont »* (of which) de la ligne de même grain
/// sans la dimension : une ligne dont une dimension analytique est renseignée
/// est un détail de la ligne où elle est NULL. Elles ne doivent donc **jamais**
/// entrer dans un **total** (bilan, compte de résultat) — ces totaux filtrent
/// `<col> IS NULL` pour ne sommer que les lignes principales. En revanche elles
/// font partie du grain de clôture (chaque « dont » a sa propre clôture).
pub fn analytical_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter()
        .filter(|d| d.category == DimCategory::Analytical)
        .map(|d| d.name.as_str())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
//  CRUD dimensions custom
// ─────────────────────────────────────────────────────────────────────────────

/// Valide un nom de dimension custom :
/// - 1 à 50 caractères
/// - Premier caractère : lettre ou underscore
/// - Reste : alphanumérique + underscore
/// - Pas un nom réservé (`level`, `amount`, `id`).
pub fn is_valid_custom_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 50
        && name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "level" | "amount" | "id")
}

/// Crée une dimension custom :
/// - Valide le nom
/// - Refuse les doublons (built-in ou déjà présente dans `dim_custom_dimension`)
/// - `ALTER TABLE fact_entry ADD COLUMN {name} TEXT`
/// - `ALTER TABLE stg_entry  ADD COLUMN {name} TEXT`
/// - `INSERT INTO dim_custom_dimension (name, label) VALUES (?, ?)`
///
/// L'injection SQL via `name` est neutralisée par la validation (alphanum +
/// underscore uniquement) ; `label` passe par un paramètre lié.
pub fn create_custom(con: &Connection, name: &str, label: &str) -> Result<(), duckdb::Error> {
    if !is_valid_custom_name(name) {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "nom de dimension invalide : {name:?} (alphanum + underscore, 1-50 caractères, \
             premier caractère lettre ou underscore, réservés : level/amount/id)"
        )));
    }
    if builtin_dims().iter().any(|d| d.name == name) {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "nom de dimension déjà utilisé (built-in) : {name}"
        )));
    }
    let exists: bool = con.query_row(
        "SELECT COUNT(*) > 0 FROM dim_custom_dimension WHERE name = ?",
        [name],
        |r| r.get(0),
    )?;
    if exists {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "dimension custom déjà existante : {name}"
        )));
    }
    con.execute(
        &format!("ALTER TABLE fact_entry ADD COLUMN {name} TEXT"),
        [],
    )?;
    con.execute(
        &format!("ALTER TABLE stg_entry ADD COLUMN {name} TEXT"),
        [],
    )?;
    con.execute(
        "INSERT INTO dim_custom_dimension (name, label) VALUES (?, ?)",
        &[&name, &label],
    )?;
    Ok(())
}

/// Supprime une dimension custom :
/// - Vérifie qu'elle existe dans `dim_custom_dimension`
/// - `ALTER TABLE fact_entry DROP COLUMN {name}`
/// - `ALTER TABLE stg_entry  DROP COLUMN {name}`
/// - `DELETE FROM dim_custom_dimension WHERE name = ?`
pub fn delete_custom(con: &Connection, name: &str) -> Result<(), duckdb::Error> {
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM dim_custom_dimension WHERE name = ?",
        [name],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "dimension custom inexistante : {name}"
        )));
    }
    con.execute(&format!("ALTER TABLE fact_entry DROP COLUMN {name}"), [])?;
    con.execute(&format!("ALTER TABLE stg_entry DROP COLUMN {name}"), [])?;
    con.execute("DELETE FROM dim_custom_dimension WHERE name = ?", [name])?;
    Ok(())
}

/// Ré-applique les colonnes custom (après un reset complet) :
/// pour chaque dim custom, `ALTER TABLE ... ADD COLUMN` + `INSERT` dans le
/// registre. Idempotent sur l'`ALTER` (la colonne peut déjà exister si la
/// table a survécu au reset — non utilisé actuellement, mais défensif).
pub fn apply_custom_columns(con: &Connection, customs: &[DimDef]) -> Result<(), duckdb::Error> {
    for d in customs {
        // ALTER TABLE fact_entry ADD (silencieux si la colonne existe déjà).
        let _ = con.execute(
            &format!("ALTER TABLE fact_entry ADD COLUMN {} TEXT", d.name),
            [],
        );
        let _ = con.execute(
            &format!("ALTER TABLE stg_entry ADD COLUMN {} TEXT", d.name),
            [],
        );
        con.execute(
            "INSERT INTO dim_custom_dimension (name, label) VALUES (?, ?)",
            &[&d.name, &d.label],
        )?;
    }
    Ok(())
}
