# Tâche : UI Règles de consolidation (React + TypeScript)

## Contexte
Le frontend est dans `~/cf-clone/web/` (React 19 + TypeScript + Vite + TanStack Table). L'API backend expose déjà toutes les routes nécessaires :
- `GET/POST /api/rules`, `GET/PUT/DELETE /api/rules/{code}`
- `GET/POST /api/rulesets`, `GET/PUT/DELETE /api/rulesets/{code}`
- `POST /api/rules/run` (body: `{ ruleset: "CODE" }`)

## Fichiers à modifier

### 1. `src/types.ts`
- Renommer `audit_id` → `analysis2` dans l'interface `Entry`
- Ajouter :

```typescript
export interface RuleSummary { code: string; libelle: string; }
export interface RuleDetail { code: string; libelle: string; definition: object; }

// Une condition de périmètre
export interface ScopeCond {
  target: 'entity' | 'partner';
  dim: string;     // methode, pct_interet, pct_integration, entree, sortie
  op: string;      // =, !=, >, <, >=, <=, IN, IS NULL, IS NOT NULL
  val: unknown;
}

// Une condition de sélection sur fact_entry
export interface SelectionCond {
  dim: string;     // scenario, entity, account, flow, etc.
  op: string;
  val: unknown;
}

// Une opération
export interface Operation {
  seq: number;
  level: string;   // corporate, reclassified, converted, consolidated
  selection: SelectionCond[];
  coefficient: { type: string; value?: number };  // pct_integration | pct_interet | constant
  multiplicateur: number;
  destination: Record<string, { mode: 'inherit' | 'override' | 'null'; value?: string }>;
}

// Définition complète d'une règle
export interface RuleDefinition {
  scope: ScopeCond[];
  operations: Operation[];
}

export interface RulesetSummary { code: string; libelle: string; }
export interface RulesetItem { ordre: number; rule_code: string; rule_libelle?: string; }
export interface RulesetDetail { code: string; libelle: string; items: RulesetItem[]; }
export interface RuleResult { rule_code: string; level: string; generated: number; }
export interface RulesetReport { ruleset: string; rules: RuleResult[]; total_generated: number; }
```

### 2. `src/api.ts`
Ajouter dans l'objet `api` :

```typescript
rules: {
  list: () => getJson<RuleSummary[]>('/rules'),
  get: (code: string) => getJson<RuleDetail>(`/rules/${code}`),
  create: (body: { code: string; libelle: string; definition: object }) =>
    postJsonRaw<RuleDetail>('/rules', body),
  update: (code: string, body: { libelle?: string; definition?: object }) =>
    putJson<RuleDetail>(`/rules/${code}`, body),
  remove: (code: string) => deleteJson<{ deleted: number }>(`/rules/${code}`),
},
rulesets: {
  list: () => getJson<RulesetSummary[]>('/rulesets'),
  get: (code: string) => getJson<RulesetDetail>(`/rulesets/${code}`),
  create: (body: { code: string; libelle: string; items: { ordre: number; rule_code: string }[] }) =>
    postJsonRaw<RulesetDetail>('/rulesets', body),
  update: (code: string, body: { libelle?: string; items?: { ordre: number; rule_code: string }[] }) =>
    putJson<RulesetDetail>(`/rulesets/${code}`, body),
  remove: (code: string) => deleteJson<{ deleted: number }>(`/rulesets/${code}`),
  run: (ruleset: string) => postJsonRaw<RulesetReport>('/rules/run', { ruleset }),
},
```

### 3. `src/components/Layout.tsx`
- Ajouter `'regles'` au type `PageId`
- Ajouter `{ id: 'regles', label: 'Règles' }` dans TABS

### 4. `src/App.tsx`
- Importer et rendre `RulesPage` quand `page === 'regles'`

### 5. `src/pages/RulesPage.tsx` (NOUVEAU)

Page avec 2 sous-onglets internes : **Bibliothèque** et **Jeux de règles**.

