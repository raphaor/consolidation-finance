//! Étape A — Agrégation (→ niveau `corporate`).
//!
//! Miroir de `conso/pipeline.py::step_a_aggregate`.
//!
//! Cumul des écritures source par entité. Lit la saisie brute (`stg_entry`),
//! agrège par le grain complet des dimensions propagées (built-in + customs)
//! et stocke au niveau *corporate* (en devise fonctionnelle). La nature fait
//! partie du grain d'agrégation : deux écritures de natures différentes ne
//! sont jamais agrégées. La dimension `partner` est également préservée au
//! grain : deux écritures interco sur des partenaires distincts restent
//! séparées (nécessaire pour les règles d'élimination interco).
//!
//! **Staging par nature** : seules les écritures de préfixe `0` ou `1` passent
//! par l'étape A. Les préfixes `2`, `3`, `4` sont injectés directement à leur
//! niveau cible par le module `staging`. Voir `docs/FLUX_CONSO.md` « Staging ».
//!
//! **Isolation par scénario + filtre de scope** (cf. docs/A_NOUVEAU.md §4 bis.2) :
//! l'agrégation ne reprend que les écritures du **scénario du run** et des
//! **entités présentes dans le périmètre** (`sat_perimeter`, toutes méthodes ;
//! entrantes/sortantes incluses via l'INNER JOIN). Les autres scénarios (ex.
//! snapshot d'à-nouveau figé) et les entités hors scope ne polluent pas le
//! corporate du run courant.
//!
//! Aucun filtre sur les flux : la saisie (mode écriture ou formulaire bilan)
//! est agrégée telle quelle. En mode écriture, les liasses ne contiennent que
//! F00/F20 ; en mode bilan, le F99 (clôture) saisi sera agrégé ici puis
//! reconstruit/écrasé plus loin par `materialize_closures` à chaque niveau de
//! stockage. La validité des flux saisis relève du formulaire d'entrée, pas de
//! cette étape.

use super::count_level;
use crate::dimensions;
use duckdb::Connection;

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions
/// (`dimensions::load_all`) : la liste des colonnes propagées définit à la
/// fois le `SELECT`, l'`INSERT` et le `GROUP BY`. Pour les 12 colonnes
/// built-in, le SQL produit est identique au SQL statique historique (test
/// golden inchangé).
///
/// `scenario` = code du scénario du run : seules ses écritures sont agrégées
/// (isolation des autres scénarios, ex. snapshot d'à-nouveau figé).
///
/// Renvoie le nombre de lignes produites au niveau `corporate`.
pub fn step_a(con: &Connection, scenario: &str) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");
    // Colonnes préfixées `s.` pour lever l'ambiguïté avec la jointure
    // `sat_perimeter` (qui porte aussi entity/scenario/period).
    let s_cols = cols
        .iter()
        .map(|c| format!("s.{c}"))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, level, amount)\n\
         SELECT\n\
             {s_cols},\n\
             'corporate' AS level,\n\
             SUM(s.amount) AS amount\n\
         FROM stg_entry s\n\
         JOIN sat_perimeter p\n\
           ON p.entity   = s.entity\n\
          AND p.scenario = s.scenario\n\
          AND p.period   = s.entry_period\n\
         WHERE substr(s.nature, 1, 1) IN ('0', '1')\n\
           AND s.scenario = ?\n\
         GROUP BY {s_cols};"
    );
    con.execute(&sql, [scenario])?;
    count_level(con, "corporate")
}
