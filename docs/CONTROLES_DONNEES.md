# Contrôles de données — Spécification et plan d'implémentation

> Module de **contrôles de données** : vérifications configurables (arithmétiques, variations, complétude) exécutées à la demande sur les données staging et/ou consolidées. Cohérent avec le moteur de formules existant (`formula.rs`, coefficients, postes, indicateurs).

---

## 1. Objectif

Permettre à l'utilisateur de **définir des contrôles** sur les données saisies et/ou consolidées pour détecter :

- **Absence de données** (entité n'ayant pas saisi un compte attendu)
- **Incohérences arithmétiques** (éliminations IC non soldées, balances débit/crédit)
- **Variations anormales** (écarts % entre N et N-1 dépassant un seuil)
- **Complétude** (toutes les entités ont-elles remonté leurs liasses ?)

Chaque contrôle s'exécute **à la demande** (bouton), sur **un ou plusieurs niveaux** (raw, corporate, converted, consolidated) et produit un **rapport** différenciant pass / warn / error / no_data, avec détail des lignes en cause.

---

## 2. Modèle d'un contrôle

```
CONTRÔLE
├── Identité : code, libellé
├── Niveaux cibles : 1 à 3 parmi (raw, corporate, converted, consolidated)
├── Grain : dimensions de regroupement (entity, partner, account, ...)
├── Sélection : filtres sur fact_entry / stg_entry (comme postes)
├── Expression : formule optionnelle (par défaut SUM(amount))
├── Assertions : 1 à N règles de validation
│   ├── range      → |valeur| <= error, |valeur| <= warn
│   ├── nonzero    → valeur ≠ 0
│   ├── existence  → au moins 1 ligne par grain
│   └── equals     → valeur = cible
└── Comparaison inter-périodes (optionnel)
    ├── type : variation_abs | variation_pct | variation
    ├── baseline : consolidation N-1 (ou exercice cible)
    ├── warn : seuil warning
    └── error : seuil erreur
```

### 2.1 Grain

Le grain définit le niveau de détail du contrôle. Le moteur agrège les données `GROUP BY <grain>` et évalue l'assertion **pour chaque combinaison de valeurs du grain**.

Exemples :
- `["entity"]` → contrôle par entité
- `["entity", "partner"]` → contrôle par paire entité/partenaire (éliminations IC)
- `[]` (vide) → contrôle sur le total (ex : le bilan est-il équilibré ?)

### 2.2 Sélection

Même modèle que les postes (`SelectionCond`) : filtres sur les dimensions avec opérateurs `=`, `!=`, `IN`, `IS NULL`, `IS NOT NULL`, et traversées `via` / `ref` / `attr`.

Pour `level = "raw"`, les filtres portent sur les colonnes TEXT de `stg_entry` (codes). Pour les autres niveaux, les filtres portent sur les colonnes INTEGER résolues de `fact_entry`.

### 2.3 Expression

Formule optionnelle utilisant le moteur `formula.rs`. Si omise, le moteur calcule `SUM(e.amount)` sur la sélection.

L'expression peut référencer :
- Des **postes** (`[code_poste]`) — résolus via `dim_aggregate`
- Des **indicateurs** (`[code_indicateur]`) — résolus via `dim_indicator`
- Des **fonctions** : `ABS`, `MIN`, `MAX`, `SAFE_DIV`, `IF`, `ROUND`
- Des **littéraux** numériques et opérateurs arithmétiques

### 2.4 Assertions

Chaque contrôle peut avoir **plusieurs assertions**. Si au moins une assertion est en `error`, le contrôle est en `error`. Sinon, si au moins une est en `warn`, le contrôle est en `warn`. Sinon, `pass`.

| Type | Signification | Paramètres |
|------|--------------|------------|
| `range` | La valeur absolue doit être ≤ au seuil | `warn: number`, `error: number` |
| `nonzero` | La valeur agrégée doit être ≠ 0 | (aucun) |
| `existence` | Au moins une ligne doit exister pour chaque grain | (aucun) |
| `equals` | La valeur doit être égale à une cible | `target: number` |

### 2.5 Comparaison consolidations

Optionnelle. Permet de comparer les données d'une consolidation N avec une baseline (N-1 ou autre consolidation).

| Métrique | Calcul |
|----------|--------|
| `variation_abs` | `\|N - N-1\|` |
| `variation_pct` | `\|(N - N-1) / N-1\| * 100` |
| `variation` | `N - N-1` (signée) |

La comparaison se fait **par niveau** : on compare `corporate N` vs `corporate N-1`, jamais de croisement entre niveaux.

Si N-1 n'a pas de valeur pour un grain donné → le résultat est `no_data` pour ce grain.

---

## 3. Niveaux multi-valués

Un contrôle peut cibler **plusieurs niveaux** simultanément. Le moteur exécute le contrôle **une fois par niveau** et produit un rapport séparé par niveau.

Le niveau `raw` cible les données brutes saisies (`stg_entry`). Les niveaux `corporate`, `converted`, `consolidated` ciblent les données traitées (`fact_entry`).

```json
{
  "levels": ["raw", "corporate", "converted", "consolidated"]
}
```

Cela évite de dupliquer un contrôle qui s'applique à tous les niveaux. Les sélections et assertions sont identiques pour chaque niveau — seule la table source et la résolution des codes diffèrent.

---

## 4. Jeux de contrôles

Comme les règles s'assemblent en règles de consolidation, les contrôles s'assemblent en **jeux de contrôles** :

```
JEU DE CONTRÔLES
├── Code, libellé
└── Contrôles ordonnés : référence à N contrôles (avec ordre)
```

L'exécution d'un jeu exécute tous ses contrôles séquentiellement et agrège les résultats dans un rapport unique.

---

## 5. Modèle de données

### 5.1 Tables

```sql
-- Bibliothèque de contrôles (survit au reset)
CREATE TABLE IF NOT EXISTS dim_control (
    code       TEXT PRIMARY KEY,
    libelle    TEXT,
    definition TEXT NOT NULL    -- JSON (cf. §5.2)
);

-- Jeux de contrôles (survit au reset)
CREATE TABLE IF NOT EXISTS dim_control_set (
    code    TEXT PRIMARY KEY,
    libelle TEXT
);

-- Items d'un jeu de contrôles (survit au reset)
CREATE TABLE IF NOT EXISTS dim_control_set_item (
    set_code     TEXT,
    control_code TEXT,
    ord          INTEGER,
    PRIMARY KEY (set_code, control_code)
);

-- Résultats d'exécution (écrasés à chaque run)
CREATE TABLE IF NOT EXISTS fact_control_result (
    control_code TEXT NOT NULL,
    level        TEXT NOT NULL,
    grain_values TEXT NOT NULL,       -- JSON {"entity":"ENT1","partner":"ENT2"}
    value        DECIMAL(18,2),
    baseline     DECIMAL(18,2),
    variation    DECIMAL(18,2),
    status       TEXT NOT NULL,       -- 'pass' | 'warn' | 'error' | 'no_data'
    row_count    INTEGER,
    sample_ids   TEXT                 -- JSON array des premiers fact_entry.id
);
```

### 5.2 JSON `definition`

```json
{
  "levels": ["raw", "corporate", "consolidated"],
  "grain": ["entity", "partner"],
  "selection": [
    { "dim": "account", "op": "IN", "val": ["600", "700"] },
    { "dim": "flow", "op": "=", "val": "F99" }
  ],
  "expression": null,
  "assertions": [
    { "type": "range", "warn": 100, "error": 1000 },
    { "type": "nonzero" }
  ],
  "compare": {
    "metric": "variation_pct",
    "baseline_consolidation_id": null,
    "warn": 10,
    "error": 50
  }
}
```

Champs :
- `levels` : array de 1 à 4 valeurs parmi `["raw", "corporate", "converted", "consolidated"]`
  - `raw` → cible `stg_entry` (données brutes saisies, colonnes TEXT)
  - `corporate` / `converted` / `consolidated` → cible `fact_entry` (données traitées, colonnes INTEGER résolues)
- `grain` : array de noms de dimensions propagées (vide = total)
- `selection` : array de `SelectionCond` (même modèle que postes/règles). Pour `raw`, les valeurs sont des codes TEXT. Pour les autres niveaux, ce sont des codes résolus en IDs par le moteur.
- `expression` : string nullable (formule `formula.rs`) — si null → `SUM(e.amount)`
- `assertions` : array d'assertions (au moins 1)
- `compare` : objet nullable (comparaison inter-périodes) — si null → pas de comparaison
  - `metric` : `"variation_abs"` | `"variation_pct"` | `"variation"`
  - `baseline_consolidation_id` : integer nullable — si null, le moteur déduit la même phase N-1 automatiquement
  - La comparaison ne s'applique pas aux contrôles de niveau `raw` (pas de notion de consolidation N vs N-1 sur le staging)

### 5.3 Schéma DuckDB

Les tables `dim_control`, `dim_control_set`, `dim_control_set_item` sont ajoutées au `ALL_DDL` de `schema.rs` mais **en dehors de `ALL_DROP`** (survivent au reset, comme `dim_coefficient`).

La table `fact_control_result` est dans `ALL_DROP` (résultats volatiles, régénérés à chaque exécution).

---

## 6. API REST

### 6.1 Contrôles

| Méthode | Route | Handler | Description |
|---------|-------|---------|-------------|
| `GET` | `/api/controls` | `list` | Liste tous les contrôles |
| `POST` | `/api/controls` | `create` | Créer un contrôle (validation JSON) |
| `GET` | `/api/controls/{code}` | `get` | Détail d'un contrôle |
| `PUT` | `/api/controls/{code}` | `update` | Modifier un contrôle |
| `DELETE` | `/api/controls/{code}` | `delete_ctrl` | Supprimer un contrôle |
| `POST` | `/api/controls/{code}/run` | `run_single` | Exécuter un contrôle isolé (body: `{ consolidation_id?, phase?, entry_period? }`) |
| `GET` | `/api/controls/operands` | `operands` | Catalogue d'opérandes (postes + indicateurs) |

### 6.2 Jeux de contrôles

| Méthode | Route | Handler | Description |
|---------|-------|---------|-------------|
| `GET` | `/api/control-sets` | `list_sets` | Liste les jeux |
| `POST` | `/api/control-sets` | `create_set` | Créer un jeu |
| `GET` | `/api/control-sets/{code}` | `get_set` | Détail d'un jeu (avec contrôles) |
| `PUT` | `/api/control-sets/{code}` | `update_set` | Modifier un jeu |
| `DELETE` | `/api/control-sets/{code}` | `delete_set` | Supprimer un jeu |
| `POST` | `/api/control-sets/{code}/run` | `run_set` | Exécuter un jeu → rapport (body: `{ consolidation_id?, phase?, entry_period? }`) |
| `GET` | `/api/control-sets/{code}/results` | `get_results` | Derniers résultats d'un jeu |

### 6.3 Réponses

**POST .../run** → rapport :
```json
{
  "set_code": "CTRL_PRE_CONSO",
  "executed_at": "2026-06-27T10:30:00Z",
  "consolidation_id": 1,
  "phase": "REEL",
  "entry_period": "2026-12",
  "summary": {
    "total": 36,
    "by_level": {
      "raw":          { "pass": 5, "warn": 0, "error": 0, "no_data": 0 },
      "corporate":    { "pass": 10, "warn": 1, "error": 0, "no_data": 1 },
      "consolidated": { "pass": 9, "warn": 1, "error": 1, "no_data": 1 }
    }
  },
  "details": [
    {
      "control_code": "CTRL_IC_SOLD",
      "control_libelle": "Élimination IC soldée",
      "levels": {
        "raw": {
          "status": "pass",
          "rows": []
        },
        "consolidated": {
          "status": "error",
          "rows": [
            { "grain": {"entity":"ENT1","partner":"ENT2"}, "value": -15230.00, "baseline": null, "variation": null, "status": "error", "row_count": 3 }
          ]
        }
      }
    }
  ]
}
```

---

## 7. UI — Page Contrôles

### 7.1 Emplacement dans la navigation

Nouvel onglet **"Contrôles"** sous "Calculs" dans la sidebar, à côté de "Règles", "Jeux de règles", etc.

### 7.2 Layout

```
┌─────────────────────────────────────────────────────────────────┐
│  Contrôles de données                                          │
├──────────────┬──────────────────────────────────────────────────┤
│  Bibliothèque│  Éditeur de contrôle                            │
│  ────────────│  ┌────────────────────────────────────────────┐ │
│  CTRL_IC_SOLD│  │ Code: CTRL_IC_SOLD                          │ │
│  CTRL_CA     │  │ Libellé: Élimination IC soldée              │ │
│  CTRL_VAR    │  │ Niveaux: ☑ raw ☑ corp ☑ conv ☑ cons              │ │
│              │  │ Grain: [entity] [partner]  [+ Ajouter]      │ │
│              │  │ Sélection:                                   │ │
│              │  │   compte IN 600,700  │ flow = F99           │ │
│              │  │ Expression: (optionnel) [____________]       │ │
│              │  │ Assertions:                                  │ │
│              │  │   ☑ range warn:100 error:1000               │ │
│              │  │   ☑ nonzero                                  │ │
│              │  │ Comparaison:                                 │ │
│              │  │   ☑ variation_pct warn:10% error:50%        │ │
│              │  └────────────────────────────────────────────┘ │
├──────────────┴──────────────────────────────────────────────────┤
│  Jeux de contrôles                                             │
│  ─────────────────                                             │
│  CTRL_PRE_CONSO: [CTRL_CA, CTRL_BILAN_EQ, ...]  [Exécuter ▶]  │
├────────────────────────────────────────────────────────────────-┤
│  Rapport d'exécution (après exécution)                         │
│  ┌─────┬───────┬─────┬───────┬─────────┐                      │
│  │Code │Niveau │Statut│Valeur │Grain    │                      │
│  ├─────┼───────┼─────┼───────┼─────────┤                      │
│  │IC   │consol │ ❌  │-15230 │ENT1/ENT2│                      │
│  │CA   │corp   │ ✅  │   --  │   --    │                      │
│  └─────┴───────┴─────┴───────┴─────────┘                      │
└────────────────────────────────────────────────────────────────┘
```

### 7.3 Composants réutilisés

| Composant existant | Usage dans Contrôles |
|---|---|
| `ConditionFields` | Éditeur de sélection (filtres dimensions) |
| `FormulaEditor` | Éditeur de formule avec autocomplete `[` |
| `OperandPalette` | Palette d'opérandes (postes + indicateurs) |
| `useDimValues` hook | Chargement des valeurs de dimensions pour les dropdowns |
| `subtabs` CSS | (non utilisé — pas de sous-onglets internes) |

---

## 8. Exécution — Moteur Rust

### 8.1 Module `controls.rs`

Nouveau fichier `prototype/rust/src/controls.rs` (~400-500 lignes estimées).

#### Structures

```rust
struct ControlDefinition {
    levels: Vec<String>,              // ["raw", "corporate", "converted", "consolidated"]
    grain: Vec<String>,               // ["entity", "partner"]
    selection: Vec<SelectionCond>,    // réutilise rules::SelectionCond
    expression: Option<String>,       // formule formula.rs
    assertions: Vec<Assertion>,
    compare: Option<Compare>,
}

enum Assertion {
    Range { warn: f64, error: f64 },
    Nonzero,
    Existence,
    Equals { target: f64 },
}

struct Compare {
    metric: String,                        // "variation_abs" | "variation_pct" | "variation"
    baseline_consolidation_id: Option<i64>, // None → déduit automatiquement N-1
    warn: f64,
    error: f64,
}
```

#### Validation (`validate_definition`)

À la création/modification d'un contrôle :
- Vérifier que `levels` contient au moins 1 valeur parmi `["raw", "corporate", "converted", "consolidated"]`
- Vérifier que `grain` ne contient que des dimensions propagées valides
- Vérifier que `selection` utilise des dimensions et valeurs valides (comme `rules::validate_definition`)
- Si `expression` est fournie, la compiler via `formula::compile` avec un `ControlOperandResolver`
- Vérifier que `assertions` n'est pas vide
- Si `compare` est fourni, vérifier que `metric` est valide et que `baseline_consolidation_id` existe (si renseigné). Interdire `compare` si le seul niveau ciblé est `raw`.

#### OperandResolver pour contrôles

```rust
struct ControlOperandResolver { ... }

impl OperandResolver for ControlOperandResolver {
    fn resolve(&self, name: &str) -> Result<Resolved, String> {
        // Pour les niveaux pipeline (corporate/converted/consolidated) :
        // 1. Chercher dans dim_aggregate (poste) → SUM(e.amount) FILTER (WHERE ...)
        // 2. Chercher dans dim_indicator (indicateur) → sous-requête
        // 3. Sinon → erreur
        //
        // Pour le niveau raw : les postes/indicateurs ne sont pas applicables
        // (ils portent sur fact_entry). L'expression doit être une formule
        // arithmétique simple sur SUM(e.amount) ou une erreur est retournée.
    }
}
```

#### Exécution (`run_control`)

Pour chaque niveau ciblé :

**Niveaux `corporate` / `converted` / `consolidated`** (source : `fact_entry`) :
```sql
-- 1. Agrégation par grain
WITH ctrl_data AS (
    SELECT
        <grain_columns>,
        <expression_sql> AS value,
        COUNT(*) AS row_count,
        ARRAY_AGG(e.id) AS sample_ids
    FROM fact_entry e
    <joins for selection traversals>
    WHERE e.consolidation_id = ?
      AND e.level = ?
      <selection_filters>
    GROUP BY <grain_columns>
)
-- 2. Comparaison inter-périodes (si applicable)
SELECT
    cd.*,
    bd.value AS baseline,
    CASE
        WHEN bd.value IS NULL THEN NULL
        WHEN bd.value = 0 THEN NULL
        ELSE ABS(cd.value - bd.value) / ABS(bd.value) * 100
    END AS variation
FROM ctrl_data cd
LEFT JOIN (
    -- même requête mais consolidation_id = baseline
) bd ON <grain_join>
```

**Niveau `raw`** (source : `stg_entry`) :
```sql
SELECT
    <grain_columns>,
    <expression_sql> AS value,
    COUNT(*) AS row_count,
    ARRAY_AGG(s.id) AS sample_ids
FROM stg_entry s
WHERE s.phase = ?
  AND s.entry_period = ?
  <selection_filters_on_text_columns>
GROUP BY <grain_columns>
```

Le niveau `raw` ne supporte pas la comparaison inter-périodes (pas de notion de consolidation). Les filtres portent directement sur les colonnes TEXT de `stg_entry`.

#### Évaluation des assertions

Pour chaque ligne résultante, parcourir les assertions et déterminer le statut :

```rust
fn evaluate(assertions: &[Assertion], value: f64, has_data: bool) -> Status {
    if !has_data { return Status::NoData; }
    let mut worst = Status::Pass;
    for a in assertions {
        let s = match a {
            Assertion::Range { warn, error } => {
                if value.abs() > *error { Status::Error }
                else if value.abs() > *warn { Status::Warn }
                else { Status::Pass }
            }
            Assertion::Nonzero => {
                if value == 0.0 { Status::Error } else { Status::Pass }
            }
            Assertion::Existence => Status::Pass, // déjà géré par has_data
            Assertion::Equals { target } => {
                if (value - target).abs() > 0.01 { Status::Error }
                else { Status::Pass }
            }
        };
        worst = worst.max(s);
    }
    worst
}
```

#### Point d'entrée API

```rust
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/controls", get(list).post(create))
        .route("/api/controls/{code}", get(get_ctrl).put(update).delete(delete_ctrl))
        .route("/api/controls/{code}/run", post(run_single))
        .route("/api/controls/operands", get(operands_catalog))
        .route("/api/control-sets", get(list_sets).post(create_set))
        .route("/api/control-sets/{code}", get(get_set).put(update_set).delete(delete_set))
        .route("/api/control-sets/{code}/run", post(run_set))
        .route("/api/control-sets/{code}/results", get(get_results))
}
```

### 8.2 Intégration dans `server.rs`

Ajouter `pub mod controls;` dans `lib.rs` et `.merge(controls::router())` dans `server.rs`.

### 8.3 Intégration dans `schema.rs`

Ajouter les DDL des tables `dim_control`, `dim_control_set`, `dim_control_set_item`, `fact_control_result` dans `ALL_DDL`.

`dim_control`, `dim_control_set`, `dim_control_set_item` : ajoutées à la liste des tables qui survivent au reset (hors `ALL_DROP`).

`fact_control_result` : dans `ALL_DROP` (résultats volatiles).

### 8.4 Intégration dans `json_migration.rs`

Ajouter la normalisation/dénormalisation des définitions de contrôles (code → id pour les valeurs de sélection, comme les postes).

---

## 9. Frontend — TypeScript

### 9.1 Types (`types.ts`)

```typescript
interface ControlAssertion {
  type: 'range' | 'nonzero' | 'existence' | 'equals';
  warn?: number;
  error?: number;
  target?: number;
}

interface ControlCompare {
  metric: 'variation_abs' | 'variation_pct' | 'variation';
  baseline_consolidation_id: number | null;
  warn: number;
  error: number;
}

interface ControlDefinition {
  levels: ('raw' | 'corporate' | 'converted' | 'consolidated')[];
  grain: string[];
  selection: SelectionCond[];
  expression: string | null;
  assertions: ControlAssertion[];
  compare: ControlCompare | null;
}

interface Control {
  code: string;
  libelle: string | null;
  definition: ControlDefinition;
}

interface ControlSet {
  code: string;
  libelle: string | null;
  controls: { code: string; libelle: string | null; ord: number }[];
}

type ControlStatus = 'pass' | 'warn' | 'error' | 'no_data';

interface ControlRowResult {
  grain: Record<string, string | null>;
  value: number | null;
  baseline: number | null;
  variation: number | null;
  status: ControlStatus;
  row_count: number;
}

interface ControlLevelResult {
  status: ControlStatus;
  rows: ControlRowResult[];
}

interface ControlReport {
  control_code: string;
  control_libelle: string | null;
  levels: Record<string, ControlLevelResult>;
}

interface ControlSetReport {
  set_code: string;
  executed_at: string;
  consolidation_id: number | null;
  phase: string | null;
  entry_period: string | null;
  summary: {
    total: number;
    by_level: Record<string, Record<string, number>>;
  };
  details: ControlReport[];
}
```

### 9.2 API client (`api.ts`)

```typescript
api.controls = {
  list:     () => getJson<Control[]>('/api/controls'),
  get:      (code: string) => getJson<Control>(`/api/controls/${code}`),
  create:   (body: Control) => postJson<Control>('/api/controls', body),
  update:   (code: string, body: Control) => putJson<Control>(`/api/controls/${code}`, body),
  remove:   (code: string) => deleteJson(`/api/controls/${code}`),
  run:      (code: string, params: { consolidation_id?: number; phase?: string; entry_period?: string }) => postJson<ControlReport>(`/api/controls/${code}/run`, params),
  operands: () => getJson<ControlOperand[]>('/api/controls/operands'),
};

api.controlSets = {
  list:     () => getJson<ControlSet[]>('/api/control-sets'),
  get:      (code: string) => getJson<ControlSet>(`/api/control-sets/${code}`),
  create:   (body: ControlSet) => postJson<ControlSet>('/api/control-sets', body),
  update:   (code: string, body: ControlSet) => putJson<ControlSet>(`/api/control-sets/${code}`, body),
  remove:   (code: string) => deleteJson(`/api/control-sets/${code}`),
  run:      (code: string, params: { consolidation_id?: number; phase?: string; entry_period?: string }) => postJson<ControlSetReport>(`/api/control-sets/${code}/run`, params),
  results:  (code: string) => getJson<ControlSetReport>(`/api/control-sets/${code}/results`),
};
```

### 9.3 Page (`ControlsPage.tsx`)

Nouveau fichier `web/src/pages/ControlsPage.tsx` (~800-1000 lignes estimées).

Structure :
- Split panel gauche (liste des contrôles) / droite (éditeur)
- Éditeur : formulaires avec composants réutilisés (`ConditionFields`, `FormulaEditor`, `OperandPalette`)
- Section "Jeux de contrôles" en bas de la colonne gauche
- Panneau de rapport d'exécution (apparaît après exécution)
- Multi-select pour les niveaux (chips toggle)

### 9.4 Intégration dans la sidebar (`Layout.tsx`)

Ajouter `{ id: 'controles', label: 'Contrôles' }` dans le groupe `calculs` de `GROUPS`.

### 9.5 Intégration dans `App.tsx`

Ajouter le mapping `page === 'controles' && <ControlsPage />` et le type `PageId`.

---

## 10. Plan d'implémentation

### Phase 1 — Moteur Rust (backend)

| # | Tâche | Fichiers | Estimation |
|---|-------|----------|------------|
| 1.1 | Définir les structures (`ControlDefinition`, `Assertion`, `Compare`) dans `controls.rs` | `prototype/rust/src/controls.rs` | — |
| 1.2 | Ajouter les DDL des tables dans `schema.rs` | `prototype/rust/src/schema.rs` | — |
| 1.3 | Implémenter `validate_definition()` | `prototype/rust/src/controls.rs` | — |
| 1.4 | Implémenter `ControlOperandResolver` | `prototype/rust/src/controls.rs` | — |
| 1.5 | Implémenter `run_control()` — SQL d'agrégation + assertions | `prototype/rust/src/controls.rs` | — |
| 1.6 | Implémenter `run_set()` — orchestration multi-contrôles | `prototype/rust/src/controls.rs` | — |
| 1.7 | Implémenter le CRUD (list, get, create, update, delete) | `prototype/rust/src/controls.rs` | — |
| 1.8 | Implémenter les handlers REST + `router()` | `prototype/rust/src/controls.rs` | — |
| 1.9 | Brancher dans `lib.rs` et `server.rs` | `lib.rs`, `server.rs` | — |
| 1.10 | Intégrer dans `json_migration.rs` | `json_migration.rs` | — |
| 1.11 | Tests unitaires + test d'intégration | `controls.rs` (tests module) | — |

### Phase 2 — Frontend TypeScript

| # | Tâche | Fichiers | Estimation |
|---|-------|----------|------------|
| 2.1 | Ajouter les types TypeScript | `web/src/types.ts` | — |
| 2.2 | Ajouter les endpoints API client | `web/src/api.ts` | — |
| 2.3 | Créer `ControlsPage.tsx` — layout split panel | `web/src/pages/ControlsPage.tsx` | — |
| 2.4 | Implémenter l'éditeur de contrôle (sélection, expression, assertions, comparaison) | `web/src/pages/ControlsPage.tsx` | — |
| 2.5 | Implémenter la section jeux de contrôles | `web/src/pages/ControlsPage.tsx` | — |
| 2.6 | Implémenter le panneau de rapport d'exécution | `web/src/pages/ControlsPage.tsx` | — |
| 2.7 | Ajouter dans la sidebar (Layout.tsx) et App.tsx | `Layout.tsx`, `App.tsx` | — |
| 2.8 | CSS pour les statuts (pass/warn/error/no_data badges) | `App.css` | — |

### Phase 3 — Tests et intégration

| # | Tâche | Fichiers |
|---|-------|----------|
| 3.1 | Tests end-to-end : créer un contrôle, l'exécuter, vérifier le rapport | — |
| 3.2 | Seed de démo : ajouter quelques contrôles de démonstration | `seed.rs` |
| 3.3 | Mise à jour de `docs/MODELE_DONNEES.md` | `docs/MODELE_DONNEES.md` |
| 3.4 | Mise à jour de `docs/TECHNIQUE.md` | `docs/TECHNIQUE.md` |

---

## 11. Décisions

| ID | Question | Décision |
|----|----------|----------|
| QC1 | Source de données | **`raw` comme niveau**. `levels` accepte `["raw", "corporate", "converted", "consolidated"]`. Le niveau `raw` cible `stg_entry` (colonnes TEXT), les autres ciblent `fact_entry` (colonnes INTEGER résolues). Pas de champ `source` séparé — le niveau détermine la table. |
| QC2 | Comparaison inter-périodes | **Phase libre**. Le champ `compare` gagne un `baseline_consolidation_id` explicite (optionnel). Si absent, le moteur déduit automatiquement la même phase N-1. L'utilisateur peut comparer Réel N vs Budget N, Réel N vs Réel N-1, etc. |
| QC3 | Historique | **Dernier résultat seulement**. `fact_control_result` est tronquée à chaque run. Pas de table d'historique. |
| QC4 | Contrôles natifs | **Pas de natifs**. Tous les contrôles sont utilisateur. |
