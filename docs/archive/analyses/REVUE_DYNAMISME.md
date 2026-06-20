# Revue : Rendre le code plus dynamique et évolutif

*19 juin 2026 — Revue post-registre des dimensions*

## Contexte

Le registre des dimensions (commit `6af74d2`) a résolu le problème central :
ajouter une dimension ne nécessite plus de toucher 5 modules pipeline. Mais le
code reste rigide sur d'autres axes. Cette revue identifie les points de
rigidité restants, les hiérarchise, et propose des specs pour les plus
crédibles.

**État des tests actuel :** cargo 16/16, golden 28/28, rules 33/33, smoke 59/59.

---

## Cartographie des rigidités restantes

| # | Zone | Problème | Impact | Effort | Crédible ? |
|---|------|----------|--------|--------|------------|
| P1 | Pipeline `mod.rs` | `run_pipeline` + `run_pipeline_timed` = duplication intégrale | Élevé | Moyen | ✅ Oui |
| P2 | Pipeline `mod.rs` | Étapes A→D en séquence codée en dur ; ajouter une étape = modifier l'orchestration | Élevé | Moyen | ✅ Oui |
| P3 | `ConvertParams` | `EUR/2024/2023` hardcoded dans `Default` ; l'API `/api/run` l'utilise toujours | Critique | Faible | ✅ Oui |
| P4 | `loader.rs` | 11 INSERT en dur, mapping fichier→table implicite | Moyen | Faible | ✅ Oui |
| P5 | `import.rs` | Colonnes required hardcoded ; ignore les dimensions custom | Moyen | Faible | ✅ Oui |
| P6 | `report.rs` | `FLOW_ORDER` hardcoded au lieu de lire `dim_flow` | Faible | Très faible | ✅ Oui |
| P7 | `consolidate.rs` | `methode IN ('globale', 'proportionnelle')` en dur dans le SQL | Faible | Faible | ⚠️ Peut-être |
| P8 | `masterdata.rs` | `TABLES` = const statique ; ajouter une master data = modifier le code | Moyen | Moyen | ⚠️ Peut-être |
| P9 | `rules.rs` | `ALLOWED_SCOPE_DIMS` hardcoded (colonnes `sat_perimeter`) | Faible | Très faible | ✅ Oui |
| P10 | Staging `mod.rs` | Mapping préfixe→niveau (`2→reclassified`, etc.) inline dans `run_pipeline` | Faible | Faible | ⚠️ Peut-être |

---

## Propositions détaillées

### P1+P2 — Pipeline déclaratif (fusion : trait `Step`)

**Problème :**
- `run_pipeline` et `run_pipeline_timed` sont des copier-coller à 90%. Le
  second ajoute juste des `Instant::now()` autour de chaque bloc.
- L'ordre A→B→C→D + staging + materialize_closures est codé en dur dans les
  deux fonctions. Insérer une étape = modifier les deux.
- Chaque étape fait en réalité 3 choses : sa transformation, l'injection
  staging (`inject_by_prefix`), et la reconstruction des clôtures
  (`materialize_closures`). Ce motif est répété 3 fois (B, C, D).

**Proposition :**

```rust
pub trait Step: Send + Sync {
    /// Nom court pour les logs / rapports ("agrégation", "conversion"…).
    fn name(&self) -> &str;
    /// Niveau lu en entrée (ex: "corporate"). None pour la première étape (lit stg_entry).
    fn input_level(&self) -> Option<&str>;
    /// Niveau produit en sortie ("corporate", "reclassified"…).
    fn output_level(&self) -> &str;
    /// Préfixe de staging injecté après cette étape ("2", "3", "4", ou "" si aucun).
    fn staging_prefix(&self) -> &str { "" }
    /// Exécute la transformation principale (sans staging ni clôtures).
    fn run(&self, con: &Connection, params: &ConvertParams) -> duckdb::Result<usize>;
}
```

Le pipeline devient :

```rust
pub fn run_pipeline(con: &Connection, params: &ConvertParams) -> Result<PipelineReport> {
    let steps: Vec<Box<dyn Step>> = vec![
        Box::new(AggregateStep),
        Box::new(ReclassifyStep),      // staging "2", clôtures
        Box::new(ConvertStep),         // staging "3", clôtures
        Box::new(ConsolidateStep),     // staging "4", clôtures
    ];
    run_steps(con, params, &steps)
}
```

