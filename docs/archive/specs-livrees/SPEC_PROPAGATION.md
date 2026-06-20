# Tâche : Propagation des dimensions optionnelles + renommage audit_id → analysis2

## Contexte
Le pipeline propage actuellement `partner` dans le grain (GROUP BY + SELECT) des 5 modules (aggregate, reclassify, convert, consolidate, staging). Mais `share`, `analysis` et `audit_id` sont absents des requêtes — elles arrivent à NULL à tous les niveaux après l'agrégation.

## Changements requis

### 1. Renommage `audit_id` → `analysis2` (PARTOUT)
Dans TOUS les fichiers (.rs, .py, .csv), renommer `audit_id` en `analysis2`.
- `schema.rs` : `stg_entry` et `fact_entry` (colonne + commentaires)
- `rules.rs` : `ALLOWED_SELECTION_DIMS`, `exec_operation` (le moteur tagge toujours `analysis2` avec `RULE:{rule_code}:{seq}`)
- Tous les CSV dans `data_golden/` (en-tête de colonne)
- Tous les fichiers de test `.py`

### 2. Propagation de `share`, `analysis`, `analysis2` dans le pipeline
Les 5 modules doivent propager ces 3 dimensions exactement comme `partner` l'est déjà :
- `src/pipeline/aggregate.rs` : ajouter au GROUP BY + INSERT + SELECT
- `src/pipeline/reclassify.rs` : ajouter au GROUP BY + INSERT + SELECT (les 4 branches SELECT + le GROUP BY final)
- `src/pipeline/convert.rs` : ajouter au SELECT des lignes converties ET des lignes d'écart
- `src/pipeline/consolidate.rs` : ajouter au INSERT + SELECT
- `src/pipeline/staging.rs` (inject_by_prefix) : ajouter au GROUP BY + INSERT + SELECT

### 3. materialize_closures.rs
NE PAS modifier le grain de reconstruction. Les 4 dimensions optionnelles (`partner`, `share`, `analysis`, `analysis2`) restent HORS grain pour les clôtures F99 (ce sont des soldes agrégés). Juste mettre à jour le commentaire d'en-tête pour mentionner les 4 dimensions.

### 4. rules.rs
- `ALLOWED_SELECTION_DIMS` : renommer `audit_id` → `analysis2`
- `PILOTABLE_DIMS` reste `["entity", "account", "flow", "nature", "partner", "share"]` — `analysis` et `analysis2` ne sont PAS pilotables
- Dans `exec_operation`, le moteur continue de setter `analysis2` automatiquement à `RULE:{rule_code}:{seq}` (comportement inchangé, juste le nom de colonne qui change)

### 5. Ordre des colonnes dans fact_entry / stg_entry
L'ordre des colonnes devient : scenario, entity, entry_period, period, account, flow, currency, nature, partner, share, analysis, analysis2, level, amount

## Vérification finale
1. `cargo build --release --bin conso-server`
2. `cargo test --release`
3. `python3 golden_test.py` → 28/28
4. `python3 rules_test.py` → 33/33 (ou 32/32 si le test 7a' sur audit_id a été ajusté)
5. `python3 smoke_test.py` → 59/59
