# Tâche : Registre central des dimensions + dimensions custom

## Objectif
Remplacer les listes en dur (GROUP BY pipeline, whitelists rules.rs, PILOTABLE_DIMS)
par un registre central `src/dimensions.rs`. Permettre à l'utilisateur d'ajouter
des dimensions custom (toujours catégorie C — Analytical).

## Catégories (3 + méta)
```
Fixed       : scenario, entry_period, period, currency
              → propagated, pas pilotable, non-nullable, dans grain clôture
Active      : entity, account, flow, nature
              → propagated, pilotable, non-nullable, dans grain clôture
Analytical  : partner, share, analysis, analysis2 (+ customs)
              → propagated, pilotable, nullable, hors grain clôture
Meta        : level, amount
              → non-propagated (amount est une mesure)
```

Règles de dérivation :
- propagated = tout sauf Meta
- pilotable = Active + Analytical
- in_closure_grain = Fixed + Active
- nullable = Analytical uniquement (les customs sont nullable par définition)

---

## 1. Nouveau module `src/dimensions.rs`

```rust
use duckdb::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimCategory { Fixed, Active, Analytical }

#[derive(Debug, Clone)]
pub struct DimDef {
    pub name: String,
    pub category: DimCategory,
    pub custom: bool,       // true si ajoutée par l'utilisateur
    pub label: String,      // libellé UI
}

impl DimDef {
    pub fn propagated(&self) -> bool {
        !matches!(self.category, DimCategory::Analytical) || true // tout sauf Meta
        // En fait: propagated = category != Meta. Pas de Meta dans le registre.
    }
    pub fn pilotable(&self) -> bool {
        matches!(self.category, DimCategory::Active | DimCategory::Analytical)
    }
    pub fn nullable(&self) -> bool {
        matches!(self.category, DimCategory::Analytical)
    }
    pub fn in_closure_grain(&self) -> bool {
        matches!(self.category, DimCategory::Fixed | DimCategory::Active)
    }
}
```

IMPORTANT: le registre ne contient QUE les vraies dimensions (pas `level`, pas `amount`).
`level` et `amount` restent gérés séparément (level n'est pas dans le GROUP BY,
amount est agrégé par SUM).

### Liste built-in (const)
```rust
const BUILTIN_DIMS: &[DimDef] = &[
    // Fixed
    DimDef { name: "scenario",     category: Fixed,       custom: false, label: "Scénario" },
    DimDef { name: "entry_period", category: Fixed,       custom: false, label: "Exercice" },
    DimDef { name: "period",       category: Fixed,       custom: false, label: "Période" },
    DimDef { name: "currency",     category: Fixed,       custom: false, label: "Devise" },
    // Active
    DimDef { name: "entity",       category: Active,      custom: false, label: "Entité" },
    DimDef { name: "account",      category: Active,      custom: false, label: "Compte" },
    DimDef { name: "flow",         category: Active,      custom: false, label: "Flux" },
    DimDef { name: "nature",       category: Active,      custom: false, label: "Nature" },
    // Analytical
    DimDef { name: "partner",      category: Analytical,  custom: false, label: "Partenaire" },
    DimDef { name: "share",        category: Analytical,  custom: false, label: "Quote-part" },
    DimDef { name: "analysis",     category: Analytical,  custom: false, label: "Analyse 1" },
    DimDef { name: "analysis2",    category: Analytical,  custom: false, label: "Analyse 2" },
];
```

Note: les `const` avec des `String` ne compilent pas en Rust stable. Utiliser
soit `&'static str` dans la const avec une conversion, soit un fn qui retourne
`Vec<DimDef>`. Préférer la fonction :

```rust
pub fn builtin_dims() -> Vec<DimDef> { ... }
```