`run_steps` orchestre : pour chaque étape → `step.run()` → `inject_by_prefix`
(si prefix non vide) → `materialize_closures` → mesurer le temps → compter.

**Bénéfices :**
- Un seul code d'orchestration (plus de duplication)
- `run_pipeline_timed` disparaît (toujours instrumenté)
- Ajouter une étape = implémenter `Step` + une ligne dans le `vec!`
- Le moteur de règles peut devenir un `Step` insérable entre C et D

**Effort estimé :** ~200 lignes refactorées, 0 nouvelle logique métier.
**Risque :** faible — les SQL générés ne changent pas, seul l'enchaînement est
restructuré.

---

### P3 — `ConvertParams` depuis les master data

**Problème :**

```rust
impl Default for ConvertParams {
    fn default() -> Self {
        Self {
            presentation_currency: "EUR".to_string(),
            current_period: "2024".to_string(),
            prev_period: "2023".to_string(),
        }
    }
}
```

Le serveur utilise **toujours** `ConvertParams::default()` :
```rust
// server.rs ligne 426
let params = ConvertParams::default();
```

Donc en production : la devise de présentation, l'exercice N et N-1 sont
hardcodés. Impossible de consolider en USD ou sur l'exercice 2025 sans
recompiler.

**Proposition :**

Lecture depuis `dim_scenario` + `dim_period` :

```sql
-- Devise de présentation : depuis dim_scenario ou une colonne dédiée
SELECT presentation_currency FROM dim_scenario WHERE code = ?;

-- Exercice courant et précédent : depuis dim_period
-- Ou plus simple : deux colonnes dans dim_scenario (current_period, prev_period)
```

Option A (la plus simple) : ajouter `presentation_currency` et `current_period`
à `dim_scenario`, dériver `prev_period` par requête sur `dim_period`.

Option B : accepter ces paramètres dans le body de `POST /api/run`.

**Mon avis :** Option A (donnée, pas code). `POST /api/run` peut optionnellement
override avec un body, mais par défaut lit le scénario.

**Effort :** faible — 1 ALTER TABLE + 1 fonction de chargement + 3 lignes
modifiées dans `server.rs`.

---

### P4 — Loader générique par registre

**Problème :**

`loader.rs` contient 11 blocs `INSERT INTO ... SELECT ... FROM read_csv_auto()`
codés en dur. Le mapping fichier → table → colonnes → casts est implicite.
Ajouter une master data ou changer un nom de fichier = modifier le code.

**Proposition :**

Un registre de chargement, similaire à `TABLES` dans `masterdata.rs` :

```rust
struct CsvMapping {
    file: &'static str,        // "scenarios.csv"
    table: &'static str,       // "dim_scenario"
    columns: &'static [&'str], // ["code", "libelle", "type", "statut"]
    casts: &'static [(&'static str, &'static str)], // [("decimales", "INTEGER")]
}
```

`load_all` itère sur le registre. Ajouter un CSV = une ligne dans le registre.

**Effort :** faible — pure refactorisation, 0 logique nouvelle.

---

### P5 — Import CSV adaptatif (dimensions custom)

**Problème :**

`import.rs::import_entries` hardcode les colonnes required et l'INSERT :

```rust
let required = &["scenario", "entity", ..., "analysis2", "amount"];
let sql = format!(
    "INSERT INTO stg_entry (scenario, entity, ..., analysis2, amount)
     SELECT scenario, entity, ..., {partner}, {share}, {analysis}, analysis2, amount
     FROM read_csv_auto('{path}', header=true)"
);
```

Si l'utilisateur ajoute une dimension custom `segment`, le CSV d'import avec une
colonne `segment` sera **ignoré** silencieusement. Le pipeline ne la propagera
pas (NULL) alors qu'elle existe dans le schéma.

**Proposition :**

1. Charger la liste des colonnes depuis `dimensions::load_all()` + les colonnes
   fixes (`level`, `amount`)
2. Générer l'INSERT dynamiquement avec toutes les colonnes connues
3. Les colonnes absentes du CSV → DuckDB `read_csv_auto` les met à NULL
   automatiquement (avec `null_padding=true` ou en spécifiant les columns)
4. Les colonnes présentes dans le CSV mais inconnues → ignorées (DuckDB
   `read_csv_auto` ne se plaint pas)

