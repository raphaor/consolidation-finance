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
use duckdb::{params, Connection};

/// Exécute l'étape D : applique la méthode d'intégration de chaque entité.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Pour
/// les 12 colonnes built-in, le SQL produit reste structurellement identique
/// au SQL statique historique (test golden inchangé).
///
/// `p` = paramètres du run (`ConvertParams`) : les lignes `converted` du run
/// sont isolées par `p.consolidation_id` (le périmètre est résolu directement
/// via `(p.perimeter_set, p.perimeter_period)`, sans jointure sur
/// `dim_consolidation`). Le staging préfixe 3 (devise fonctionnelle, AVANT le
/// × pct) est consommé ici par UNION, tagué lui aussi `p.consolidation_id`.
///
/// Renvoie le nombre de lignes produites au niveau `consolidated`.
pub fn step_d(con: &Connection, p: &super::ConvertParams) -> duckdb::Result<usize> {
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
    ({col_list}, consolidation_id, level, amount)\n\
SELECT\n\
    {f_cols},\n\
    f.consolidation_id,\n\
    'consolidated' AS level,\n\
    f.amount * COALESCE(per.pct_integration, 1.0) AS amount\n\
FROM (\n\
    SELECT {col_list}, consolidation_id, amount FROM fact_entry\n\
    WHERE level = 'converted' AND consolidation_id = ?\n\
    UNION ALL\n\
    -- Staging préfixe 3 : écritures injectées au consolidé AVANT la mécanique de\n\
    -- taux → elles subissent le × pct_integration (via le JOIN sat_perimeter),\n\
    -- comme les flux convertis. Cf. docs/A_NOUVEAU.md §4 bis.\n\
    SELECT {col_list}, ? AS consolidation_id, amount FROM stg_entry\n\
    WHERE substr(nature, 1, 1) = '3' AND phase = ? AND entry_period = ?\n\
) f\n\
JOIN sat_perimeter per\n\
  ON per.perimeter_set = ?\n\
 AND per.entity = f.entity\n\
 AND per.period = ?\n\
JOIN dim_method m\n\
  ON m.id = per.methode\n\
WHERE m.consolidated = true;  -- équivalence et méthodes futures exclues par flag"
    );
    con.execute(
        &sql,
        params![
            p.consolidation_id,
            p.consolidation_id,
            p.phase,
            p.exercice,
            p.perimeter_set,
            p.perimeter_period,
        ],
    )?;
    count_level(con, "consolidated")
}