### Chargement runtime
```rust
/// Charge toutes les dimensions : built-in + custom (depuis dim_custom_dimension).
pub fn load_all(con: &Connection) -> Result<Vec<DimDef>, duckdb::Error> {
    // 1. Commencer par builtin_dims()
    // 2. SELECT name, label FROM dim_custom_dimension ORDER BY name
    // 3. Ajouter chaque custom comme DimDef { category: Analytical, custom: true, .. }
}

/// Retourne la liste des noms propagés (pour générer GROUP BY / SELECT).
pub fn propagated_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().map(|d| d.name.as_str()).collect()
    // Toutes les dims du registre sont propagated.
}

/// Retourne la liste des noms pilotables.
pub fn pilotable_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().filter(|d| d.pilotable()).map(|d| d.name.as_str()).collect()
}

/// Retourne les noms dans le grain des clôtures.
pub fn closure_grain_cols(dims: &[DimDef]) -> Vec<&str> {
    dims.iter().filter(|d| d.in_closure_grain()).map(|d| d.name.as_str()).collect()
}

/// Crée une dimension custom (ALTER TABLE + INSERT dans dim_custom_dimension).
pub fn create_custom(con: &Connection, name: &str, label: &str) -> Result<(), duckdb::Error> {
    // Valider le nom : alphanum + underscore, pas déjà dans builtin
    // ALTER TABLE fact_entry ADD COLUMN {name} TEXT;
    // ALTER TABLE stg_entry ADD COLUMN {name} TEXT;
    // INSERT INTO dim_custom_dimension (name, label) VALUES (?, ?);
}

/// Supprime une dimension custom.
pub fn delete_custom(con: &Connection, name: &str) -> Result<(), duckdb::Error> {
    // Vérifier qu'elle existe dans dim_custom_dimension
    // ALTER TABLE fact_entry DROP COLUMN {name};
    // ALTER TABLE stg_entry DROP COLUMN {name};
    // DELETE FROM dim_custom_dimension WHERE name = ?;
}
```

### Validation des noms custom
```rust
fn is_valid_custom_name(name: &str) -> bool {
    // Entre 1 et 50 caractères
    // Premier caractère : lettre ou underscore
    // Reste : alphanumérique + underscore
    // Pas dans les noms réservés : level, amount, id
    !name.is_empty()
        && name.len() <= 50
        && name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "level" | "amount" | "id")
}
```

---

## 2. Schema (`src/schema.rs`)

### Ajouter le DDL de la table registre
```rust
pub const DDL_DIM_CUSTOM_DIMENSION: &str = "\
CREATE TABLE dim_custom_dimension (\n\
    name  TEXT PRIMARY KEY,\n\
    label TEXT NOT NULL\n\
);";
```
Ajouter dans `ALL_DDL` AVANT `DDL_STG_ENTRY` et `DDL_FACT_ENTRY` (pour que la table
existe quand on fait les ALTER).

