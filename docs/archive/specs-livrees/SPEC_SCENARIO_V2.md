# SPEC : Scénario de consolidation v2 et taux pivot

> ⚠️ **SUPERSEDÉ par [Q41]** (redesign identité, 2026-06-23). L'objet `dim_scenario` décrit ici a
> été renommé `dim_consolidation` et repensé : PK technique `id` auto + clé naturelle
> `(phase, exercice, perimeter_set, variant, presentation_currency)` ; `code` supprimé ;
> `category`→`phase`, `entry_period`→`exercice`, `a_nouveau_scenario`→`a_nouveau_consolidation_id` ;
> périodes explicites `perimeter_period`/`rate_period`. Voir `MODELE_DONNEES.md` §3 et
> `QUESTIONS_OUVERTES.md` Q41. **Spec livrée conservée pour l'historique — ne pas réécrire.**

*19 juin 2026 — Rédigé par Hermes, à implémenter par Opencode*

## Objectif

Redéfinir le scénario de consolidation comme un objet composite unique qui
agrège toutes les références nécessaires à un run. Introduire le concept de
taux pivot au niveau application. Rendre les paramètres de run entièrement
dynamiques (suppression du `ConvertParams::default()` hardcoded).

Cette spec remplace la proposition P3 de `REVUE_DYNAMISME.md` et l'étend
significativement.

---

## 1. Taux pivot applicatif

### Principe

L'application utilise une **devise pivot unique** pour toutes les conversions
de devises. Tous les taux stockés dans `sat_exchange_rate` convertissent vers
cette devise pivot.

Le pivot est défini **une fois pour toute l'application**. Une autre instance
(déploiement) de l'application pourrait utiliser un pivot différent, mais au
sein d'une même instance, le pivot est invariant.

### Configuration

```sql
CREATE TABLE app_config (
    key   TEXT PRIMARY KEY,
    value TEXT
);
-- Insert initial : ('pivot_currency', 'EUR')
```

Le pivot se lit au runtime via :
```sql
SELECT value FROM app_config WHERE key = 'pivot_currency';
```

### Cross-rate

Pour convertir une devise fonctionnelle vers la devise de présentation :

```
taux_cross = taux(fonctionnelle → pivot) / taux(présentation → pivot)
```

Cas particuliers :
- Si fonctionnelle = présentation : taux = 1.0 (déjà géré)
- Si présentation = pivot : taux_pres = 1.0, cross = taux_func (comportement actuel EUR)
- Si fonctionnelle = pivot : taux_func = 1.0, cross = 1 / taux_pres

### Impact sur convert.rs

Le SQL de `step_c` gagne un JOIN supplémentaire pour le taux de la devise de
présentation vers le pivot. Les taux applicables au flux (taux_flux) et le
taux de référence (taux_close_n) deviennent des cross-rates (division).

Les paramètres `?` passent de 7 à 9 (ajout de `pivot_currency` et `rate_set`).
Voir la spec technique pour le SQL exact.

---

## 2. Jeux de taux (rate sets)

### Besoin

Différents scénarios peuvent utiliser des jeux de taux différents (taux réels
vs taux budget). Le scénario référence le jeu de taux à utiliser.

### dim_rate_set

```sql
CREATE TABLE dim_rate_set (
    code    TEXT PRIMARY KEY,   -- 'RATES_EUR', 'BUDGET', etc.
    libelle TEXT                -- 'Taux réels', 'Taux budget', etc.
);
```

### sat_exchange_rate étendue

```sql
CREATE TABLE sat_exchange_rate (
    rate_set        TEXT,        -- FK dim_rate_set
    currency_source TEXT,        -- devise convertie vers le pivot
    period          TEXT,
    taux_close      DECIMAL(18,8),
    taux_moyen      DECIMAL(18,8),
    PRIMARY KEY (rate_set, currency_source, period)
);
```

Le PK passe de `(currency_source, period)` à `(rate_set, currency_source, period)`.

---

## 3. Variante

### Besoin

Décliner un même cadre (catégorie + période + devise de présentation) avec des
hypothèses différentes (ex: scénario optimiste vs pessimiste).

```sql
CREATE TABLE dim_variant (
    code    TEXT PRIMARY KEY,   -- 'BASE', 'OPT1', 'PESSIMIST'
    libelle TEXT
);
```

---

## 4. Catégorie de scénario

### Besoin

