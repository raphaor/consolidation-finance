//! Étape D — Consolidation (→ niveau `consolidated`).
//!
//! Miroir de `conso/python/pipeline.py::step_d_consolidate`.
//!
//! Application des méthodes de consolidation (natif MVP) :
//!   - globale         : copie à 100 % (`pct_integration = 1.0`)
//!   - proportionnelle : `amount × pct_integration`
//!   - équivalence     : EXCLUE du MVP (`dim_method.consolidated = false`)
//!
//! Le filtre des méthodes intégrées est piloté par `dim_method.consolidated`
//! (table maître éditable) : ajouter une méthode consolidée = insérer une
//! ligne dans `dim_method`, sans toucher au SQL de cette étape.
//!
//! **NB** : tous les flux sont consolidés, **clôtures (F99) comprises** : le
//! `pct_integration` est appliqué à la clôture (indispensable pour la méthode
//! proportionnelle). La clôture consolidée (portée) est ensuite écrasée par
//! [`super::materialize_closures`] au niveau consolidated, qui la reconstruit
//! depuis les constituants consolidés (même valeur, mais autoritaire).

use super::count_level;
use crate::dimensions;
use duckdb::Connection;

/// Exécute l'étape D : applique la méthode d'intégration de chaque entité.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Pour
/// les 12 colonnes built-in, le SQL produit reste structurellement identique
/// au SQL statique historique (test golden inchangé).
///
/// `scenario` = code du scénario du run : seules ses lignes `converted` sont
/// consolidées (isolation des autres scénarios, ex. snapshot d'à-nouveau figé).
///
/// Renvoie le nombre de lignes produites au niveau `consolidated`.
pub fn step_d(con: &Connection, scenario: &str) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let f_cols = cols
        .iter()
        .map(|c| format!("f.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let col_list = cols.join(", ");

    let sql = format!(
        "\
INSERT INTO fact_entry\n\
    ({col_list}, level, amount)\n\
SELECT\n\
    {f_cols},\n\
    'consolidated' AS level,\n\
    f.amount * COALESCE(p.pct_integration, 1.0) AS amount\n\
FROM (\n\
    SELECT {col_list}, amount FROM fact_entry\n\
    WHERE level = 'converted' AND scenario = ?\n\
    UNION ALL\n\
    -- Staging préfixe 3 : écritures injectées au consolidé AVANT la mécanique de\n\
    -- taux → elles subissent le × pct_integration (via le JOIN sat_perimeter),\n\
    -- comme les flux convertis. Cf. docs/A_NOUVEAU.md §4 bis.\n\
    SELECT {col_list}, amount FROM stg_entry\n\
    WHERE substr(nature, 1, 1) = '3' AND scenario = ?\n\
) f\n\
JOIN dim_scenario sc ON sc.code = f.scenario\n\
JOIN sat_perimeter p\n\
  ON p.perimeter_set = sc.perimeter_set\n\
 AND p.entity = f.entity\n\
 AND p.period = f.entry_period\n\
JOIN dim_method m\n\
  ON m.code = p.methode\n\
WHERE m.consolidated = true;  -- équivalence et méthodes futures exclues par flag"
    );
    con.execute(&sql, [scenario, scenario])?;
    count_level(con, "consolidated")
}