#### Sous-onglet « Bibliothèque »
- Tableau des règles (code, libelle) avec boutons Éditer/Supprimer
- Bouton « Nouvelle règle » → formulaire modal :
  - Code (PK, verrouillé en édition)
  - Libellé
  - **Scope** : liste dynamique de conditions. Chaque condition = ligne avec :
    - target (select: entity / partner)
    - dim (select: methode, pct_interet, pct_integration, entree, sortie)
    - op (select: =, !=, >, <, >=, <=, IN, IS NULL, IS NOT NULL)
    - val (input text — caché si IS NULL / IS NOT NULL)
    - Bouton supprimer
    - Bouton « + Ajouter une condition »
  - **Opérations** : liste dynamique d'opérations (empilées verticalement). Chaque opération :
    - seq (number, autoincrémenté)
    - level (select: corporate / reclassified / converted / consolidated)
    - **Sélection** : sous-liste dynamique de conditions (dim select parmi les colonnes fact_entry, op, val) avec bouton +
    - Coefficient (select: pct_integration / pct_interet / constant) + champ value si constant
    - Multiplicateur (number, défaut 1)
    - **Destination** : pour chaque dimension pilotable (entity, account, flow, nature, partner, share), une ligne avec :
      - nom de la dimension (label fixe)
      - mode (select: inherit / override / null)
      - value (input text, visible seulement si override)
    - Bouton supprimer l'opération
    - Bouton « + Ajouter une opération »

Le formulaire sérialise tout en JSON (`RuleDefinition`) et l'envoie comme `definition`.

#### Sous-onglet « Jeux de règles »
- Tableau des rulesets (code, libelle, nb règles) avec boutons Éditer/Supprimer/Exécuter
- Bouton « Nouveau jeu » → formulaire modal :
  - Code (PK)
  - Libellé
  - **Items** : liste ordonnée de règles. Chaque item = ligne avec :
    - ordre (number)
    - rule_code (select depuis la bibliothèque de règles, chargée via `api.rules.list()`)
    - Bouton supprimer
    - Boutons monter/descendre pour réordonner
  - Bouton « + Ajouter une règle au jeu »
- Bouton **Exécuter** sur chaque ruleset → `api.rulesets.run(code)` → affiche le rapport (nombre de lignes générées par règle/niveau)

### 6. `src/App.css`
Ajouter les styles nécessaires. Suivre les conventions existantes :
- Utiliser les classes CSS existantes (`.page`, `.page__header`, `.btn`, `.modal`, `.field`, `.grid`, etc.)
- Nouvelles classes pour les listes dynamiques de conditions/opérations :
  - `.rule-section` : bloc avec titre (Scope / Opérations)
  - `.rule-condition` : ligne de condition (flexbox horizontal)
  - `.rule-operation` : carte d'opération (bordure, padding, margin-bottom)
  - `.rule-dest-row` : ligne de destination dimension
  - `.rule-add-btn` : bouton d'ajout stylé discret

## Constantes utiles
```typescript
const PILOTABLE_DIMS = ['entity', 'account', 'flow', 'nature', 'partner', 'share'];
const SELECTION_DIMS = ['scenario', 'entity', 'entry_period', 'period', 'account', 'flow', 'currency', 'nature', 'partner', 'share', 'analysis', 'analysis2', 'level'];
const SCOPE_DIMS = ['methode', 'pct_interet', 'pct_integration', 'entree', 'sortie'];
const LEVELS = ['corporate', 'reclassified', 'converted', 'consolidated'];
const OPS = ['=', '!=', '>', '<', '>=', '<=', 'IN', 'IS NULL', 'IS NOT NULL'];
const COEFF_TYPES = ['pct_integration', 'pct_interet', 'constant'];
```

## Contraintes
- Tout en français (libellés, commentaires)
- Suivre strictement les patterns existants (useCallback/useEffect/useState, TanStack Table pour les tableaux)
- Le formulaire de règle est un modal (comme MasterDataPage), pas une page séparée
- TypeScript strict — pas d'`any`
- Après `npm run build`, le build doit passer sans erreur

## Vérification finale
```bash
cd ~/cf-clone/web && npm run build
```
Le build TypeScript doit réussir.