Le champ libre `type` actuel de `dim_scenario` devient une véritable dimension
referencée. Pour l'instant il n'y a que "réel", mais "budget" et "prévision"
viendront.

```sql
CREATE TABLE dim_scenario_category (
    code    TEXT PRIMARY KEY,   -- 'REEL', 'BUDGET', 'PREVISION'
    libelle TEXT
);
```

---

## 5. dim_scenario v2

Le scénario devient le **point d'entrée unique** d'un run. Toutes les
informations nécessaires sont lues depuis cette ligne + `app_config`.

```sql
CREATE TABLE dim_scenario (
    code                  TEXT PRIMARY KEY,
    libelle               TEXT,
    category              TEXT,   -- FK dim_scenario_category ('REEL', 'BUDGET'…)
    entry_period          TEXT,   -- FK dim_period ('2024')
    presentation_currency TEXT,   -- FK dim_currency ('EUR')
    variant               TEXT,   -- FK dim_variant ('BASE')
    ruleset_code          TEXT,   -- FK dim_ruleset (NULL = pas de règles)
    rate_set              TEXT,   -- FK dim_rate_set
    statut                TEXT    -- 'ouvert' / 'verrouillé' (existant)
);
```

**Colonnes supprimées** vs ancien schéma : `type` (remplacé par `category`).
**Colonnes conservées** : `code`, `libelle`, `statut`.

### Dérivation de prev_period

`current_period = scenario.entry_period`.

`prev_period` est **dérivé** depuis `dim_period` : la période de type
'exercice' dont `date_fin` précède immédiatement celle de `entry_period`.

```sql
SELECT p2.code FROM dim_period p1
JOIN dim_period p2
  ON p2.date_fin < p1.date_debut
 AND p2.type = 'exercice'
WHERE p1.code = ?  -- entry_period
ORDER BY p2.date_fin DESC
LIMIT 1;
```

Si aucune période précédente n'est trouvée → erreur (un run nécessite N et N-1).

---

## 6. ConvertParams (runtime)

Plus de `impl Default`. Les paramètres sont chargés depuis la DB via :

```rust
pub fn load_params(con: &Connection, scenario_code: &str) -> Result<ConvertParams>;
```

Struct étendue :

```rust
pub struct ConvertParams {
    pub presentation_currency: String,   // from scenario
    pub pivot_currency: String,          // from app_config
    pub current_period: String,          // = scenario.entry_period
    pub prev_period: String,             // derived from dim_period
    pub rate_set: String,                // from scenario
    pub scenario_code: String,           // le code scénario (pour filtrer)
}
```

---

## 7. POST /api/run

Le handler accepte désormais un code scénario :

```
POST /api/run
{"scenario": "REEL_2024"}
```

