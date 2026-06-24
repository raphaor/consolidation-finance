# SPEC TECHNIQUE : Scénario v2 et taux pivot

> ⚠️ **SUPERSEDÉ par [Q41]** (redesign identité, 2026-06-23) : `dim_scenario`→`dim_consolidation`.
> Les DDL/SQL ci-dessous reflètent l'état pré-Q41. Voir `MODELE_DONNEES.md` §3 pour le schéma courant.

*Annexe d'implémentation de [`SPEC_SCENARIO_V2.md`](./SPEC_SCENARIO_V2.md).*

Cette note décrit l'implémentation concrète : DDL exacts, SQL de cross-rate,
signatures Rust et adaptations des modules affectés. Aucune logique métier
additionnelle — uniquement la plomberie pour porter le scénario en objet
composite et introduire le pivot applicatif.

---

## 1. DDL — nouvelles tables et modifications

### 1.1 Nouvelles tables

```sql
-- Config applicative (singleton d'instance). Pas de CRUD master data.
CREATE TABLE app_config (
    key   TEXT PRIMARY KEY,
    value TEXT
);

-- Catalogue des jeux de taux (réels, budget…).
CREATE TABLE dim_rate_set (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);

-- Catalogue des variantes (BASE, OPT1, PESSIMIST…).
CREATE TABLE dim_variant (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);

-- Catalogue des catégories de scénario (REEL, BUDGET, PREVISION).
CREATE TABLE dim_scenario_category (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);
```

### 1.2 `dim_scenario` v2 (refonte)

Ancien :

```sql
CREATE TABLE dim_scenario (
    code     TEXT PRIMARY KEY,
    libelle  TEXT,
    type     TEXT,
    statut   TEXT
);
```

Nouveau :

```sql
CREATE TABLE dim_scenario (
    code                  TEXT PRIMARY KEY,
    libelle               TEXT,
    category              TEXT,   -- FK dim_scenario_category
    entry_period          TEXT,   -- FK dim_period
    presentation_currency TEXT,   -- FK dim_currency
    variant               TEXT,   -- FK dim_variant
    ruleset_code          TEXT,   -- FK dim_ruleset (NULLABLE)
    rate_set              TEXT,   -- FK dim_rate_set
    statut                TEXT    -- 'ouvert' / 'verrouillé'
);
```

Colonne `type` supprimée, remplacée par `category` (référencée). Ajout de 6
colonnes. `ruleset_code` est nullable (un scénario sans règles est légitime).

### 1.3 `sat_exchange_rate` (PK étendue)

Ancien :

```sql
CREATE TABLE sat_exchange_rate (
    currency_source TEXT,
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    PRIMARY KEY (currency_source, period)
);
```

Nouveau :

```sql
CREATE TABLE sat_exchange_rate (
    rate_set        TEXT,            -- nouvelle colonne, 1ère position
    currency_source TEXT,
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    PRIMARY KEY (rate_set, currency_source, period)
);
```

`rate_set` en première position pour cohérence avec la PK.

### 1.4 Ordre dans `ALL_DDL`

```
DDL_SEQ_ENTRY
DDL_APP_CONFIG                       ← nouveau (avant sat_exchange_rate)
DDL_DIM_SCENARIO_CATEGORY            ← nouveau (avant dim_scenario)
DDL_DIM_RATE_SET                     ← nouveau (avant sat_exchange_rate)
DDL_DIM_VARIANT                      ← nouveau (avant dim_scenario)
DDL_DIM_SCENARIO                     ← étendu
DDL_DIM_ENTITY
DDL_DIM_PERIOD
DDL_DIM_ACCOUNT
DDL_DIM_SOUS_CLASSE
DDL_DIM_FLOW
DDL_DIM_CURRENCY
DDL_DIM_NATURE
DDL_SAT_PERIMETER
DDL_SAT_EXCHANGE_RATE                ← étendue
DDL_DIM_RULE
DDL_DIM_RULESET
DDL_DIM_RULESET_ITEM
DDL_DIM_CUSTOM_DIMENSION             (survit aux resets)
DDL_STG_ENTRY
DDL_FACT_ENTRY
```

### 1.5 `ALL_DROP`

