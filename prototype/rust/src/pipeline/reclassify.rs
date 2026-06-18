//! Étape B — Reclassification de périmètre (→ niveau `reclassified`).
//!
//! Miroir de `conso/pipeline.py::step_b_reclassify`.
//!
//! Travail en devise fonctionnelle (pas de conversion ici) :
//!   - Entité entrante  : F00 → F01 (l'ouverture de l'entrant est isolée en F01)
//!   - Entité sortante  : tous les flux passent à l'identique (clôtures incluses)
//!                        + chaque CONSTITUANT X génère un miroir −X sur F98,
//!                        donc F98 = −Σ(constituants) → F99 = 0 par identité de
//!                        reconstruction (le solde de la sortante ne fuit pas
//!                        dans F99 consolidé).
//!   - Entité continue  : copie à l'identique
//!
//! Le miroir F98 (branche 3b) cible les **constituants** (flux non-clôture de
//! `dim_flow`, i.e. `code <> flux_de_report`) : une clôture étant la somme des
//! constituants, la refléter sur F98 la compterait deux fois. Le passthrough
//! (3a), lui, ne filtre rien — les clôtures transitent puis sont écrasées par
//! la reconstruction au niveau reclassified. Cf. docs/FLUX_CONSO.md §9.

use super::count_level;
use crate::dimensions;
use duckdb::Connection;

/// Construit une liste de sélections SQL pour des colonnes données.
///
/// Pour chaque colonne, par défaut `{prefix}.{name}`. Les `overrides`
/// permettent de remplacer l'expression d'une colonne (ex. CASE WHEN … AS flow).
fn build_select_cols(
    cols: &[&str],
    prefix: &str,
    overrides: &[(&str, &str)],
) -> String {
    cols.iter()
        .map(|c| match overrides.iter().find(|(k, _)| k == c) {
            Some((_, expr)) => format!("{expr} AS {c}"),
            None => format!("{prefix}.{c}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Exécute l'étape B : reclassifie les flux selon les variations de périmètre.
///
/// Le SQL (4 branches UNION ALL) est généré dynamiquement depuis le registre
/// des dimensions. Pour les 12 colonnes built-in, le SQL produit reste
/// structurellement identique au SQL statique historique (test golden
/// inchangé).
///
/// Renvoie le nombre de lignes produites au niveau `reclassified`.
pub fn step_b(con: &Connection) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");

    // Pour chaque branche, on génère la liste des colonnes sélectionnées.
    // Branche 2 (entrantes) : F00 → F01, donc override sur `flow`.
    let flow_override: &[(&str, &str)] = &[(
        "flow",
        "CASE WHEN f.flow = 'F00' THEN 'F01' ELSE f.flow END",
    )];
    // Branche 3b (sortantes — miroir F98) : flow forcé à 'F98'. Le montant est
    // nié à part (on garde la même structure que les autres branches : 12 cols
    // propagées + `f.amount` en 13e position, mais ici nié).
    let f98_override: &[(&str, &str)] = &[("flow", "'F98'")];

    let sel_passthrough = build_select_cols(&cols, "f", &[]);
    let sel_entrante = build_select_cols(&cols, "f", flow_override);
    let sel_miroir = build_select_cols(&cols, "f", f98_override);

    let sql = format!(
        "\
INSERT INTO fact_entry\n\
    ({col_list}, level, amount)\n\
SELECT\n\
    {col_list},\n\
    'reclassified' AS level,\n\
    SUM(amount)    AS amount\n\
FROM (\n\
    -- 1) Entités continues : copie à l'identique\n\
    SELECT {sel_passthrough}, f.amount\n\
    FROM fact_entry f\n\
    JOIN sat_perimeter p\n\
      ON p.entity = f.entity\n\
     AND p.scenario = f.scenario\n\
     AND p.period = f.entry_period\n\
    WHERE f.level = 'corporate'\n\
      AND NOT COALESCE(p.entree, FALSE)\n\
      AND NOT COALESCE(p.sortie, FALSE)\n\
\n\
    UNION ALL\n\
\n\
    -- 2) Entités entrantes : F00 → F01, autres flux inchangés\n\
    SELECT {sel_entrante}, f.amount\n\
    FROM fact_entry f\n\
    JOIN sat_perimeter p\n\
      ON p.entity = f.entity\n\
     AND p.scenario = f.scenario\n\
     AND p.period = f.entry_period\n\
    WHERE f.level = 'corporate'\n\
      AND COALESCE(p.entree, FALSE)\n\
      AND NOT COALESCE(p.sortie, FALSE)\n\
\n\
    UNION ALL\n\
\n\
    -- 3a) Entités sortantes — passthrough de TOUS les flux à l'identique\n\
    --     (clôtures incluses : une clôture saisie transite, puis sera écrasée\n\
    --      par la reconstruction au niveau reclassified).\n\
    SELECT {sel_passthrough}, f.amount\n\
    FROM fact_entry f\n\
    JOIN sat_perimeter p\n\
      ON p.entity = f.entity\n\
     AND p.scenario = f.scenario\n\
     AND p.period = f.entry_period\n\
    WHERE f.level = 'corporate'\n\
      AND COALESCE(p.sortie, FALSE)\n\
\n\
    UNION ALL\n\
\n\
    -- 3b) Entités sortantes — miroir négatif sur F98 : chaque CONSTITUANT X\n\
    --      (flux non-clôture) génère −X. On cible les constituants seulement :\n\
    --      une clôture est la somme des constituants, donc la refléter sur F98\n\
    --      la compterait deux fois. Agrégé par compte, F98 = −Σ(constituants) ;\n\
    --      comme F98 reporte à F99 (flux_de_report = 'F99'), l'identité\n\
    --      F99 = F00 + F20 + … + F98 se referme à 0.\n\
    SELECT {sel_miroir}, -f.amount\n\
    FROM fact_entry f\n\
    JOIN sat_perimeter p\n\
      ON p.entity = f.entity\n\
     AND p.scenario = f.scenario\n\
     AND p.period = f.entry_period\n\
    WHERE f.level = 'corporate'\n\
      AND COALESCE(p.sortie, FALSE)\n\
      AND f.flow IN (SELECT code FROM dim_flow WHERE code <> flux_de_report)\n\
) rec\n\
GROUP BY {col_list};"
    );
    con.execute(&sql, [])?;
    count_level(con, "reclassified")
}