Workflow du handler :
1. `ConvertParams::load_params(con, scenario_code)`
2. `DELETE FROM fact_entry` (nettoyage, comme aujourd'hui)
3. `run_pipeline(con, &params)`
4. Si `scenario.ruleset_code` non NULL : `run_ruleset(con, ruleset_code)`
5. Retourner les comptes + le rapport de règles

Le handler peut accepter un body vide `{}` auquel cas il utilise le premier
scénario de statut 'ouvert' (pour la rétro-compatibilité pendant le dev).

---

## 8. Effets sur les modules existants

### schema.rs

- Nouvelle table : `app_config`
- Nouvelle table : `dim_rate_set`
- Nouvelle table : `dim_variant`
- Nouvelle table : `dim_scenario_category`
- `dim_scenario` : refonte complète (nouvelles colonnes)
- `sat_exchange_rate` : ajout colonne `rate_set`, nouvelle PK
- Ordre de création : `app_config` et `dim_rate_set` avant `sat_exchange_rate`
  (FK logique). `dim_scenario_category`, `dim_variant`, `dim_ruleset` avant
  `dim_scenario`.
- `ALL_DROP` : ajouter les nouvelles tables.

### masterdata.rs

- Nouvelles `TableDef` : `scenario_categories`, `variants`, `rate_sets`
- `scenarios` : colonnes étendues
- `rates` : colonne `rate_set` ajoutée
- `app_config` : **ne pas** exposer via masterdata CRUD (config système).

### loader.rs

- Nouveaux CSV à charger : `scenario_categories.csv`, `variants.csv`,
  `rate_sets.csv`
- `scenarios.csv` : nouvelles colonnes
- `rates.csv` : nouvelle colonne `rate_set`
- `app_config.csv` : chargé une fois (1 ligne : pivot_currency)

### seed.rs

Données de test étendues pour garder tous les tests existants verts :
- `app_config` : `('pivot_currency', 'EUR')`
- `dim_rate_set` : `('RATES', 'Taux réels')`
- `dim_variant` : `('BASE', 'Base')`
- `dim_scenario_category` : `('REEL', 'Réel')`
- `dim_scenario` : un scénario complet, ex `('REEL', 'Réel 2024', 'REEL', '2024', 'EUR', 'BASE', NULL, 'RATES', 'ouvert')`
- `sat_exchange_rate` : mêmes taux qu'aujourd'hui mais avec `rate_set = 'RATES'`

**Contrainte absolue** : pivot = EUR = présentation. Donc tous les
cross-rates doivent produire les mêmes valeurs numériques que les taux directs
actuels. Les tests golden (28/28) doivent rester verts sans modification de
leurs assertions numériques.

### convert.rs

Le SQL gagne :
- Un JOIN supplémentaire `r_pres` sur `sat_exchange_rate` pour le taux de la
  présentation vers le pivot
- `taux_flux` et `taux_close_n` deviennent des divisions (cross-rate)
- Les filtres `f.currency = ?` (présentation) restent
- Nouveaux paramètres `?` : `pivot_currency` (pour le CASE), `rate_set` (pour
  les JOIN)

### pipeline/mod.rs

- `ConvertParams` : supprimer `impl Default`, ajouter `load_params()`
- `run_pipeline` et `run_pipeline_timed` : inchangés (reçoivent déjà `&ConvertParams`)

### server.rs

- `run_pipeline_handler` : lit le body `{scenario}`, charge les params, run
  pipeline + ruleset
- Nouvel endpoint ou extension : `GET /api/scenarios` pourrait lister les
  scénarios avec leurs params (pour l'UI)

---

## 9. UI (front-end)

### Formulaire de run

Remplace le simple bouton "Run" actuel par un sélecteur de scénario :
- Dropdown des scénarios (code + libellé)
- Affichage en lecture seule des paramètres (devise, période, catégorie,
  variante, rate_set, ruleset)
- Bouton "Lancer la consolidation"

### CRUD scénario

La page master data "scenarios" doit afficher les nouvelles colonnes avec des
dropdowns pour les FK (category, variant, rate_set, ruleset, entry_period,
presentation_currency).

---

## 10. Ordre d'implémentation recommandé

1. **Schema** : créer toutes les nouvelles tables + modifier les existantes
2. **seed.rs** : données de test cohérentes (pivot=EUR=présentation)
3. **ConvertParams + load_params** : chargement depuis la DB
4. **convert.rs** : cross-rate SQL
5. **`cargo test`** : vérifier que tout compile et que les tests passent
6. **server.rs** : POST /api/run avec scénario
7. **loader.rs + masterdata.rs** : nouveaux CSV et endpoints CRUD
8. **UI** : formulaire de run

L'étape 5 (cargo test) est le jalon de validation principal. Si les tests
golden passent à ce stade, la logique de cross-rate est correcte.

---

## 11. Non-objectifs

- Pas de migration de données (reset complet du schéma à chaque fois)
- Pas de multi-pivot (une seule devise pivot par instance)
- Pas de gestion d'historique des taux (une valeur par période)
- Pas de nouvelle logique de consolidation (les 4 étapes A/B/C/D sont
  inchangées dans leur algorithme — seul le SQL de conversion est étendu)

---

## 12. Décisions de design tranchées

| Question | Décision | Raison |
|----------|----------|--------|
| Où vit le pivot ? | `app_config` (singleton) | Une valeur pour toute l'instance, pas de redondance |
| Rate sets séparés du pivot ? | Oui, `dim_rate_set` | Permet taux réels vs budget ; le pivot est commun |
| `type` → `category` ? | Oui, + `dim_scenario_category` | Le champ libre devient une dimension référencée |
| prev_period stocké ou dérivé ? | Dérivé de `dim_period` | Évite la redondance, reste cohérent si les périodes changent |
| ruleset sur le scénario ? | Oui, nullable | Un scénario sans règles est légitime |
| `app_config` via masterdata CRUD ? | Non | Config système, pas editable comme une master data |
