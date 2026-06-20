//! Étape transversale — Reconstruction des flux de clôture par `flux_de_report`.
//!
//! Un flux est une **clôture reconstruite** ssi `dim_flow.flux_de_report(code) = code`
//! (auto-référence). Aujourd'hui seul F99 vérifie cette propriété ; un autre flux
//! peut être déclaré clôture en l'auto-référençant dans `dim_flow`.
//!
//! Pour chaque clôture C, on reconstruit à l'identité :
//!
//! ```text
//! C = Σ( flux X | flux_de_report(X) = C et X ≠ C )
//! ```
//!
//! Cf. `docs/FLUX_CONSO.md §3` (identité de reconstruction par les flux).
//!
//! # Sémantique d'écrasement (valeur autoritaire)
//!
//! La valeur reconstruite est **autoritaire** : pour un grain dimensionnel donné,
//! elle remplace toute valeur de clôture pré-existante. Concrètement, on supprime
//! d'abord les lignes de clôture **dont le grain possède une reconstruction**, puis
//! on insère la reconstruction. Les clôtures sur un grain sans composante (ex. saisie
//! résiduelle sur un autre code `Nature`) ne sont **pas** touchées.
//!
//! Cela rend l'étape idempotente (ré-exécution sans double-compte) et garantit que
//! la reconstruction l'emporte sur toute saisie résiduelle sur un flux de clôture.
//!
//! # ⚠ Grain de reconstruction — LIRE AVANT D'AJOUTER UNE DIMENSION
//!
//! Le « grain » est l'ensemble des dimensions qui identifient un solde de clôture
//! **unique**. La reconstruction agrège au grain ; l'écrasement est ciblé au grain.
//!
//! Grain actuel : **grain complet** (toutes les dimensions propagées) privé de
//! `flow` (ex. `scenario, entity, entry_period, period, account, currency,
//! nature, partner, share, analysis, analysis2` + customs).
//!
//! La dimension `flow` est **hors grain** : la clôture est identifiée par
//! `fl.flux_de_report` (qui vaut F99 pour tous les constituants reportant à F99),
//! donc le `GROUP BY` porte sur `fl.flux_de_report` plutôt que sur `e.flow`.
//!
//! Les dimensions analytiques (`partner`, `share`, `analysis`, `analysis2` + customs)
//! **font partie du grain** : une ligne dont une dimension analytique est
//! renseignée est un *« dont »* (of which) de la ligne où elle est NULL ; elle
//! subit les **mêmes automatismes** (clôtures, conversion) à son propre grain.
//! Ainsi la F99 d'un `partner = B` est reconstruite depuis ses seuls constituants
//! `partner = B`, et la clôture **principale** (analytiques NULL) ne somme que
//! les constituants principaux — pas de double compte. Les **totaux** (bilan,
//! compte de résultat) excluent les « dont » en filtrant `<analytique> IS NULL`
//! (cf. `dimensions::analytical_cols`, `report.rs`, `server.rs`).
//!
//! Le grain étant complet, toute nouvelle dimension (custom comprise) y entre
//! automatiquement : deux clôtures qui diffèrent par n'importe quelle dimension
//! propagée sont des soldes distincts.

use crate::dimensions;
use duckdb::Connection;

/// # Reconstruction des clôtures (DELETE ciblé puis INSERT).
///
/// `level` = niveau de stockage où reconstruire (`corporate`, `converted`, `consolidated`).
/// Renvoie le nombre de lignes de clôture présentes à ce niveau après l'opération.
///
/// Deux requêtes paramétrées par `level` :
///
/// 1. **DELETE ciblé** : ne supprime que les clôtures (flux auto-référentiels)
///    dont le grain va être reconstruit ( EXISTS de composantes au même grain ).
///    Préserve les clôtures sur un grain sans composante (saisie résiduelle,
///    autre code `Nature`, etc.).
/// 2. **INSERT** : agrège les composantes au grain et écrit une ligne de clôture
///    par (clôture × grain).
pub fn materialize_closures(con: &Connection, level: &str) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    // Grain de reconstruction = grain COMPLET (toutes les dimensions propagées),
    // sans `flow` (la clôture est identifiée par `fl.flux_de_report`, pas par le
    // flux source). Les dimensions analytiques (`partner`, `share`, `analysis`…)
    // font partie du grain : chaque « dont » a sa propre clôture (ex. la F99 du
    // partner B = Σ de ses constituants partner B), et la clôture principale
    // (analytiques NULL) ne somme que les constituants principaux. Les totaux
    // (bilan, compte de résultat) excluent ensuite les « dont » via `IS NULL`.
    let grain_cols: Vec<&str> = dimensions::propagated_cols(&dims)
        .into_iter()
        .filter(|c| *c != "flow")
        .collect();
    let grain_list = grain_cols.join(", ");
    let e_grain_list = grain_cols
        .iter()
        .map(|c| format!("e.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    // `IS NOT DISTINCT FROM` (et non `=`) : les dimensions analytiques sont
    // nullables et `NULL = NULL` vaut NULL en SQL. Cet opérateur traite deux
    // NULL comme égaux, sinon l'écrasement ciblé raterait les clôtures
    // principales (analytiques NULL).
    let fe_grain_match = grain_cols
        .iter()
        .map(|c| format!("e.{c} IS NOT DISTINCT FROM fe.{c}"))
        .collect::<Vec<_>>()
        .join("\n        AND ");

    // 1) DELETE ciblé — écrasement au grain seulement.
    con.execute(
        &format!(
            "\
DELETE FROM fact_entry fe\n\
WHERE fe.level = ?\n\
  AND fe.flow IN (SELECT code FROM dim_flow d WHERE d.code = d.flux_de_report)\n\
  AND EXISTS (\n\
      SELECT 1\n\
      FROM fact_entry e\n\
      JOIN dim_flow fl ON fl.code = e.flow\n\
      WHERE e.level = ?\n\
        AND fl.flux_de_report = fe.flow\n\
        AND e.flow <> fl.flux_de_report\n\
        AND {fe_grain_match}\n\
  );"
        ),
        [level, level],
    )?;

    // 2) INSERT de la reconstruction, agrégée au grain.
    con.execute(
        &format!(
            "\
INSERT INTO fact_entry\n\
    ({grain_list}, flow, level, amount)\n\
SELECT\n\
    {e_grain_list},\n\
    fl.flux_de_report AS flow,\n\
    ?                AS level,\n\
    SUM(e.amount)    AS amount\n\
FROM fact_entry e\n\
JOIN dim_flow fl ON fl.code = e.flow\n\
WHERE e.level = ?\n\
  AND fl.flux_de_report IS NOT NULL\n\
  AND e.flow <> fl.flux_de_report\n\
  -- On ne reconstruit que les flux de clôture (auto-référentiels) :\n\
  AND fl.flux_de_report IN (SELECT code FROM dim_flow d WHERE d.code = d.flux_de_report)\n\
GROUP BY\n\
    {e_grain_list}, fl.flux_de_report;"
        ),
        [level, level],
    )?;

    let n: i64 = con.query_row(
        "SELECT COUNT(*) FROM fact_entry fe \
         WHERE fe.level = ? \
           AND fe.flow IN (SELECT code FROM dim_flow d WHERE d.code = d.flux_de_report)",
        [level],
        |row| row.get(0),
    )?;
    Ok(n as usize)
}