### `create_schema()` doit appeler `dimensions::apply_custom_columns()`
Après avoir exécuté tous les DDL, si `dim_custom_dimension` contient des lignes
(cas d'un reset où les customs survivent — ou pas, voir note), faire les ALTER TABLE.

NOTE: Au reset complet (DROP toutes les tables), les customs sont perdues.
C'est acceptable pour le prototype. La table `dim_custom_dimension` est aussi droppée.
Les customs sont recréées par l'utilisateur via l'UI.

En fait NON — si on DROP dim_custom_dimension, on perd le registre. Solution:
- `ALL_DROP` ne doit PAS dropper `dim_custom_dimension`.
- Au reset, on DROP tout SAUF dim_custom_dimension, puis on recrée le schéma,
  puis on lit dim_custom_dimension et on fait les ALTER TABLE ADD COLUMN.

Implémentation: modifier `ALL_DROP` pour exclure `dim_custom_dimension`.
Ou: dans `create_schema()`, lire les customs AVANT le DROP, les mémoriser,
recréer le schéma, puis les réappliquer. Plus simple :

```rust
pub fn create_schema(con: &Connection) -> duckdb::Result<()> {
    // 1. Sauvegarder les customs existantes (si la table existe)
    let saved_customs = match dimensions::load_customs(con) {
        Ok(c) => c,
        Err(_) => Vec::new(), // la table n'existe pas encore
    };
    // 2. DROP (ne pas dropper dim_custom_dimension)
    // 3. CREATE (toutes les tables sauf dim_custom_dimension si elle existe déjà)
    // 4. Réappliquer les customs : pour chaque saved_custom, ALTER TABLE ADD COLUMN + re-INSERT
    // 5. Ok
}
```

---

## 3. Pipeline — génération dynamique des GROUP BY / SELECT

### Modules concernés
- `src/pipeline/aggregate.rs` (étape A)
- `src/pipeline/reclassify.rs` (étape B — staging 2)
- `src/pipeline/staging.rs` (staging 1)
- `src/pipeline/convert.rs` (étape C — mais attention, pas de GROUP BY, c'est un INSERT...SELECT)
- `src/pipeline/consolidate.rs` (étape D — pas de GROUP BY non plus)
- `src/pipeline/materialize_closures.rs` (GROUP BY spécifique, grain clôture)

### Pattern pour les modules A/B/Staging (qui ont un GROUP BY)

Remplacer le SQL statique par une fonction qui génère le SQL :

```rust
use crate::dimensions;

pub fn step_a(con: &Connection) -> duckdb::Result<usize> {
    let dims = dimensions::load_all(con)?;
    let cols = dimensions::propagated_cols(&dims);
    let col_list = cols.join(", ");
    let group_by = cols.join(", ");

    let sql = format!(
        "INSERT INTO fact_entry\n\
         ({col_list}, level, amount)\n\
         SELECT\n\
         {col_list},\n\
         'corporate' AS level,\n\
         CAST(SUM(amount) AS DECIMAL(18,2)) AS amount\n\
         FROM stg_entry\n\
         WHERE substr(nature, 1, 1) IN ('0', '1')\n\
         GROUP BY {group_by};",
        col_list = col_list,
        group_by = group_by,
    );
    con.execute(&sql, [])?;
    count_level(con, "corporate")
}
```

IMPORTANT: Le SQL généré doit être IDENTIQUE au SQL actuel pour les 12 colonnes
builtin. Le test golden doit toujours passer à 28/28 sans modification.

### Étape C (convert.rs) et D (consolidate.rs)
Pas de GROUP BY, mais le SELECT liste toutes les colonnes. Rendre dynamique :
remplacer la liste en dur par `{col_list}` générée depuis le registre.

### Étape B (reclassify.rs)
Même pattern : générer le GROUP BY dynamiquement.

### materialize_closures.rs
Le grain des clôtures est différent (Fixed + Active seulement, pas Analytical).
Utiliser `dimensions::closure_grain_cols(&dims)`.

---

## 4. Module rules.rs — whitelists dynamiques

Remplacer les `const` statiques par des valeurs lues depuis le registre.

### Fonctions de validation
Les fonctions `parse_selection_cond` et `parse_destination` prennent actuellement
les whitelists depuis des `const`. Modifier pour accepter une `&[DimDef]` en
paramètre (ou un struct de contexte).

Pattern recommandé : créer un struct de contexte passé à `run_ruleset` :

```rust
pub struct RuleContext {
    pub selection_dims: Vec<String>,
    pub pilotable_dims: Vec<String>,
}
```

Dans `run_ruleset`, charger le registre :
```rust
let dims = dimensions::load_all(con)?;
let ctx = RuleContext {
    selection_dims: dims.iter().map(|d| d.name.clone()).collect(),
    pilotable_dims: dimensions::pilotable_cols(&dims).into_iter().map(String::from).collect(),
};
```

Puis passer `&ctx` aux fonctions de parsing.

IMPORTANT: La validation de sécurité (whitelist) doit TOUJOURS avoir lieu.
La liste est dynamique mais le principe reste : tout identifiant dans
selection.dim, destination.key, scope.dim doit être dans la whitelist.
C'est critique pour éviter l'injection SQL.

Le nom des colonnes custom vient de `dim_custom_dimension` (contrôlé par l'API),
pas du JSON de la règle (qui vient de l'utilisateur). Donc pas de risque
d'injection via les noms de colonnes — ils sont validés à la création.

### Coefficient expr (pas de changement)
`pct_integration` et `pct_interet` sont des colonnes de `sat_perimeter`, pas des
dimensions. Pas de changement.

---

## 5. API serveur — nouveaux endpoints

### `GET /api/meta/dimensions`
Retourne toutes les dimensions (built-in + custom) :
```json
[
  {"name": "scenario", "category": "Fixed", "custom": false, "label": "Scénario", "pilotable": false},
  {"name": "entity", "category": "Active", "custom": false, "label": "Entité", "pilotable": true},
  {"name": "partner", "category": "Analytical", "custom": false, "label": "Partenaire", "pilotable": true},
  {"name": "segment", "category": "Analytical", "custom": true, "label": "Segment", "pilotable": true}
]
```

Handler :
```rust
async fn list_dimensions(State(state): State<Arc<AppState>>) -> Result<Json<Vec<DimensionInfo>>, AppError> {
    let con = state.db.lock().unwrap();
    let dims = dimensions::load_all(&con).map_err(...)?;
    Ok(Json(dims.iter().map(|d| DimensionInfo {
        name: d.name.clone(),
        category: format!("{:?}", d.category),
        custom: d.custom,
        label: d.label.clone(),
        pilotable: d.pilotable(),
    }).collect()))
}
```

### `POST /api/meta/dimensions`
Body: `{ "name": "segment", "label": "Segment produit" }`
- Valide le nom (is_valid_custom_name)
- Appelle `dimensions::create_custom`
- Retourne 201 + la dimension créée

### `DELETE /api/meta/dimensions/{name}`
- Appelle `dimensions::delete_custom`
- Retourne 200 `{ "deleted": 1 }`

### Routes à ajouter dans le Router
```rust
.route("/api/meta/dimensions", get(list_dimensions).post(create_dimension))
.route("/api/meta/dimensions/{name}", delete(delete_dimension))
```

---

## 6. Frontend

### `src/api.ts`
Ajouter :
```typescript
dimensions: {
  list: () => getJson<DimensionInfo[]>('/meta/dimensions'),
  create: (body: { name: string; label: string }) =>
    postJsonRaw<DimensionInfo>('/meta/dimensions', body),
  remove: (name: string) => deleteJson<{ deleted: number }>(`/meta/dimensions/${name}`),
},
```

### `src/types.ts`
```typescript
export interface DimensionInfo {
  name: string;
  category: 'Fixed' | 'Active' | 'Analytical';
  custom: boolean;
  label: string;
  pilotable: boolean;
}
```

### `src/pages/RulesPage.tsx`
1. **Charger les dimensions au montage de la page** via `api.dimensions.list()`
2. Remplacer `PILOTABLE_DIMS` (const hardcodée) par les dims pilotables de l'API
3. Remplacer `SELECTION_DIMS` par toutes les dims de l'API
4. Dans le formulaire de règle, les destinations proposent maintenant TOUTES les
   dims pilotables (built-in actives + analytiques + customs)
5. Ajouter un **3ème sous-onglet « Dimensions »** dans RulesPage :
   - Tableau des dimensions existantes (nom, catégorie, personnalisée?, libellé)
   - Bouton « Ajouter une dimension » → petit formulaire (nom technique, libellé)
   - Bouton supprimer sur les customs uniquement (les builtin sont verrouillées)

### `src/components/Layout.tsx`
Pas de changement (l'onglet « Règles » existe déjà).

---

## 7. Ordre d'implémentation

1. `src/dimensions.rs` (registre + fonctions)
2. `src/schema.rs` (DDL dim_custom_dimension + create_schema modifié)
3. `src/lib.rs` (pub mod dimensions)
4. Pipeline : aggregate.rs, reclassify.rs, staging.rs, convert.rs, consolidate.rs, materialize_closures.rs
5. `src/rules.rs` (whitelists dynamiques)
6. `src/bin/server.rs` (routes /api/meta/dimensions)
7. Frontend : api.ts, types.ts, RulesPage.tsx
8. `cargo build --release` + tests Python

## 8. Vérification finale

```bash
cd ~/cf-clone/prototype/rust
cargo build --release --bin conso-server
python3 golden_test.py   # 28/28
python3 rules_test.py    # 33/33
python3 smoke_test.py    # 59/59
cd ~/cf-clone/web
npm run build            # 0 erreur
```

Le test golden doit passer SANS modification — le SQL généré pour 12 colonnes
builtin doit être identique à ce qui existe aujourd'hui.