**Subtilité :** `read_csv_auto` infère le schéma depuis le fichier, pas depuis
la table cible. Pour les colonnes custom, il faut soit:
- Utiliser `read_csv(..., columns={...})` avec toutes les colonnes en VARCHAR
- Ou faire un `CREATE TEMP TABLE` avec le bon schéma puis `COPY FROM`

**Effort :** moyen — attention à l'inférence de types DuckDB.

---

### P6 — `FLOW_ORDER` depuis `dim_flow`

**Problème :**

```rust
pub const FLOW_ORDER: &[&str] = &["F00", "F01", "F20", "F80", "F81", "F98", "F99"];
```

Hardcoded. Si on ajoute un flux F30 (variation de capital), il n'apparaîtra
pas dans les rapports.

**Proposition :**

```rust
pub fn flow_order(con: &Connection) -> Vec<String> {
    con.query("SELECT code FROM dim_flow ORDER BY code", [])
}
```

**Effort :** trivial. 5 lignes.

---

### P9 — `ALLOWED_SCOPE_DIMS` dynamique

**Problème :**

```rust
const ALLOWED_SCOPE_DIMS: &[&str] = &[
    "methode", "pct_interet", "pct_integration", "entree", "sortie",
];
```

Hardcoded. Si on ajoute une colonne à `sat_perimeter`, il faut modifier
`rules.rs`.

**Proposition :**

Lire les colonnes depuis `information_schema.columns WHERE table_name = 'sat_perimeter'`
au runtime, dans `RuleContext::from_registry()`.

**Effort :** trivial. 3 lignes.

---

## Propositions secondaires (moins prioritaires)

### P7 — Méthodes de consolidation configurables

Le filtre `methode IN ('globale', 'proportionnelle')` est dans le SQL de
`consolidate.rs`. La mise en équivalence est exclue ("hors MVP").

Si on voulait l'ajouter, il faudrait modifier le SQL. Une approche :
une table `dim_method` avec un flag `consolidated` (oui/non), et le filtre
devient `WHERE m.consolidated = TRUE`.

**Mon avis :** pas urgent. La liste des méthodes est stable et courte.
À garder pour quand la mise en équivalence deviendra réelle.

### P8 — `TABLES` dynamique dans `masterdata.rs`

Le CRUD des master data est piloté par une `const TABLES`. Ajouter une table
 nécessite de modifier le code Rust.

**Mon avis :** faible valeur. Les master data évoluent rarement, et la
validation statique (`reject_unknown_fields`) est une protection. Pas la
peine de dynamiser pour 10 tables fixes.

### P10 — Mapping préfixe→niveau configurable

Le mapping `'2' → reclassified`, `'3' → converted`, `'4' → consolidated`
est inline dans `run_pipeline`. Serait élégant à mettre dans une table ou un
registre.

**Mon avis :** se résout naturellement avec P1+P2 (le `staging_prefix()` sur
le trait `Step`). Pas besoin de spec séparée.

---

## Plan recommandé (priorité décroissante)

| Priorité | Proposition | Effort | Bénéfice |
|----------|------------|--------|----------|
| 🔴 1 | **P3** — ConvertParams depuis master data | Faible | Critique : débloque l'usage réel |
| 🟠 2 | **P1+P2** — Pipeline déclaratif (trait Step) | Moyen | Élimine la duplication, ouvre l'extensibilité |
| 🟡 3 | **P5** — Import adaptatif dimensions custom | Moyen | Sans ça, les customs sont inutilisables en pratique |
| 🟡 4 | **P6** — FLOW_ORDER depuis dim_flow | Très faible | Propreté immédiate |
| 🟢 5 | **P9** — ALLOWED_SCOPE_DIMS dynamique | Très faible | Propreté immédiate |
| 🟢 6 | **P4** — Loader générique | Faible | Propreté, faible valeur métier |

**P3 est le plus urgent** : sans lui, l'outil ne peut pas consolider un autre
scénario que EUR/2024/2023. C'est un bug de production, pas juste une rigidité.

---

## Décisions à prendre

1. **P3** : Option A (dans `dim_scenario`) ou Option B (body de `/api/run`) ?
2. **P1+P2** : On fusionne dans un seul chantier ou on laisse `run_pipeline` tel quel ?
3. **P5** : Priorité sur les autres ? Sans ça, les dimensions custom ajoutées via
   l'UI ne peuvent pas être alimentées par import CSV.
4. **P6 + P9** : On les fait d'un coup (5 minutes de code) ?
5. **P7/P8/P10** : On laisse tomber pour l'instant ?
