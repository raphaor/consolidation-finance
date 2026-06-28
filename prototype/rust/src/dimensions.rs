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

use crate::references;

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
    /// Nom technique de la colonne (`scenario`, `partner`, …) — identifiant API.
    pub name: String,
    /// Nom physique dans `fact_entry` / `stg_entry`.
    /// Pour les built-ins : identique à `name`.
    /// Pour les custom : `x{id}` (B1 étape 10).
    pub col: String,
    /// Catégorie (Fixed / Active / Analytical).
    pub category: DimCategory,
    /// `true` si ajoutée par l'utilisateur (custom).
    pub custom: bool,
    /// Libellé UI.
    pub label: String,
    /// Dimension cible pour les dimensions empruntées (§11).
    /// `None` = dimension libre (TEXT, pas de validation).
    /// `Some("entity")` = emprunte les valeurs de `dim_entity`.
    pub target_dimension: Option<String>,
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
    macro_rules! builtin {
        ($name:literal, $cat:expr, $label:literal) => {
            DimDef {
                name: $name.into(),
                col: $name.into(),
                category: $cat,
                custom: false,
                label: $label.into(),
                target_dimension: None,
            }
        };
    }
    vec![
        builtin!("phase",        DimCategory::Fixed,      "Phase"),
        builtin!("entity",       DimCategory::Active,     "Entité"),
        builtin!("entry_period", DimCategory::Fixed,      "Exercice"),
        builtin!("period",       DimCategory::Fixed,      "Période"),
        builtin!("account",      DimCategory::Active,     "Compte"),
        builtin!("flow",         DimCategory::Active,     "Flux"),
        builtin!("currency",     DimCategory::Fixed,      "Devise"),
        builtin!("nature",       DimCategory::Active,     "Nature"),
        builtin!("partner",      DimCategory::Analytical, "Partenaire"),
        builtin!("share",        DimCategory::Analytical, "Titre"),
        builtin!("analysis",     DimCategory::Analytical, "Analyse 1"),
        builtin!("analysis2",    DimCategory::Analytical, "Analyse 2"),
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
    let mut stmt =
        con.prepare("SELECT name, label, id, target_dimension FROM dim_custom_dimension ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let id: Option<i64> = row.get(2)?;
        let col = id.map(|i| format!("x{i}")).unwrap_or_else(|| name.clone());
        Ok(DimDef {
            name,
            col,
            category: DimCategory::Analytical,
            custom: true,
            label: row.get(1)?,
            target_dimension: row.get(3)?,
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

/// Retourne les colonnes physiques propagées (toutes les dims du registre).
/// Pour les built-ins : identique au nom. Pour les custom : `x{id}`.
pub fn propagated_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().map(|d| d.col.as_str()).collect()
}

/// Retourne les colonnes physiques pilotables (Active + Analytical).
pub fn pilotable_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter()
        .filter(|d| d.pilotable())
        .map(|d| d.col.as_str())
        .collect()
}

/// Retourne les colonnes physiques du grain de reconstruction des clôtures
/// (Fixed + Active).
pub fn closure_grain_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter()
        .filter(|d| d.in_closure_grain())
        .map(|d| d.col.as_str())
        .collect()
}

/// Retourne le nom physique (`col`) d'une dimension dans la slice, ou le nom
/// API si non trouvé (built-in où col == name).
pub fn col_of<'a>(dims: &'a [DimDef], name: &'a str) -> &'a str {
    dims.iter()
        .find(|d| d.name == name)
        .map(|d| d.col.as_str())
        .unwrap_or(name)
}

/// Retourne les colonnes physiques des dimensions analytiques (catégorie
/// `Analytical`). Utilisé pour les filtres `IS NULL` dans les totaux.
pub fn analytical_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter()
        .filter(|d| d.category == DimCategory::Analytical)
        .map(|d| d.col.as_str())
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

/// Migration §11 : ajoute `target_dimension` à `dim_custom_dimension` si absent.
/// Idempotent — ne fait rien si la colonne existe déjà.
pub fn migrate_custom_dimension_target(con: &Connection) -> Result<(), duckdb::Error> {
    let has_col: bool = con
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.columns \
             WHERE table_schema = 'main' AND table_name = 'dim_custom_dimension' \
             AND column_name = 'target_dimension'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_col {
        con.execute(
            "ALTER TABLE dim_custom_dimension ADD COLUMN target_dimension TEXT",
            [],
        )?;
    }
    Ok(())
}

/// Crée une dimension custom (B1 étape 10) :
/// - Valide le nom
/// - Refuse les doublons (built-in ou déjà présente dans `dim_custom_dimension`)
/// - `INSERT INTO dim_custom_dimension` (obtient l'id auto)
/// - `ALTER TABLE fact_entry ADD COLUMN x{id} TEXT`
/// - `ALTER TABLE stg_entry  ADD COLUMN x{id} TEXT`
///
/// La colonne physique `x{id}` ne dépend pas du code `name` : renommer la
/// dimension ne nécessite pas d'`ALTER TABLE`.
///
/// `target_dimension` (optionnel) : si fourni, la dimension emprunte ses
/// valeurs d'une autre dimension (comme `partner` → `entity`). La colonne
/// `x{id}` sera alors validée contre la master data de la dimension cible.
pub fn create_custom(
    con: &Connection,
    name: &str,
    label: &str,
    target_dimension: Option<&str>,
) -> Result<(), duckdb::Error> {
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
    // Valider que la dimension cible a une master data (si fournie).
    if let Some(target) = target_dimension {
        if references::dimension_master(target).is_none() {
            return Err(duckdb::Error::InvalidParameterName(format!(
                "dimension cible sans master data : {target}"
            )));
        }
    }
    // INSERT d'abord : la séquence alloue l'id.
    con.execute(
        "INSERT INTO dim_custom_dimension (name, label, target_dimension) VALUES (?, ?, ?)",
        duckdb::params![name, label, target_dimension],
    )?;
    // Lire l'id obtenu pour nommer la colonne physique.
    let id: i64 = con.query_row(
        "SELECT id FROM dim_custom_dimension WHERE name = ?",
        [name],
        |r| r.get(0),
    )?;
    let col = format!("x{id}");
    con.execute(&format!("ALTER TABLE fact_entry ADD COLUMN {col} TEXT"), [])?;
    con.execute(&format!("ALTER TABLE stg_entry ADD COLUMN {col} TEXT"), [])?;
    Ok(())
}

/// Supprime une dimension custom :
/// - Vérifie qu'elle existe dans `dim_custom_dimension`
/// - `ALTER TABLE fact_entry DROP COLUMN x{id}`
/// - `ALTER TABLE stg_entry  DROP COLUMN x{id}`
/// - `DELETE FROM dim_custom_dimension WHERE name = ?`
pub fn delete_custom(con: &Connection, name: &str) -> Result<(), duckdb::Error> {
    let row: Option<(i64, i64)> = con
        .query_row(
            "SELECT COUNT(*), id FROM dim_custom_dimension WHERE name = ?",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let id = match row {
        Some((1, id)) => id,
        _ => {
            return Err(duckdb::Error::InvalidParameterName(format!(
                "dimension custom inexistante : {name}"
            )))
        }
    };
    let col = format!("x{id}");
    con.execute(&format!("ALTER TABLE fact_entry DROP COLUMN {col}"), [])?;
    con.execute(&format!("ALTER TABLE stg_entry DROP COLUMN {col}"), [])?;
    con.execute("DELETE FROM dim_custom_dimension WHERE name = ?", [name])?;
    Ok(())
}

/// Renomme le code d'une dimension custom.
///
/// Sous B1, la colonne physique (`x{id}`) est immunisée au renommage : seul le
/// champ `name` de `dim_custom_dimension` change. Bloque si la dimension est
/// référencée dans une règle / un poste / un indicateur (JSON stocke le nom API).
pub fn rename_custom(
    con: &Connection,
    old_name: &str,
    new_name: &str,
) -> Result<(), duckdb::Error> {
    if !is_valid_custom_name(new_name) {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "nom de dimension invalide : {new_name:?}"
        )));
    }
    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM dim_custom_dimension WHERE name = ?",
        [old_name],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "dimension custom inexistante : {old_name}"
        )));
    }
    if builtin_dims().iter().any(|d| d.name == new_name) {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "nom réservé (built-in) : {new_name}"
        )));
    }
    let taken: bool = con.query_row(
        "SELECT COUNT(*) > 0 FROM dim_custom_dimension WHERE name = ?",
        [new_name],
        |r| r.get(0),
    )?;
    if taken {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "dimension custom déjà existante : {new_name}"
        )));
    }
    // Garde rôle 3 : la dimension ne doit pas apparaître comme clé de dimension
    // dans les JSON de règles / postes (dim dans selection ou clé de destination).
    let blockers = scan_custom_dim_blockers(con, old_name)?;
    if !blockers.is_empty() {
        return Err(duckdb::Error::InvalidParameterName(format!(
            "renommage bloqué — dimension '{old_name}' citée dans : {}",
            blockers.join(", ")
        )));
    }
    con.execute(
        "UPDATE dim_custom_dimension SET name = ? WHERE name = ?",
        &[&new_name, &old_name],
    )?;
    Ok(())
}

