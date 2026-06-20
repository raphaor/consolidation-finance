# Spécification technique — Moteur de règles de consolidation

> Ce document est destiné à opencode. Il décrit l'implémentation complète du
> module de règles de consolidation dans le moteur Rust existant.

## Contexte du projet

Moteur de consolidation financière en Rust + DuckDB + Axum.
Code source : `~/cf-clone/prototype/rust/`
Le serveur écoute sur `:3000`.

### Architecture existante

- `src/schema.rs` — DDL DuckDB : `ALL_DDL`, `ALL_DROP`, `create_schema()`
- `src/lib.rs` — déclarations de modules + re-exports
- `src/bin/server.rs` — serveur Axum, routes API, AppState
- `src/masterdata.rs` — CRUD générique sur les tables master data
- `src/state.rs` — `AppState` (Mutex<Connection> + csv_dir), `AppError`, `lock_con()`, `db_err()`
- `src/pipeline/mod.rs` — pipeline 4 étapes (A→B→C→D), `run_pipeline()`
- `src/pipeline/materialize_closures.rs` — reconstruction des clôtures F99
- `src/pipeline/staging.rs` — injection par préfixe de nature

### Modèle de données existant

`fact_entry` a ces colonnes :
```
id, scenario, entity, entry_period, period, account, flow,
currency, nature, partner, share, analysis, audit_id, level, amount
```

4 niveaux dans `level` : corporate, reclassified, converted, consolidated.

`sat_perimeter` a ces colonnes :
```
entity, scenario, period, methode, pct_interet, pct_integration, entree, sortie
PRIMARY KEY (entity, scenario, period)
```

---

## Ce qu'il faut implémenter

### 1. Nouvelles tables (dans `src/schema.rs`)

Ajouter 3 tables au DDL. Les ajouter à `ALL_DDL` et `ALL_DROP`.

```sql
-- Règles : bibliothèque centrale
CREATE TABLE dim_rule (
    code        TEXT PRIMARY KEY,
    libelle     TEXT,
    definition  TEXT          -- JSON : scope + operations (voir §2)
);

-- Jeux de règles
CREATE TABLE dim_ruleset (
    code        TEXT PRIMARY KEY,
    libelle     TEXT
);

-- Items ordonnés d'un jeu (références vers des règles)
CREATE TABLE dim_ruleset_item (
    ruleset_code TEXT,
    ordre        INTEGER,
    rule_code    TEXT,
    PRIMARY KEY (ruleset_code, ordre)
);
```

### 2. Format JSON de définition d'une règle

Le champ `dim_rule.definition` contient un JSON de cette forme :

```json
{
  "scope": [
    {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
    {"target": "partner", "dim": "methode", "op": "=", "val": "globale"}
  ],
  "operations": [
    {
      "seq": 1,
      "level": "consolidated",
      "selection": [
        {"dim": "partner", "op": "IS NOT NULL"}
      ],
      "coefficient": {"type": "pct_integration"},
      "multiplicateur": -1,
      "destination": {
        "nature":  {"mode": "override", "value": "2ELI"},
        "partner": {"mode": "inherit"}
      }
    }
  ]
}
```

#### Détails du format

**scope** : liste de conditions sur le périmètre. Chaque condition a :
- `target` : `"entity"` ou `"partner"` (sur quelle entité filtrer)
- `dim` : nom de colonne dans `sat_perimeter` (`methode`, `pct_interet`, `pct_integration`, `entree`, `sortie`)
- `op` : opérateur (`=`, `!=`, `>`, `<`, `>=`, `<=`, `IN`)
- `val` : valeur (string, ou liste pour `IN`)
- Peut être vide (`[]`) → pas de condition de périmètre

**operations** : liste ordonnée d'opérations. Chaque opération a :
- `seq` : numéro de séquence (entier)
- `level` : niveau de sélection ET d'écriture (`corporate`, `reclassified`, `converted`, `consolidated`)
- `selection` : liste de filtres sur les dimensions de `fact_entry`. Chaque filtre :
  - `dim` : nom de colonne (`account`, `flow`, `nature`, `partner`, `entity`, `share`, etc.)
  - `op` : opérateur (`=`, `!=`, `IS NULL`, `IS NOT NULL`, `IN`)
  - `val` : valeur (string ou liste pour `IN`). Absent pour `IS NULL` / `IS NOT NULL`.
