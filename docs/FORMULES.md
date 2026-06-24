# Moteur de formules — coefficients utilisateur & indicateurs

> Spec fonctionnelle du module **formules** ([Q43](./QUESTIONS_OUVERTES.md)). Statut : **volet 1 (coefficients) IMPLÉMENTÉ** (2026-06-24) ; volet 2 (indicateurs/KPI) à venir. Inspiration ergonomique : l'éditeur de formules de Pigment.
>
> **Implémentation volet 1** : moteur pur `prototype/rust/src/formula.rs` (lexer/parser/AST → compilation `(SQL, CoeffJoins)` + interpréteur f64 pour la preview) ; bibliothèque `prototype/rust/src/coefficients.rs` (table `dim_coefficient`, seed des natifs comme formules, résolution, API REST) ; intégration `rules.rs` (enum `Coefficient` → `Constant` | `Named`, résolution déléguée). Front : `web/src/pages/CoefficientsPage.tsx` (éditeur + preview) + menu Coefficient de `RulesPage` alimenté par la bibliothèque. Tests : 101 unitaires lib + suites `pipeline` (16) / `rules` (15) vertes.
>
> Décision de cadrage (2026-06-24, avec l'utilisateur) : **le volet 1 (coefficients de règles) est prioritaire**, le volet 2 (indicateurs / KPI) vient ensuite. Les deux partagent **le même moteur de formules** (lexer / parser / AST / éditeur) ; seul le **catalogue d'opérandes** change selon le contexte.
>
> Les cinq questions de conception **F1–F5 sont toutes tranchées** (2026-06-24, cf. §8) : la spec est prête pour l'implémentation de la phase 1.

---

## 1. Objectif & double ancrage

Donner à l'utilisateur un **langage de formules proche d'Excel** pour exprimer des calculs, branché sur **deux points** de l'application :

| Volet | Ce qu'on calcule | Sortie | Statut |
|-------|------------------|--------|--------|
| **1 — Coefficients** (prioritaire) | Un **coefficient** appliqué au montant d'une opération de règle, calculé à partir des taux de périmètre. | Un scalaire par grain d'écriture, multiplié au montant. | Spec cible de ce document. |
| **2 — Indicateurs / KPI** | Une **mesure dérivée** (marge, ROE, BFR, ratio d'endettement…) combinant des soldes consolidés. | Une valeur (ou une série) affichée dans les rapports / un dashboard. **Jamais réinjectée dans `fact_entry`.** | Conception esquissée ici, implémentation en phase 2. |

Le fil rouge : **un seul moteur, deux catalogues d'opérandes**. C'est ce qui rend la fonctionnalité cohérente avec la philosophie *data-driven, SQL déclaratif* du moteur, plutôt qu'un module greffé.

### 1.1 Pourquoi le volet 1 généralise l'existant

Aujourd'hui, le coefficient d'une opération de règle est une **liste fermée** codée en dur (`rules.rs::Coefficient`) : `pct_integration`, `pct_interet`, `constant`, `elim_ic_corp_n` / `_n1` / `_var`. Or les `elim_ic_corp_*` **sont déjà des formules écrites à la main** :

```
elim_ic_corp_n   ≡   Min(1, INTEG_PA / INTEG_EN)
elim_ic_corp_var ≡   Min(1, INTEG_PA / INTEG_EN) − Min(1, INTEG_PA_N1 / INTEG_EN_N1)
```

Le moteur de formules **transforme cette enum en données** : chaque coefficient natif devient une formule prédéfinie de la bibliothèque, et l'utilisateur peut en écrire de nouvelles sans toucher au Rust — débloquant les intérêts minoritaires, les retraitements, la répartition des résultats (cf. [`REGLES_CONSO.md`](./REGLES_CONSO.md) §10).

---

## 2. Le langage de formules (commun aux deux volets)

Volontairement petit, syntaxe Excel pour une prise en main immédiate.

### 2.1 Grammaire (informelle)

```
expr      := terme (('+' | '−') terme)*
terme     := facteur (('×' | '÷') facteur)*       # '*' et '/' acceptés en saisie
facteur   := nombre
           | reference                            # [ … ]
           | fonction '(' args ')'
           | '(' expr ')'
           | ('−' | '+') facteur                  # unaire
           | expr comparateur expr                # dans un IF
args      := expr (';' expr)*                     # séparateur d'arguments : ';'
comparateur := '>' | '<' | '>=' | '<=' | '=' | '<>'
reference := '[' nom ']'                          # résolu contre le catalogue du contexte
nombre    := littéral décimal (point décimal)
```

- **Références** entre crochets `[ … ]` : résolues à l'enregistrement contre le **catalogue d'opérandes du contexte** (périmètre pour le volet 1, agrégats/indicateurs pour le volet 2). Inconnu → erreur de validation.
- Le **séparateur d'arguments** est `;` (cohérent avec la convention Excel francophone, évite l'ambiguïté avec le séparateur décimal).

### 2.2 Fonctions de base

| Fonction | Sémantique | Note |
|----------|------------|------|
| `MIN(a ; b ; …)` / `MAX(a ; b ; …)` | Minimum / maximum. | **Pas** `LEAST`/`GREATEST` SQL (qui ignorent les NULL sous DuckDB) — compilé en `CASE` explicite, cf. `rules.rs::min_ratio`. |
| `ABS(a)` | Valeur absolue. | |
| `ROUND(a ; n)` | Arrondi à `n` décimales. | |
| `IF(cond ; a ; b)` | Conditionnel. | `cond` = comparaison. |
| `SAFE_DIV(a ; b)` | Division protégée : `b = 0 → 0`. | **Disponible** pour qui veut se protéger, mais **non imposée** : c'est à l'auteur de la formule de l'employer là où un dénominateur peut être nul (décision F3). |
| `÷` / `/` | Division simple. | Autorisée sans garde-fou automatique. `b = 0` produit le comportement SQL natif — la protection relève de l'utilisateur (cf. `SAFE_DIV`). |

> Le jeu de fonctions est volontairement minimal au départ ; il s'étend par ajout dans le compilateur (une entrée = un patron SQL), jamais par interpolation de chaîne utilisateur.

### 2.3 Précision

Les montants restent en `rust_decimal::Decimal` (jamais de `f64`, cf. CLAUDE.md). Les coefficients sont manipulés en flottant dans l'expression SQL (`coefficient_expr` émet des littéraux `f64`) ; le moteur de formules **conserve cette parité** (décision F1, 2026-06-24). Justification : un coefficient est un taux (ex. 0,80) où les micro-erreurs du flottant binaire sont négligeables, et le produit `montant × coefficient` se fait de toute façon en `Decimal`. Pas de bascule `DECIMAL` prévue.

---

## 3. Volet 1 — Coefficients utilisateur (prioritaire)

### 3.1 Contexte d'évaluation

Un coefficient est évalué **au grain d'une écriture source** d'une opération de règle, exactement comme aujourd'hui. La compilation produit le couple attendu par le moteur :

```
formule  ──compilation──▶  (expr_sql, CoeffJoins)
```

`expr_sql` est l'expression scalaire injectée dans `e.amount × ({expr_sql}) × {mult}` ; `CoeffJoins` indique quelles perspectives de `sat_perimeter` joindre. **C'est le point d'insertion exact de `rules.rs::coefficient_expr`** — le compilateur de formules se substitue à cette fonction (ou la complète) sans changer le reste de `exec_operation`.

### 3.2 Catalogue d'opérandes (périmètre)

Les références `[ … ]` d'un coefficient pointent vers des **valeurs de périmètre** lues sur `sat_perimeter`, à l'une des **quatre perspectives** déjà gérées par `CoeffJoins` :

| Perspective | Entité lue | Période | JOIN (`CoeffJoins`) |
|-------------|-----------|---------|----------------------|
| `EN` | l'entité de l'écriture | courante | `p_ent` |
| `PA` | le partenaire de l'écriture | courante | `p_part` |
| `EN_N1` | l'entité | N-1 (via à-nouveau) | `p_ent_n1` |
| `PA_N1` | le partenaire | N-1 (via à-nouveau) | `p_part_n1` |

Champs disponibles par perspective : `pct_integration`, `pct_interet` (et tout champ scalaire de `sat_perimeter` whitelisté). La source N-1 réutilise le **snapshot de la consolidation d'à-nouveau** (`dim_consolidation.a_nouveau_consolidation_id`), comme le carry — zéro schéma supplémentaire (cf. [Q40](./QUESTIONS_OUVERTES.md), [`A_NOUVEAU.md`](./A_NOUVEAU.md)).

**Référence proposée (lisible, autocomplétée)** :

```
[Intégration · entité]            → COALESCE(p_ent.pct_integration, 0)
[Intégration · partenaire]        → COALESCE(p_part.pct_integration, 0)
[Intégration · entité N-1]        → COALESCE(p_ent_n1.pct_integration, 0)
[Intérêt · entité]                → COALESCE(p_ent.pct_interet, 0)
```

Chaque référence se résout en `(champ, perspective)` → émet l'expression `COALESCE(p_<persp>.<champ>, 0)` **et** lève le flag de JOIN correspondant.

**Défaut uniforme = 0** (décision F3, 2026-06-24) : tout taux de périmètre absent (entité ou partenaire hors périmètre, perspective N-1 d'une entité entrante) vaut **0**, sans exception. Conséquences assumées :

- Un coefficient `pct_integration` posé **seul** sur une écriture dont l'entité est hors périmètre **annule** l'écriture (× 0). C'est un **changement de comportement** par rapport à l'actuel `coefficient_expr` (qui retombait sur `1.0` pour `pct_integration`/`pct_interet` solo) : choix délibéré, pas de magie de neutralité.
- La vigilance « n'utiliser un coefficient à partenaire que là où il y a un partenaire » relève de **l'utilisateur**, pas d'un défaut implicite.
- Conséquence de modèle : **un seul catalogue plat d'opérandes** (tous à défaut 0), plus besoin de distinguer « coefficients prêts à l'emploi » (défaut 1) et « briques de formule » (défaut 0).

> **Pas de taux de change dans un coefficient** : la conversion FX reste du ressort du pipeline natif (étape C). Contrainte héritée de [`REGLES_CONSO.md`](./REGLES_CONSO.md) R2 — maintenue. Le catalogue d'opérandes des coefficients est donc **strictement le périmètre**.

### 3.3 Exemples

Les trois coefficients natifs, ré-exprimés en formules (équivalence stricte) :

```
pct_integration   =  [Intégration · entité]
pct_interet       =  [Intérêt · entité]
elim_ic_corp_n    =  MIN(1 ; SAFE_DIV([Intégration · partenaire] ; [Intégration · entité]))
elim_ic_corp_var  =  MIN(1 ; SAFE_DIV([Intégration · partenaire] ;     [Intégration · entité]))
                   −  MIN(1 ; SAFE_DIV([Intégration · partenaire N-1] ; [Intégration · entité N-1]))
```

Coefficients nouveaux, désormais possibles **sans code** :

```
quote-part minoritaire  =  1 − [Intérêt · entité]
écart intérêt/intégration =  [Intérêt · entité] − [Intégration · entité]
```

### 3.4 Persistance — bibliothèque de coefficients

Modèle aligné sur la bibliothèque de règles ([`REGLES_CONSO.md`](./REGLES_CONSO.md) §8, principe d'immutabilité) :

- **Table `dim_coefficient`** : `code` (PK), `libellé`, `expression` (texte de la formule), `kind` (`builtin` | `user`).
- Les coefficients **natifs** sont **seedés** dans cette table comme `builtin` (formules prédéfinies) — l'enum `Coefficient` en dur devient un *fast-path* / un seed, plus la seule source de vérité.
- Une opération de règle référence un coefficient **par `code`** (le champ `coefficient` du JSON d'opération accepte soit un code de la bibliothèque, soit le mode `constant` inline existant — rétro-compatible).
- **Modifiable en place** (décision F2/F4, 2026-06-24) : un coefficient est un réglage **vivant** — l'éditer met à jour toutes les règles qui le référencent au prochain run, **sans** copie ni versioning de la formule. Choix de simplicité assumé pour le POC : la traçabilité fine de « quelle formule a produit telle conso » n'est pas un objectif à ce stade (la provenance d'une ligne passe par `Source`, le versioning des traitements par les rulesets). Conséquence à connaître : un coefficient partagé entre plusieurs règles change leur résultat de façon globale quand on l'édite.

### 3.5 Validation à l'enregistrement

Miroir de `rules.rs::validate_definition` :

- **Parsing** : grammaire §2.1 ; parenthèses équilibrées ; arité des fonctions.
- **Références** : chaque `[ … ]` résout vers `(champ, perspective)` du catalogue périmètre — champ whitelisté contre les colonnes de `sat_perimeter`, perspective parmi les 4. Inconnu → rejet, message explicite.
- **Sécurité SQL** : aucun identifiant issu de la formule n'est interpolé brut. Les noms de champs/perspectives passent par les whitelists du registre ; seules les **constantes numériques** sont émises comme littéraux (formatés point-décimal, cf. `format_float`). Cf. CLAUDE.md « Sécurité SQL ».

### 3.6 UI (volet 1)

Dans l'éditeur de règle (`web/src/pages/RulesPage.tsx`), le sélecteur de coefficient d'une opération gagne :

- les coefficients **de la bibliothèque** (natifs + utilisateur) listés par `code - libellé` ;
- une option **« Nouvelle formule… »** qui ouvre l'**éditeur de formules** (§5) avec le catalogue *périmètre* ;
- l'option `constant` inline conservée.

Une page / sous-onglet **« Coefficients »** (sœur de la bibliothèque de règles) gère le CRUD des coefficients utilisateur.

---

## 4. Volet 2 — Indicateurs / KPI (phase 2)

Même langage, **catalogue d'opérandes différent** : des soldes agrégés de `fact_entry` au lieu de taux de périmètre.

### 4.1 Deux objets

- **Poste (agrégat nommé)** — brique de base : une **sélection sauvegardée** sur `fact_entry` (un `level` + des conditions dimensionnelles), agrégée en un montant signé. Réutilise **le modèle `SelectionCond`** des règles (`dim`/`op`/`val`, traversées `via`/`ref` comprises). Persisté dans `dim_aggregate` (`code`, `libellé`, `level`, `definition` JSON).

  ```
  [Chiffre d'affaires] = niveau consolidated · account via comportement = 'VENTES' · flow = 'F99'
  ```

- **Indicateur (formule)** — combine postes et autres indicateurs par une expression §2, avec un **grain** de restitution (dimensions d'affichage) et un **format** (%, ratio, nombre, devise). Persisté dans `dim_indicator` (`code`, `libellé`, `expression`, `grain` JSON, `format`).

  ```
  Marge opérationnelle = SAFE_DIV([Résultat d'exploitation] ; [Chiffre d'affaires])     # format %
  BFR                  = [Stocks] + [Créances clients] − [Dettes fournisseurs]
  Ratio d'endettement  = SAFE_DIV([Dettes financières] ; [Capitaux propres])
  ```

### 4.2 Compilation → SQL au grain

Pour un grain `G` (ex. `entity`), chaque poste devient un **agrégat conditionnel**, et la formule devient de l'arithmétique dans le `SELECT` — une seule requête, ensembliste :

```sql
SELECT entity,
       SAFE_DIVIDE( SUM(amount) FILTER (WHERE <sél. Résultat expl.>),
                    SUM(amount) FILTER (WHERE <sél. CA>) ) AS marge_op
FROM fact_entry
WHERE consolidation_id = ? AND level = 'consolidated'
GROUP BY entity
```

### 4.3 Non-additivité — garde-fou

Un ratio **n'est pas additif** : on ne somme pas des marges %. C'est le pendant de la sémantique *« of which »* ([`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §4 bis). Conséquences :

- un indicateur se calcule **au grain demandé**, recalculé pour chaque niveau d'agrégation (pas de somme d'une colonne de ratios) ;
- l'UI signale / empêche la sommation d'un total de ratios ;
- **les indicateurs ne sont jamais écrits dans `fact_entry`** — couche dérivée de présentation. Seuls les *coefficients* (volet 1) influencent les écritures, via les règles.

### 4.4 Surfaces

- Colonnes KPI dans `RapportsPage` (bilan / compte de résultat).
- Dashboard de cartes KPI (la vitrine).
- `POST /api/indicators/preview` : évalue une formule non sauvegardée → preview live (§5).

### 4.5 Comparaison temporelle (N-1) — décision F5

Le N-1 d'un indicateur est un **opérande nommé** (`[CA · N-1]`), résolu via le lien d'à-nouveau existant (`dim_consolidation.a_nouveau_consolidation_id`) — **pas** une fonction `PREV()`. Même patron que le N-1 des coefficients (§3.2), donc un seul mécanisme de N-1 dans toute l'appli.

```
Croissance CA = SAFE_DIV([CA] − [CA · N-1] ; [CA · N-1])
```

Conséquences : (1) **zéro impact sur la phase 1** — pas de fonction `PREV` à réserver, le lexer/parser/AST ne bougent pas ; le N-1 du volet 2 n'est que de **nouvelles entrées de catalogue** ajoutées en phase 2 (purement additif). (2) **Zéro schéma nouveau** (réutilise l'à-nouveau). (3) **Contrainte phase 1** : garder la résolution des opérandes **abstraite** (un objet *contexte* fournit les opérandes au compilateur), sans coder en dur « exactement une consolidation » — laisse la voie d'un **sélecteur de consolidation comparée libre** (réel vs budget, etc.) ouverte en extension future, sans réécrire le langage.

---

## 5. Éditeur de formules (ergonomie — le « ça claque »)

Composant React **commun** aux deux volets ; le catalogue d'opérandes est injecté selon le contexte (périmètre ou agrégats).

- **Barre de formule** avec coloration syntaxique (références / fonctions / nombres en couleurs distinctes).
- **Autocomplétion au `[`** : liste filtrable des références du catalogue (insertion à la position du curseur).
- **Panneau latéral de références** : bibliothèque des opérandes disponibles, insérables au clic (façon Pigment).
- **Preview live** : à droite, le résultat évalué sur la consolidation courante s'affiche en temps réel (le scalaire pour un coefficient sur un grain d'exemple ; le nombre ou une mini-table pour un indicateur). C'est l'élément qui donne la sensation « magique ».
- **Validation inline** : référence inconnue, parenthèse manquante, arité de fonction, incompatibilité de type → soulignés + message (réutilise le pattern `validate_definition`). La division non protégée **n'est pas** signalée comme une erreur (décision F3 : protection à la charge de l'utilisateur).
- Définition d'un poste via **le même sélecteur de conditions** que l'éditeur de règles (cohérence visuelle totale).

---

## 6. Modèle de données & API (récap)

| Objet | Table | Routes (proposées) |
|-------|-------|--------------------|
| Coefficient (volet 1) | `dim_coefficient` (`code`, `libellé`, `expression`, `kind`) | `/api/coefficients*`, `POST /api/coefficients/preview` |
| Poste (volet 2) | `dim_aggregate` (`code`, `libellé`, `level`, `definition`) | `/api/meta/aggregates*` |
| Indicateur (volet 2) | `dim_indicator` (`code`, `libellé`, `expression`, `grain`, `format`) | `/api/indicators*`, `POST /api/indicators/preview` |

Toutes ces tables **survivent au reset** (registre hors `ALL_DROP`, comme `dim_rule` / caractéristiques) et entrent dans l'export/import complet (`export.rs` / `import.rs`).

---

## 7. Phasage

1. **Phase 1 — Coefficients (prioritaire)** : moteur de formules (lexer/parser/AST), compilateur **contexte périmètre** → `(expr_sql, CoeffJoins)`, `dim_coefficient` + seed des natifs, éditeur de formules avec autocomplete + preview, intégration dans le sélecteur de coefficient de `RulesPage`.
2. **Phase 2 — Indicateurs / KPI** : compilateur **contexte agrégats** → SQL au grain, `dim_aggregate` + `dim_indicator`, colonnes KPI dans les rapports, dashboard, opérandes N-1 (§4.5).

> **Contrainte de conception phase 1** (issue de F5) : garder la résolution des opérandes derrière une abstraction *contexte* (le compilateur reçoit ses opérandes d'un fournisseur, sans présumer d'une consolidation unique). Coût nul, évite toute réécriture du langage au volet 2.

---

## 8. Questions ouvertes

Toutes tranchées le 2026-06-24.

| ID | Question | Décision |
|----|----------|----------|
| ~~F1~~ | **Précision des coefficients** : `f64` ou `DECIMAL` ? | **`f64`** (cf. §2.3). Un coefficient est un taux ; les micro-erreurs du flottant sont négligeables et le produit `montant × coefficient` reste en `Decimal`. Pas de bascule prévue. |
| ~~F2~~ | **Coefficient inline vs bibliothèque.** | **Bibliothèque nommée** `dim_coefficient` (cf. §3.4) ; `constant` inline conservé pour les cas triviaux. |
| ~~F3~~ | **Convention `COALESCE` par défaut.** | **Défaut uniforme = 0** pour tout taux absent (cf. §3.2). Pas de neutralité magique, pas de protection auto contre la division par zéro (`SAFE_DIV` disponible mais non imposée) — vigilance à la charge de l'utilisateur. Catalogue d'opérandes plat. **Changement** vs `coefficient_expr` actuel (`pct_integration`/`pct_interet` solo passaient de `1.0` à `0`). |
| ~~F4~~ | **Immutabilité** d'un coefficient référencé. | **Non — modifiable en place** (cf. §3.4). Réglage vivant, pas de versioning de la formule ; simplicité assumée pour le POC. |
| ~~F5~~ | **Indicateurs N-1 / intelligence temporelle.** | **Opérande nommé** (`[CA · N-1]`) résolu via l'à-nouveau, **pas** de fonction `PREV()` (cf. §4.5). Même patron que le N-1 des coefficients ; zéro impact phase 1 ; sélecteur de conso comparée libre laissé en extension future (contrainte : garder la résolution des opérandes abstraite en phase 1). |