/// Scanne les JSON de règles et postes pour détecter si `dim_name` y est
/// référencé comme dimension (clé de sélection ou destination). Retourne la
/// liste des codes bloquants.
fn scan_custom_dim_blockers(
    con: &Connection,
    dim_name: &str,
) -> duckdb::Result<Vec<String>> {
    // Heuristique rapide : recherche textuelle du nom entre guillemets dans le JSON.
    // Couvre selection[*].dim et les clés de destination sans faux-positifs sur
    // des valeurs littérales (qui sont des codes de master data, pas des noms de dim).
    let quoted = format!("\"{}\"", dim_name);
    let mut blockers = Vec::new();

    // dim_rule.definition
    let mut stmt = con.prepare(
        "SELECT code FROM dim_rule WHERE definition LIKE ?",
    )?;
    let like_pat = format!("%{}%", quoted);
    let codes: Vec<String> = stmt
        .query_map([&like_pat], |r| r.get(0))?
        .flatten()
        .collect();
    for c in codes {
        blockers.push(format!("rule:{c}"));
    }

    // dim_aggregate.definition
    let mut stmt2 = con.prepare(
        "SELECT code FROM dim_aggregate WHERE definition LIKE ?",
    )?;
    let codes2: Vec<String> = stmt2
        .query_map([&like_pat], |r| r.get(0))?
        .flatten()
        .collect();
    for c in codes2 {
        blockers.push(format!("aggregate:{c}"));
    }

    Ok(blockers)
}