Ajout des 4 nouvelles tables (dans l'ordre inverse de création) :

```
DROP TABLE IF EXISTS dim_variant;
DROP TABLE IF EXISTS dim_rate_set;
DROP TABLE IF EXISTS dim_scenario_category;
DROP TABLE IF EXISTS app_config;
```

`dim_scenario` et `sat_exchange_rate` déjà présents.

---

## 2. SQL de cross-rate (étape C, `convert.rs`)

### 2.1 Principe

```
taux_cross(fonctionnelle → présentation) = taux(fonctionnelle → pivot) / taux(présentation → pivot)
```

Cas particuliers :
- `fonctionnelle = présentation` → `taux = 1.0`
- `présentation = pivot` → `taux_pres = 1.0` → `cross = taux_func` (cas du seed EUR)
- `fonctionnelle = pivot` → `taux_func = 1.0` → `cross = 1 / taux_pres`

### 2.2 CTE `params`

Pour éviter la prolifération de `?` et rester lisible, les 5 valeurs de
runtime sont bindées une fois dans une CTE `params` puis référencées par
`CROSS JOIN` dans le corps du SQL :

```sql
WITH params AS (
    SELECT
        ?::TEXT AS presentation,
        ?::TEXT AS pivot,
        ?::TEXT AS rate_set,
        ?::TEXT AS cur_period,
        ?::TEXT AS prev_period
),
conv AS (
    SELECT
        {f_cols}, f.amount,
        fl.taux_conversion,
        fl.flux_ecart,
        -- taux_flux = cross-rate applicable au flux
        CASE
            WHEN f.currency = p.presentation THEN 1.0
            ELSE
                (CASE
                    WHEN f.currency = p.pivot THEN 1.0
                    WHEN fl.taux_conversion = 'close_n1' THEN r_n1.taux_close
                    WHEN fl.taux_conversion = 'avg'      THEN r_n.taux_moyen
                    WHEN fl.taux_conversion IN ('close_n', 'terminal')
                        THEN r_n.taux_close
                END)
                /
                (CASE
                    WHEN p.presentation = p.pivot THEN 1.0
                    WHEN fl.taux_conversion = 'close_n1' THEN r_pres_n1.taux_close
                    WHEN fl.taux_conversion = 'avg'      THEN r_pres_n.taux_moyen
                    WHEN fl.taux_conversion IN ('close_n', 'terminal')
                        THEN r_pres_n.taux_close
                END)
        END AS taux_flux,
        -- taux_close_n = cross-rate au taux close_n (référence pour l'écart)
        CASE
            WHEN f.currency = p.presentation THEN 1.0
            ELSE
                (CASE WHEN f.currency = p.pivot THEN 1.0 ELSE r_n.taux_close END)
                /
                (CASE WHEN p.presentation = p.pivot THEN 1.0 ELSE r_pres_n.taux_close END)
        END AS taux_close_n
    FROM fact_entry f
    JOIN dim_flow fl ON fl.code = f.flow
    CROSS JOIN params p
    -- Taux de la devise fonctionnelle vers le pivot (N et N-1)
    LEFT JOIN sat_exchange_rate r_n
           ON r_n.rate_set        = p.rate_set
          AND r_n.currency_source = f.currency
          AND r_n.period          = p.cur_period
    LEFT JOIN sat_exchange_rate r_n1
           ON r_n1.rate_set        = p.rate_set
          AND r_n1.currency_source = f.currency
          AND r_n1.period          = p.prev_period
    -- Taux de la devise de présentation vers le pivot (N et N-1)
    LEFT JOIN sat_exchange_rate r_pres_n
           ON r_pres_n.rate_set        = p.rate_set
          AND r_pres_n.currency_source = p.presentation
          AND r_pres_n.period          = p.cur_period
    LEFT JOIN sat_exchange_rate r_pres_n1
           ON r_pres_n1.rate_set        = p.rate_set
          AND r_pres_n1.currency_source = p.presentation
          AND r_pres_n1.period          = p.prev_period
    WHERE f.level = 'reclassified'
)
INSERT INTO fact_entry ({insert_col_list}, level, amount)
SELECT {final_cols_convert}, 'converted', amount * taux_flux
FROM conv
UNION ALL
SELECT {final_cols_ecart}, 'converted', amount * (taux_close_n - taux_flux)
FROM conv
WHERE currency <> ?
  AND flux_ecart IS NOT NULL
  AND ABS(amount * (taux_close_n - taux_flux)) >= 0.005;
```

### 2.3 Paramètres `?` (ordre, 8 au total)

| # | Valeur                          | Rôle                                         |
|---|---------------------------------|----------------------------------------------|
| 1 | `presentation_currency`         | CTE `params.presentation`                    |
| 2 | `pivot_currency`                | CTE `params.pivot`                           |
| 3 | `rate_set`                      | CTE `params.rate_set`                        |
| 4 | `current_period`                | CTE `params.cur_period`                      |
| 5 | `prev_period`                   | CTE `params.prev_period`                     |
| 6 | `presentation_currency`         | `final_cols_convert` (colonne `currency`)    |
| 7 | `presentation_currency`         | `final_cols_ecart` (colonne `currency`)      |
| 8 | `presentation_currency`         | `WHERE currency <> ?` (filtre écart)         |

L'écart avec le comptage « 9 » de la spec fonctionnelle : la CTE `params`
mutualise les références à `pivot` et `rate_set` (sinon 4 `?` pour `rate_set`
seul dans les JOIN, et plusieurs pour `pivot` dans les CASE). Le compte exact
importe peu ; la logique, elle, doit être strictement celle du §2.1.

### 2.4 Vérification de l'invariant seed

Pour `pivot = EUR` et `presentation = EUR` (configuration du seed) :

- Pour toute devise fonctionnelle `X` non-EUR :
  - `taux_func(X)` = taux lu dans `sat_exchange_rate` (ex. USD/2024 close = 0.90)
  - `taux_pres(EUR)` = `1.0` (car `presentation = pivot` → CASE court-circuit)
  - `cross = taux_func / 1.0 = taux_func` → **identique au comportement historique**

Les 16 tests doivent rester verts sans modification de leurs assertions
numériques.

---

## 3. Signatures Rust

### 3.1 `ConvertParams` (pipeline/mod.rs)

```rust
#[derive(Debug, Clone)]
pub struct ConvertParams {
    pub presentation_currency: String,
    pub pivot_currency:        String,
    pub current_period:        String,
    pub prev_period:           String,
    pub rate_set:              String,
    pub scenario_code:         String,
}

// impl Default — SUPPRIMÉ.

impl ConvertParams {
    /// Charge les paramètres d'un run depuis `dim_scenario` + `app_config`.
    /// Dérive `prev_period` depuis `dim_period`.
    pub fn load_params(
        con: &duckdb::Connection,
        scenario_code: &str,
    ) -> duckdb::Result<Self> {
        // 1. Lecture (scenario, pivot, current_period, rate_set) en une requête.
        // 2. Dérivation de prev_period.
        // 3. Retourne ConvertParams complet.
    }
}
```

### 3.2 Requêtes SQL de `load_params`

Lecture scénario + pivot :

```sql
SELECT s.presentation_currency,
       COALESCE((SELECT value FROM app_config WHERE key = 'pivot_currency'), 'EUR'),
       s.entry_period,
       s.rate_set
FROM dim_scenario s
WHERE s.code = ?;
```

Dérivation `prev_period` :

```sql
SELECT p2.code
FROM dim_period p1
JOIN dim_period p2
  ON p2.date_fin < p1.date_debut
 AND p2.type = 'exercice'
WHERE p1.code = ?
ORDER BY p2.date_fin DESC
LIMIT 1;
```

Si aucune période N-1 trouvée → `Err` (un run nécessite N et N-1).

### 3.3 Export depuis `lib.rs`

```rust
pub use pipeline::{run_pipeline, ConvertParams};
```

`ConvertParams::load_params` est accessible via `conso_engine::ConvertParams::load_params`
ou `conso_engine::pipeline::ConvertParams::load_params`.

---

## 4. Changements par module

### 4.1 `schema.rs`

- Nouvelles constantes : `DDL_APP_CONFIG`, `DDL_DIM_RATE_SET`,
  `DDL_DIM_VARIANT`, `DDL_DIM_SCENARIO_CATEGORY`.
- `DDL_DIM_SCENARIO` : refonte (suppression `type`, ajout des 6 colonnes).
- `DDL_SAT_EXCHANGE_RATE` : ajout colonne `rate_set` (1ère position), PK étendue.
- `ALL_DDL` : insertion dans l'ordre §1.4.
- `ALL_DROP` : ajout des 4 nouvelles tables.

### 4.2 `seed.rs`

Données étendues pour préserver la cohérence numérique (pivot=EUR=présentation) :

```text
app_config                : ('pivot_currency', 'EUR')
dim_scenario_category     : ('REEL', 'Réel')
dim_variant               : ('BASE', 'Base')
dim_rate_set              : ('RATES', 'Taux réels')
dim_scenario              : ('REEL', 'Réel 2024', 'REEL', '2024', 'EUR', 'BASE', NULL, 'RATES', 'ouvert')
sat_exchange_rate         : mêmes taux qu'aujourd'hui, avec rate_set = 'RATES'
```

Toutes les autres données (entités, périodes, comptes, flux, périmètre,
saisie brute) sont inchangées.

### 4.3 `pipeline/convert.rs`

Réécriture du SQL comme détaillé au §2. Insertion d'une CTE `params` en tête.
Les 3 `?` résiduels restent liés à la décoration de la devise dans les SELECT
finaux (`final_cols_convert`, `final_cols_ecart`) et au filtre d'écart.

### 4.4 `masterdata.rs`

Nouvelles `TableDef` (CRUD accessible depuis l'UI) :
- `scenario_categories` → `dim_scenario_category (code, libelle)` — PK `code`
- `variants`            → `dim_variant (code, libelle)` — PK `code`
- `rate_sets`           → `dim_rate_set (code, libelle)` — PK `code`

`scenarios` étendue : colonnes `code, libelle, category, entry_period,
presentation_currency, variant, ruleset_code, rate_set, statut`.

`rates` étendue : colonnes `rate_set, currency_source, period, taux_close,
taux_moyen` ; PK `(rate_set, currency_source, period)`.

`app_config` **n'est pas** exposée via masterdata CRUD (config système).

### 4.5 `loader.rs`

Nouveaux fichiers CSV à charger depuis `data/` :

| Fichier                    | Table cible              |
|----------------------------|--------------------------|
| `scenario_categories.csv`  | `dim_scenario_category`  |
| `variants.csv`             | `dim_variant`            |
| `rate_sets.csv`            | `dim_rate_set`           |
| `app_config.csv`           | `app_config`             |

`scenarios.csv` : nouvelles colonnes (en-tête modifié).
`rates.csv` : nouvelle colonne `rate_set` en première position.

### 4.6 `bin/server.rs`

`POST /api/run` accepte désormais :

```json
{"scenario": "REEL"}
```

Body `{}` ou absent → utilise le premier scénario de statut `'ouvert'`
(rétro-compatibilité dev).

Workflow handler :
1. Résolution du code scénario (body explicite ou premier `'ouvert'`).
2. `ConvertParams::load_params(con, scenario_code)`.
3. `DELETE FROM fact_entry`.
4. `run_pipeline(con, &params)`.
5. Si `scenario.ruleset_code` non NULL : `run_ruleset(con, ruleset_code)`.
6. Retourne `{ counts, ruleset_report? }`.

L'endpoint `GET /api/scenarios` (nouveau) expose la liste des scénarios avec
leurs paramètres dépliés (pour le dropdown UI).

### 4.7 `main.rs` et `bin/bench.rs`

Ces binaires utilisaient `ConvertParams::default()`. Remplacement par
`ConvertParams::load_params(&con, "REEL")`.

`bench.rs` doit en plus :
- Générer `app_config` (`pivot_currency = 'EUR'`), `dim_scenario_category`,
  `dim_variant`, `dim_rate_set` avant `dim_scenario`.
- Adapter `INSERT INTO dim_scenario` au nouveau schéma.
- Ajouter `rate_set = 'RATES'` aux `INSERT INTO sat_exchange_rate`.

### 4.8 `tests/pipeline.rs`

`ConvertParams::default()` est remplacé par `load_params(&con, "REEL")` dans
`setup()` et `pipeline_reproductible_apres_reset`. Les assertions numériques
sont strictement inchangées.

---

## 5. Frontend (`web/src/pages/PipelinePage.tsx`)

Remplacement du bouton « Exécuter le pipeline » par un formulaire :

```
[Scénario ▼] [params en lecture seule] [Lancer la consolidation]
```

- `GET /api/scenarios` → liste des scénarios (`code`, `libelle`, params).
- Sélection → affichage en lecture seule des paramètres dépliés (devise,
  période, catégorie, variante, rate_set, ruleset).
- Bouton « Lancer » → `POST /api/run { scenario: code }`.

`api.ts` : ajout de `api.scenarios.list()` et modification de `api.run(scenario)`.

---

## 6. Plan de validation

1. `cargo check` après chaque étape de code.
2. `cargo test` : les 16 tests existants doivent rester verts **sans modifier
   leurs assertions numériques**. C'est le critère d'acceptation principal du
   cross-rate (pivot=EUR=présentation → valeurs identiques).
3. Tests runtime HTTP (POST /api/run avec scénario) : dévolus à l'utilisateur
   (pattern `Start-Process` selon `AGENTS.md`).
