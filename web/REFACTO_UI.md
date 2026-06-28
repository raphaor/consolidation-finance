# Plan de refactorisation UI — factorisation des duplications

> Périmètre : **frontend uniquement** (`web/src/`). **Aucun impact `conso-server`** :
> pas de route, pas de contrat JSON, pas de type d'échange touché. Vérification à
> chaque vague : `npm run build` (= `tsc -b && vite build`) + `npm run lint`.

## Contexte

La couche UI a été construite par couches successives. Le socle partagé est sain
(`ConditionFields`, `FormulaEditor`, `OperandPalette`, `MasterDetailEditor`,
`useDimValues`, `format.ts`). La dette se concentre sur les pages ajoutées
ensuite (Coefficients, Postes, Indicateurs, Contrôles, Règles) qui réimplémentent
chacune le même squelette « bibliothèque à gauche + éditeur à droite ».

## Vagues

### Vague 1 — mécanique, risque nul

1. **`errMsg(e)`** — `utils/errMessage.ts`. Remplace les ~65 occurrences de
   `e instanceof Error ? e.message : String(e)` (15 fichiers).
2. **Constantes/helpers de formule** — déplacer `FUNCTIONS`
   (`['MIN','MAX','SAFE_DIV','IF','ABS','ROUND']`) et `formatValue`
   (`Number(v.toFixed(6))`) dans `utils/format.ts`. Supprimer les copies de
   `CoefficientsPage` et `IndicatorsPage`.
3. **Classes CSS** — remplacer les styles inline répétés par des classes :
   `.editor-split` (`flex; gap:24; align-items:flex-start`), `.editor-pane`
   (`flex:1; min-width:0`), `.form-actions` (`margin-top:12; display:flex; gap:8`).
4. **`<PageHeader title hint>`** — `components/PageHeader.tsx`. Remplace les
   `<div className="page__header">…</div>` recopiés et **corrige l'incohérence**
   `<h1>` de `ControlsPage` (toutes les autres pages utilisent `<h2>`).

### Vague 2 — hooks & composants partagés

5. **`useDimensionMetadata()`** — `hooks/useDimensionMetadata.ts`. Factorise le
   quadruplet `dimensions/characteristics/customReferences/nativeEnums` chargé
   dans `ControlsPage`, `IndicatorsPage` (Postes), `RulesPage`,
   `CaracteristiquesPage` (partiel).
6. **`<SubTabs items active onChange>`** — `components/SubTabs.tsx`. Remplace les
   boutons `subtab` recodés à la main dans `ControlsPage` et `CaracteristiquesPage`.

### Vague 3 — structurant (page par page)

7. **`useCrudResource({ list, create, update, remove })`** — gère
   `selected: string|'new'|null` + `form` + `saving` + `reload`/`open`/`save`/`remove`.
8. **`<LibraryList>`** — table de gauche « + Nouveau X » + colonne code + badge +
   actions. Appliqué à Coefficients, Postes, Indicateurs, Contrôles, Règles.

   Procéder **une page à la fois**, `npm run build` entre chaque migration.

## Vérification

- `npm run build` après chaque vague (échec de type = stop).
- `npm run lint`.
- Revue visuelle laissée à l'utilisateur (il lance le serveur lui-même).

## État de livraison

**Vague 1 — livrée.**
- `utils/errMessage.ts` (`errMsg`, avec fallback optionnel) ; ~65 occurrences
  remplacées dans 15 fichiers.
- `utils/format.ts` : `FORMULA_FUNCTIONS` + `formatFormulaValue` mutualisés.
- `components/PageHeader.tsx` ; appliqué à Coefficients, Postes, Indicateurs,
  Contrôles (corrige le `<h1>` → `<h2>`).
- CSS : `.editor-split`, `.editor-pane`, `.editor-actions`.

**Vague 2 — livrée.**
- `hooks/useDimensionMetadata.ts` ; consommé par Contrôles et Postes.
- `components/SubTabs.tsx` ; appliqué à Contrôles et Attributs de dimension.
  `margin-bottom` ajouté à `.subtabs`.

**Vague 3 — livrée (périmètre formules).**
- `hooks/useCrudResource.ts` + `components/LibraryList.tsx`.
- Appliqués à **Coefficients, Postes, Indicateurs**.
- **Exclus volontairement** : `ControlsPage` et `RulesPage`. Leurs éditeurs
  divergent trop du gabarit (onglets internes, exécution/run, modales de
  renommage, items ordonnés, page Règles ~2000 lignes) : les forcer dans le
  hook générique présentait un risque de régression disproportionné. Candidats
  à une étape ultérieure si le gabarit se stabilise.

**Aucun impact `conso-server`** : toutes les modifications sont dans `web/src/`.
Lint inchangé (7 problèmes préexistants, 0 introduit) ; `npm run build` OK.
