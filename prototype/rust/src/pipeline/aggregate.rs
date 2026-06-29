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
use crate::{dimensions, references};
use duckdb::{params, Connection};

/// Exécute l'étape A : agrège les écritures brutes au niveau corporate.
///
/// Le SQL est généré dynamiquement depuis le registre des dimensions. Pour les
/// dimensions built-in id-typées (10 colonnes), les codes TEXT de `stg_entry`
/// sont résolus en INTEGER ids via JOIN avant insertion dans `fact_entry`.
/// Les dimensions libres (analysis, analysis2, custom) restent TEXT.
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

    // Construit les JOINs de résolution code→id et les expressions de la
    // sous-requête. Pour les dims id-typées : JOIN + `_d{name}.id AS {name}`.
    // Pour les dims libres : `s.{name}` directement.
    //
    // La résolution code→id est isolée dans une sous-requête : l'agrégation
    // (GROUP BY + SUM) opère ensuite sur des noms de colonnes propres et
    // non-ambigus (`phase`, `entity`, …). On évite ainsi à la fois
    // l'ambiguïté `s.entity`/`per.entity` (GROUP BY sur alias) et le GROUP BY
    // positionnel (rejeté par DuckDB dans un INSERT … SELECT).
    let mut id_joins = String::new();
    let mut inner_exprs: Vec<String> = Vec::new();
    for dim in &dims {
        let name = &dim.name;
        if let Some((table, code_col)) = references::dimension_master(name) {
            let alias = format!("_d{name}");
            let join_type = if dim.nullable() { "LEFT JOIN" } else { "JOIN" };
            id_joins.push_str(&format!(
                "\n{join_type} {table} {alias} ON {alias}.{code_col} = s.{name}"
            ));
            inner_exprs.push(format!("{alias}.id AS {name}"));
        } else {
            inner_exprs.push(format!("s.{name}"));
        }
    }
    let inner_list = inner_exprs.join(",\n        ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, consolidation_id, level, amount)\n\
         SELECT\n\
             {col_list},\n\
             ? AS consolidation_id,\n\
             'corporate' AS level,\n\
             SUM(amount) AS amount\n\
         FROM (\n\
           SELECT\n\
             {inner_list},\n\
             s.amount AS amount\n\
           FROM stg_entry s\n\
           JOIN sat_perimeter per\n\
             ON per.perimeter_set = ?\n\
            AND per.entity        = s.entity\n\
            AND per.period        = ?\n\
           {id_joins}\n\
           WHERE substr(s.nature, 1, 1) IN ('0', '1')\n\
             AND s.phase = ?\n\
             AND s.entry_period = ?\n\
         ) t\n\
         GROUP BY {col_list};"
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
