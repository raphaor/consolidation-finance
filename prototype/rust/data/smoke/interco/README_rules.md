# Règle R-INT — Élimination interco standard (smoke)

> ⚠ **Le moteur ne charge pas les règles depuis CSV** — `loader.rs` n'a pas de
> mapping pour `dim_rule` / `dim_ruleset` / `dim_ruleset_item` (contrairement
> aux autres master data). Cette règle est fournie dans
> [`rule_R-INT.json`](./rule_R-INT.json) et doit être injectée via l'**API REST**
> ou par **`seed_all`** (Rust in-memory).

## Injecter via l'API REST

Démarrer le serveur (cf. [`AGENTS.md`](../../../../AGENTS.md)), puis :

```powershell
# 1. Créer la règle (le fichier .json EST la définition, on l'envoie telle quelle)
$definition = Get-Content "data/smoke/interco/rule_R-INT.json" -Raw
$body = @{ code = "R-INT"; libelle = "Élimination interco"; definition = $definition } | ConvertTo-Json -Depth 20
Invoke-RestMethod -Method POST -Uri "http://localhost:3000/api/rules" -Body $body -ContentType "application/json"

# 2. Créer le ruleset et y référencer la règle
$rs = @{ code = "RS_INTERCO"; libelle = "Ruleset interco smoke" } | ConvertTo-Json
Invoke-RestMethod -Method POST -Uri "http://localhost:3000/api/rulesets" -Body $rs -ContentType "application/json"
$item = @{ rule_code = "R-INT" } | ConvertTo-Json
Invoke-RestMethod -Method POST -Uri "http://localhost:3000/api/rulesets/RS_INTERCO/items" -Body $item -ContentType "application/json"

# 3. Le scénario SMOKE_IC référence déjà RS_INTERCO dans scenarios.csv
#    → /api/run l'exécutera automatiquement après le pipeline.
```

## Injecter en Rust (test in-memory)

```rust
use conso_engine::{create_schema, loader, rules::run_ruleset};
use duckdb::Connection;

let con = Connection::open_in_memory().unwrap();
create_schema(&con).unwrap();
loader::load_all(&con, std::path::Path::new("data/smoke/interco")).unwrap();

let definition = std::fs::read_to_string("data/smoke/interco/rule_R-INT.json").unwrap();
con.execute(
    "INSERT INTO dim_rule (code, libelle, definition) VALUES ('R-INT', 'Élim interco', ?)",
    [&definition],
).unwrap();
con.execute(
    "INSERT INTO dim_ruleset (code, libelle) VALUES ('RS_INTERCO', 'Smoke interco')",
    [],
).unwrap();
con.execute(
    "INSERT INTO dim_ruleset_item (ruleset_code, ordre, rule_code) VALUES ('RS_INTERCO', 1, 'R-INT')",
    [],
).unwrap();
```

## Évolution possible

Ajouter 3 entrées à `CSV_MAPPINGS` dans [`src/loader.rs`](../../../../src/loader.rs)
(`rules.csv` → `dim_rule`, `rulesets.csv` → `dim_ruleset`,
`ruleset_items.csv` → `dim_ruleset_item`) pouruniformiser le chargement.
Facultatif — l'absence actuelle privilégie la **création via UI/API** (workflow
éditeur) plutôt qu'un import bulk.

## Dépendances moteur (à implémenter avant exécution)

1. Coef **`min_pct_integration`** dans [`src/rules.rs`](../../../../src/rules.rs)
   (parse + `coefficient_expr`).
2. Filtre **`nature NOT LIKE '2%'`** dans `pipeline::consolidate` (sinon les
   écritures `2ELI` seraient re-multipliées par `pct_integration`).
