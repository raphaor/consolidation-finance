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
//! **Isolation par consolidation + filtre de scope** (cf. docs/A_NOUVEAU.md
//! §4 bis.2) : l'agrégation ne reprend que les écritures de la **remontée du
//! run** (`stg_entry.phase = p.phase AND stg_entry.entry_period = p.exercice`)
//! et des **entités présentes dans le périmètre** (`sat_perimeter`, toutes
//! méthodes ; entrantes/sortantes incluses via l'INNER JOIN). Les autres
//! remontées et les entités hors scope ne polluent pas le corporate du run
//! courant. Les lignes agrégées sont taguées avec `p.consolidation_id` (isolation
//! des résultats du run dans `fact_entry`).
//!
//! Aucun filtre sur les flux : la saisie (mode écriture ou formulaire bilan)
//! est agrégée telle quelle. En mode écriture, les liasses ne contiennent que
//! F00/F20 ; en mode bilan, le F99 (clôture) saisi sera agrégé ici puis
//! reconstruit/écrasé plus loin par `materialize_closures` à chaque niveau de
//! stockage. La validité des flux saisis relève du formulaire d'entrée, pas de
//! cette étape.

use super::count_level;
use crate::dimensions;
use duckdb::{params, Connection};

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions
/// (`dimensions::load_all`) : la liste des colonnes propagées définit à la
/// fois le `SELECT`, l'`INSERT` et le `GROUP BY`. Pour les 12 colonnes
/// built-in, le SQL produit est identique au SQL statique historique (test
/// golden inchangé).
///
/// `p` = paramètres du run (`ConvertParams`) : la remontée est sélectionnée par
/// `(p.phase, p.exercice)`, le périmètre par `(p.perimeter_set, p.perimeter_period)`,
/// et les lignes agrégées sont isolées dans `fact_entry` via `p.consolidation_id`.
///
/// Renvoie le nombre de lignes produites au niveau `corporate`.
pub fn step_a(con: &Connection, p: &super::ConvertParams) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");
    // Colonnes préfixées `s.` pour lever l'ambiguïté avec la jointure
    // `sat_perimeter per` (qui porte aussi entity/period).
    let s_cols = cols
        .iter()
        .map(|c| format!("s.{c}"))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, consolidation_id, level, amount)\n\
         SELECT\n\
             {s_cols},\n\
             ? AS consolidation_id,\n\
             'corporate' AS level,\n\
             SUM(s.amount) AS amount\n\
         FROM stg_entry s\n\
         JOIN sat_perimeter per\n\
           ON per.perimeter_set = ?\n\
          AND per.entity        = s.entity\n\
          AND per.period        = ?\n\
         WHERE substr(s.nature, 1, 1) IN ('0', '1')\n\
           AND s.phase = ?\n\
           AND s.entry_period = ?\n\
         GROUP BY {s_cols};"
    );
    con.execute(
        &sql,
        params![
            p.consolidation_id,
            p.perimeter_set,
            p.perimeter_period,
            p.phase,
            p.exercice,
        ],
    )?;
    count_level(con, "corporate")
}