- `coefficient` : `{"type": "pct_integration"}` ou `{"type": "pct_interet"}` ou `{"type": "constant", "value": 1.0}` ou absent (défaut = 1.0)
- `multiplicateur` : entier ou décimal, typiquement 1 ou -1. Défaut = 1.
- `destination` : objet clé/valeur. Les clés sont les dimensions pilotables : `entity`, `account`, `flow`, `nature`, `partner`, `share`. Les dimensions absentes de l'objet sont héritées par défaut. Chaque valeur :
  - `{"mode": "inherit"}` — reprend la valeur source
  - `{"mode": "override", "value": "X"}` — remplace par X
  - `{"mode": "null"}` — met à NULL

**Dimensions toujours héritées** (non pilotables) : `scenario`, `entry_period`, `period`, `currency`, `analysis`.

### 3. Module moteur (`src/rules.rs`)

Créer un nouveau module `src/rules.rs` avec :

```rust
pub fn run_ruleset(con: &Connection, ruleset_code: &str) -> Result<RulesetReport, duckdb::Error>;
```

#### Algorithme d'exécution

```
Pour chaque règle du ruleset (dans l'ordre des dim_ruleset_item.ordre) :
    1. Lire dim_rule.definition (JSON), le parser
    2. Pour chaque niveau L distinct parmi les opérations de la règle :
       CREATE TEMP TABLE _rule_snap_L AS SELECT * FROM fact_entry WHERE level = 'L'
    3. Pour chaque opération (indépendantes — toutes lisent le snapshot) :
       a. Construire la requête SQL INSERT ... SELECT depuis _rule_snap_L
       b. Les filtres de sélection → clause WHERE
       c. Les conditions de scope → JOINs sur sat_perimeter
       d. Le coefficient → si pct_integration/pct_interet, JOIN sat_perimeter pour la valeur
       e. Le multiplicateur → multiplication dans le SELECT
       f. La destination → colonnes du SELECT (inherit=e.col, override='val', null=NULL)
       g. audit_id = 'RULE:{rule_code}:{seq}'
       h. Exécuter l'INSERT
    4. Pour chaque niveau L :
       DROP TABLE _rule_snap_L
       materialize_closures(con, L)   -- reconstruire F99
```

#### Génération SQL pour une opération

Pour une opération au niveau `L` avec sélection, scope, coefficient, multiplicateur, destination :

```sql
INSERT INTO fact_entry (
    scenario, entity, entry_period, period, account, flow,
    currency, nature, partner, share, analysis, audit_id, level, amount
)
SELECT
    e.scenario,        -- always inherited
    <entity_dest>,     -- entity: inherited or overridden
    e.entry_period,    -- always inherited
    e.period,          -- always inherited
    <account_dest>,    -- account: inherited or overridden
    <flow_dest>,       -- flow: inherited or overridden
    e.currency,        -- always inherited
    <nature_dest>,     -- nature: inherited or overridden
    <partner_dest>,    -- partner: inherited, overridden, or NULL
    <share_dest>,      -- share: inherited or overridden
    e.analysis,        -- always inherited
    ?,                 -- audit_id = 'RULE:{rule_code}:{seq}'
    ?,                 -- level = same as selection level
    e.amount * <coeff> * <mult>   -- factored amount
FROM _rule_snap_{L} e
[JOIN sat_perimeter p_ent ON p_ent.entity = e.entity
    AND p_ent.scenario = e.scenario
    AND p_ent.period = e.entry_period
    AND {scope_conditions_on_entity}]
[JOIN sat_perimeter p_part ON p_part.entity = e.partner
    AND p_part.scenario = e.scenario
    AND p_part.period = e.entry_period
    AND {scope_conditions_on_partner}]
WHERE {selection_conditions}
```

Les JOINs sur sat_perimeter sont ajoutés si :
- Il y a des conditions de scope sur entity → JOIN p_ent
- Il y a des conditions de scope sur partner → JOIN p_part
- Le coefficient est pct_integration ou pct_interet → JOIN p_ent (pour récupérer la valeur)

Si le coefficient est `constant` ou absent → pas de JOIN pour le coefficient (utiliser la valeur littérale).