/// Ré-applique les colonnes custom (après un reset complet) :
/// pour chaque dim custom, `ALTER TABLE ... ADD COLUMN x{id} TEXT`.
///
/// `dim_custom_dimension` survit au reset (ses lignes persistent avec leurs ids).
/// L'`INSERT OR IGNORE` est une garde défensive si la table avait été vidée.
/// Idempotent sur les `ALTER` (silencieux si la colonne existe déjà).
pub fn apply_custom_columns(con: &Connection, customs: &[DimDef]) -> Result<(), duckdb::Error> {
    for d in customs {
        // Colonne physique x{id} (B1 étape 10).
        let _ = con.execute(
            &format!("ALTER TABLE fact_entry ADD COLUMN {} TEXT", d.col),
            [],
        );
        let _ = con.execute(
            &format!("ALTER TABLE stg_entry ADD COLUMN {} TEXT", d.col),
            [],
        );
        // La ligne dans dim_custom_dimension survit normalement au reset ;
        // INSERT OR IGNORE est défensif.
        let _ = con.execute(
            "INSERT OR IGNORE INTO dim_custom_dimension (name, label) VALUES (?, ?)",
            &[&d.name, &d.label],
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let con = Connection::open_in_memory().expect("open_in_memory");
        crate::schema::create_schema(&con).expect("create_schema");
        con
    }

    #[test]
    fn create_custom_libre_ok() {
        let con = setup();
        create_custom(&con, "region", "Région", None).unwrap();
        let customs = load_customs(&con).unwrap();
        assert_eq!(customs.len(), 1);
        assert_eq!(customs[0].name, "region");
        assert_eq!(customs[0].target_dimension, None);
    }

    #[test]
    fn create_custom_empruntee_ok() {
        let con = setup();
        create_custom(&con, "devise_tx", "Devise transaction", Some("currency")).unwrap();
        let customs = load_customs(&con).unwrap();
        assert_eq!(customs.len(), 1);
        assert_eq!(customs[0].target_dimension.as_deref(), Some("currency"));
    }

    #[test]
    fn create_custom_cible_invalide_erreur() {
        let con = setup();
        let err = create_custom(&con, "x", "X", Some("inexistant"));
        assert!(err.is_err(), "dimension cible inexistante doit échouer");
    }

    #[test]
    fn dynamic_references_empruntee() {
        let con = setup();
        create_custom(&con, "devise_tx", "Devise transaction", Some("currency")).unwrap();
        let customs = load_customs(&con).unwrap();
        let col = &customs[0].col; // "x{id}"
        let refs = references::dynamic_references(&con);
        assert!(
            refs.iter().any(|r| r.table == "stg_entry"
                && r.column == *col
                && r.target_table == "dim_currency"),
            "la dimension empruntée doit avoir une référence dans le graphe"
        );
    }
}
