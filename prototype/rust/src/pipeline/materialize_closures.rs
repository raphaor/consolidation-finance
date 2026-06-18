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
//! Grain actuel : `(scenario, entity, entry_period, period, account, currency, nature)`.
//!
//! Les dimensions `partner`, `share`, `analysis`, `analysis2` existent dans
//! `fact_entry` mais sont volontairement **hors grain** : une clôture est un
//! solde agrégé, pas une écriture détaillée (l'interco / l'analyse n'a pas de
//! sens sur un solde de clôture).
//!
//! **À chaque ajout de dimension** (ex. `Nature` à venir, puis éventuellement
//! d'autres), se poser la question :
//!
//! > *Deux clôtures qui ne diffèrent que par cette dimension sont-elles des soldes
//! > distincts ?*
//!
//! - **Oui** (ex. `Nature` « liasse » vs « ajustement » : ce sont des clôtures
//!   différentes) → la dimension **entre dans le grain**. Il faut alors l'ajouter
//!   aux **deux** endroits repérés par le marqueur `// GRAIN` dans le SQL ci-dessous
//!   (le `GROUP BY` + `SELECT` de l'INSERT, **et** la clause de match du DELETE).
//!   Sinon l'écrasement serait trop large (une reconstruction sur un code nature
//!   écraserait aussi celle d'un autre code nature).
//! - **Non** → ne pas l'ajouter : la dimension sera agrégée (somme) dans la clôture,
//!   ce qui n'a de sens que si elle est nulle ou constante pour les flux concernés.
//!
//! Mettre à jour le grain documenté ci-dessus à chaque évolution.

use duckdb::Connection;

/// # Reconstruction des clôtures (DELETE ciblé puis INSERT).
///
/// `level` = niveau de stockage où reconstruire (`reclassified`, `consolidated`).
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
    // 1) DELETE ciblé — écrasement au grain seulement.
    con.execute(
        "\
DELETE FROM fact_entry fe
WHERE fe.level = ?
  AND fe.flow IN (SELECT code FROM dim_flow d WHERE d.code = d.flux_de_report)
  AND EXISTS (
      SELECT 1
      FROM fact_entry e
      JOIN dim_flow fl ON fl.code = e.flow
      WHERE e.level = ?
        AND fl.flux_de_report = fe.flow
        AND e.flow <> fl.flux_de_report
        AND e.scenario      = fe.scenario
        AND e.entity        = fe.entity
        AND e.entry_period  = fe.entry_period
        AND e.period        = fe.period
        AND e.account       = fe.account
        AND e.currency      = fe.currency
        AND e.nature        = fe.nature
        -- GRAIN : ajouter ici `AND e.<nouvelle_dim> = fe.<nouvelle_dim>`
        --        pour toute dimension devant entrer dans le grain de clôture.
        --        Voir doc d'en-tête.
  );",
        [level, level],
    )?;

    // 2) INSERT de la reconstruction, agrégée au grain.
    con.execute(
        "\
INSERT INTO fact_entry
    (scenario, entity, entry_period, period, account, flow, currency, nature, level, amount)
SELECT
    e.scenario, e.entity, e.entry_period, e.period, e.account,
    fl.flux_de_report AS flow,
    e.currency,
    e.nature,
    ?                AS level,
    SUM(e.amount)    AS amount
FROM fact_entry e
JOIN dim_flow fl ON fl.code = e.flow
WHERE e.level = ?
  AND fl.flux_de_report IS NOT NULL
  AND e.flow <> fl.flux_de_report
  -- On ne reconstruit que les flux de clôture (auto-référentiels) :
  AND fl.flux_de_report IN (SELECT code FROM dim_flow d WHERE d.code = d.flux_de_report)
GROUP BY
    e.scenario, e.entity, e.entry_period, e.period, e.account, e.currency,
    e.nature, fl.flux_de_report;",
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