Les conditions de scope sont ajoutées comme clauses AND dans le ON du JOIN.

Le coefficient en SQL :
- `pct_integration` → `COALESCE(p_ent.pct_integration, 1)`
- `pct_interet` → `COALESCE(p_ent.pct_interet, 1)`
- `constant` ou absent → la valeur littérale (ex: `1.0`)

#### Report

```rust
#[derive(Debug, Clone, Serialize)]
pub struct RuleResult {
    pub rule_code: String,
    pub level: String,
    pub generated: usize,   // nombre de lignes générées
}

#[derive(Debug, Clone, Serialize)]
pub struct RulesetReport {
    pub ruleset: String,
    pub rules: Vec<RuleResult>,
    pub total_generated: usize,
}
```

### 4. API REST (dans `src/bin/server.rs` ou nouveau module)

Ajouter ces routes au serveur :

**CRUD règles** (comme masterdata mais dédié) :
- `GET /api/rules` → liste toutes les règles (code, libelle)
- `GET /api/rules/:code` → détail d'une règle (code, libelle, definition parsée en JSON)
- `POST /api/rules` → crée une règle `{code, libelle, definition}`
- `PUT /api/rules/:code` → modifie
- `DELETE /api/rules/:code` → supprime

**CRUD rulesets** :
- `GET /api/rulesets` → liste tous les jeux
- `GET /api/rulesets/:code` → détail + items ordonnés (avec jointure sur dim_rule pour libellés)
- `POST /api/rulesets` → crée `{code, libelle, items: [{ordre, rule_code}]}`
- `PUT /api/rulesets/:code` → modifie (incluant réordonnancement des items)
- `DELETE /api/rulesets/:code` → supprime le jeu + ses items

**Exécution** :
- `POST /api/rules/run` → body `{"ruleset": "CODE"}` → exécute le ruleset contre fact_entry, renvoie RulesetReport

### 5. Intégration

- Ajouter `pub mod rules;` dans `src/lib.rs`
- Ajouter les routes dans le Router du server.rs
- Les tables de règles doivent être créées par `create_schema()` (ajouter dans ALL_DDL/ALL_DROP)
- Les tables de règles doivent survivre à un POST /api/reset (qui fait DROP+CREATE)

### 6. Tests

Créer `rules_test.py` (Python, même style que `golden_test.py` et `smoke_test.py`) qui :

1. Démarre le serveur avec le dataset golden existant (`data_golden/`)
2. POST /api/run (pipeline natif)
3. Récupère le consolidated avant règles (baseline)
4. Crée une règle d'élimination interco via POST /api/rules :
   - scope : entity.methode = globale, partner.methode = globale
   - 4 opérations (extourne + contrepartie × partner hérité/vidé)
5. Crée un ruleset contenant cette règle
6. POST /api/rules/run avec ce ruleset
7. Vérifie :
   - De nouvelles lignes sont apparues au niveau consolidated avec nature=2ELI
   - Le solde interco (partner non null) est extourné
   - Le bilan agrégé (partner null) est équilibré
8. Tue le serveur, exit 0 ou 1

Pour le test, il faut ajouter des écritures interco dans le dataset golden.
Ajouter dans `data_golden/entries.csv` quelques lignes avec `partner` rempli
(par exemple M vend à G sur le compte 700 pour 1000 EUR).

### Conventions

- **Langue** : commentaires et messages en français
- **Style** : suivre les conventions existantes (même style que le reste du code)
- **SQL** : utiliser des requêtes paramétrées (`?`) partout où possible pour les valeurs
- **Erreurs** : utiliser `AppError` et `db_err` existants
- **JSON** : utiliser `serde_json` pour parser/sérialiser les définitions de règles
- **Tests** : `urllib` stdlib (pas de `requests`)

### IMPORTANT

1. Ne pas casser les tests existants : `cargo test`, `smoke_test.py`, `golden_test.py`
2. Les nouvelles tables doivent être dans `create_schema()` et `ALL_DDL`/`ALL_DROP`
3. Compiler avec `cargo build --release --bin conso-server` et vérifier que ça compile
4. Lancer `python3 golden_test.py` pour vérifier que les tests existants passent encore
5. Lancer `python3 rules_test.py` pour vérifier les nouveaux tests
