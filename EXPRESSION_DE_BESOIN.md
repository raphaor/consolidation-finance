# Expression de besoin — Outil de consolidation financière

> Document principal, **volontairement court**. Les points non tranchés sont reportés dans [`docs/QUESTIONS_OUVERTES.md`](./docs/QUESTIONS_OUVERTES.md) plutôt qu'éparpillés ici.

---

## 1. Vision

Un outil de consolidation **multi-entités, multi-devises, rapide**, utilisant la **méthode de consolidation par les flux**.

## 2. Contexte & problème

Les solutions actuelles sont soit professionnelles et coûteuses, soit s'appuient largement sur Excel. Objectif : un outil **facilement déployable, rapide, facile à maintenir**.

## 3. Périmètre fonctionnel

### 3.1 Types de consolidation couverts
- [x] Légale (IFRS / French GAAP / US GAAP…)
- [x] Budgétaire / prévisionnelle
- [x] De gestion (KPI internes)
- [x] Multi-phases (réel / budget / prévision)

> MVP : **réel seul**. Budget / prévision / multi-phases en post-MVP.

### 3.2 Structure du groupe
- Définie par un **périmètre de consolidation** (scope) : mère + entités, chacune avec sa méthode (globale, proportionnelle, équivalence, cas spéciaux type IFRS 5).
- Calcul des **intérêts minoritaires**.
- Gestion des **entrées / sorties / fusions**, en cours d'exercice ou en début de période.

> Périmètre = table satellite **versionnée** (`sat_perimeter`, clé `perimeter_set × entity × period`) ; méthode + % par entité, réutilisable entre consolidations ([Q5](./docs/QUESTIONS_OUVERTES.md), [Q35](./docs/QUESTIONS_OUVERTES.md) TRANCHÉES).

### 3.3 Référentiels & plans de compte
- Plan de compte **customisable**.
- Saisie **directement dans le plan du groupe** (pas de mapping — option d'évolution).
- Conversion de devises : **taux clôture / moyen selon le schéma de flux** du compte (bilan vs résultat), via une **devise pivot** applicative.

> Taux : clôture + moyen par `(jeu de taux, devise, période)`, CRUD + import CSV ([Q4](./docs/QUESTIONS_OUVERTES.md) TRANCHÉE). Taux par flux piloté par le **schéma de flux** ([Q32](./docs/QUESTIONS_OUVERTES.md) TRANCHÉE).

### 3.4 Opérations de consolidation

Deux natures de traitements (la dichotomie B/C est abandonnée) :

- **Natifs** : implémentés dans le moteur, non paramétrables.
- **Construits via l'éditeur de règles** : l'utilisateur compose lui-même les écritures automatiques paramétrables (**implémenté dans le prototype** — moteur + API + UI ; voir [Q24](./docs/QUESTIONS_OUVERTES.md) TRANCHÉE).

**Traitements natifs — MVP**
- Agrégation / cumul des comptes
- Conversion multi-devises — taux par **schéma de flux** du compte (bilan : taux du flux + écarts F80/F81 ; résultat : taux moyen sans écart), voir [Q32](./docs/QUESTIONS_OUVERTES.md)
- Gestion des méthodes de consolidation **pilotables** (`dim_method`, sans liste en dur) : intégration **globale**, **proportionnelle** (application native — la mise en **équivalence** et le calcul des **intérêts minoritaires** sont reportés en post-MVP, voir [Q26](./docs/QUESTIONS_OUVERTES.md), [Q33](./docs/QUESTIONS_OUVERTES.md) et l'éditeur de règles [Q24](./docs/QUESTIONS_OUVERTES.md))
- Variations de périmètre : entrées / sorties (par comparaison au scope de la consolidation d'ouverture)

**Traitements natifs — extensions post-MVP**
- **Mise en équivalence** (capitaux propres au `% d'intégration`, contrepartie sur compte d'actif paramétrable, P&L condensé) — voir [`docs/FLUX_CONSO.md`](./docs/FLUX_CONSO.md) §9
- Fusions, entrées / sorties en cours d'exercice
- IFRS 5 (held-for-sale / discontinued operations)

**Éditeur de règles de consolidation** ([Q24](./docs/QUESTIONS_OUVERTES.md) TRANCHÉE — implémenté)
Écritures automatiques paramétrables par l'utilisateur. Premières règles prévues : éliminations interco (ventes, créances/dettes, marges en stock, dividendes) et éliminations des participations ; puis **intérêts minoritaires**, retraitements, variations de capital, répartition des résultats. Spécification détaillée dans [`docs/REGLES_CONSO.md`](./docs/REGLES_CONSO.md).

**Moteur de formules** ([Q43](./docs/QUESTIONS_OUVERTES.md) — implémenté) : créateur de formules type Excel, branché sur deux usages. (1) **Coefficients de règles** : la liste fermée des coefficients (`pct_integration`…) devient des formules éditables par l'utilisateur. (2) **Indicateurs / KPI** : mesures calculées combinant des « postes » (sélections agrégées) à un grain. Spécification : [`docs/FORMULES.md`](./docs/FORMULES.md).

### 3.5 Process & workflow
- Saisie par **chargement de fichier** (format §4) + écritures manuelles par-dessus.
- Calendrier : clôture **mensuelle / trimestrielle / annuelle**, prévisionnelle multi-années possible.
- Validation par étapes : statuts **brouillon / soumis** pour liasses et écritures (évolution possible).
- Deux modes : consolidation **complète** ou **à la marge**.

> Périmètre du POC (mode, workflow, granularité) : voir [Q6](./docs/QUESTIONS_OUVERTES.md), [Q8](./docs/QUESTIONS_OUVERTES.md), [Q9](./docs/QUESTIONS_OUVERTES.md).

## 4. Sources de données

- Format d'échange : **CSV** (prototype ; évolutif ensuite — voir [Q18](./docs/QUESTIONS_OUVERTES.md)).
- Champs en entrée (ordre et casse à respecter) :

  `Scenario, Entity, Entry_period, Period, Account, Flow, Currency, Nature, Partner*, Share*, Analysis*, Analysis2*, Source*, Amount`

  `*` = champ optionnel. Tout autre champ est obligatoire. (`Audit_id` a été supprimé : son rôle de traçabilité est tenu par `Source` — métadonnée non-dimensionnelle — et l'ancien axe a été renommé `Analysis2`. Voir [Q13](./docs/QUESTIONS_OUVERTES.md).)
- Sémantique détaillée des champs et master data de chaque dimension : voir [`docs/MODELE_DONNEES.md`](./docs/MODELE_DONNEES.md).

## 5. Restitution & reporting

**Sorties du POC** (format : **web interactif**) :
- Table consolidée filtrable (toutes les lignes, filtres sur tous les champs dont **nature**).
- **Bilan par flux** : comptes en lignes, flux en colonnes (`solde_ouverture` / `variation` / `solde_clôture`), filtrable par **nature**.
- **Compte de résultat** : flux d'ouverture et flux de clôture, filtrable par **nature**.
- **Indicateurs / KPI** : mesures calculées (marge, ratios…) via un **créateur de formules** type Excel, affichées à un grain choisi (voir [`docs/FORMULES.md`](./docs/FORMULES.md)).

**À terme** (post-POC) : bilan consolidé mis en forme, tableau de flux de trésorerie, annexe / notes, dashboards analytiques.

## 6. Exigences non-fonctionnelles

| Domaine | État | Réf. |
|---|---|---|
| Performance | Critère de validation du POC (test sur gros volumes) ; temps cible exact à préciser | [Q12](./docs/QUESTIONS_OUVERTES.md) |
| Volumétrie | **Large** : 50+ entités, milliers de comptes, millions de lignes — performance testée sur gros volumes | [Q3](./docs/QUESTIONS_OUVERTES.md) |
| Sécurité | Ignoré initialement | [Q15](./docs/QUESTIONS_OUVERTES.md) |
| Audit / traçabilité | Provenance tracée par le champ `Source` (ex-`Audit_id`, non-dimensionnel) | [Q13](./docs/QUESTIONS_OUVERTES.md) (TRANCHÉE) |
| Évolutivité | Ajout filiale / référentiel / module sans refonte | [Q14](./docs/QUESTIONS_OUVERTES.md) |

## 7. Contraintes & préférences techniques

- **Stack** (détaillée dans [`docs/TECHNIQUE.md`](./docs/TECHNIQUE.md)) : moteur en **Rust** (logique de conso) + **DuckDB** embarqué (stockage analytique) + serveur web **Axum** (Rust, API JSON) + frontend **React / Vite / TanStack Table** (TypeScript).
- **Hébergement** : local, accessible via une page web.
- **Licence** : privée pour l'instant — voir [Q16](./docs/QUESTIONS_OUVERTES.md).

## 8. Glossaire

- **Consolidation** : agrégation des comptes des entités d'un groupe en comptes uniques.
- **Périmètre de consolidation** : ensemble des entités incluses.
- **Interco** : opérations entre entités du groupe, à éliminer.
- **Retraitement** : ajustement pour homogénéiser la comptabilisation.
- **Minoritaires / intérêts hors groupe** : part des actionnaires non contrôlants.

---

## Documents liés

- [`docs/QUESTIONS_OUVERTES.md`](./docs/QUESTIONS_OUVERTES.md) — registre des décisions à prendre (priorisé par impact sur le POC).
- *À venir* : annexe modèle de données détaillé, annexe règles de consolidation C, etc. — créées au fur et à mesure.

## MVP / POC — périmètre défini

> **État d'implémentation détaillé** (fait / partiel / reste, et comportements) :
> [`docs/ETAT_AVANCEMENT.md`](./docs/ETAT_AVANCEMENT.md). La section ci-dessous fixe le **scope**.

**Dans le MVP**
- Scénario : **réel seul** (objet composite : période, devise, variante, jeu de taux, jeu de périmètre, ruleset, à-nouveau).
- Traitements **natifs** : agrégation, conversion multi-devises (taux par **schéma de flux** du compte), méthodes de conso **pilotables** (globale, proportionnelle), **report d'à-nouveau** (F99 N-1 → F00 N). Les **variations de périmètre** (entrées/sorties) passent désormais par des **règles** (suppression du niveau natif *reclassifié*). Mise en équivalence et intérêts minoritaires reportés (post-MVP).
- **Éditeur de règles de consolidation** ([Q24](./docs/QUESTIONS_OUVERTES.md) TRANCHÉE, implémenté par anticipation) : éliminations interco et participations. Spécification dans [`docs/REGLES_CONSO.md`](./docs/REGLES_CONSO.md). Reste post-MVP : intérêts minoritaires, retraitements, variations de capital, répartition des résultats (catalogue §10).
- **Moteur de formules** ([Q43](./docs/QUESTIONS_OUVERTES.md), implémenté) : coefficients de règles éditables + indicateurs / KPI ([`docs/FORMULES.md`](./docs/FORMULES.md)).
- Restitutions : table filtrable, **bilan par flux**, **compte de résultat**, **indicateurs/KPI** (§5).
- Master data : **CRUD complet** pour chaque dimension et table satellite + import CSV (liasses + taux) (§3.4, [`docs/MODELE_DONNEES.md`](./docs/MODELE_DONNEES.md)).
- Volumétrie : **large** (50+ entités, millions de lignes) — la performance est un critère de validation.

**Reporté (post-MVP)**
- Extensions natives : fusions, entrées/sorties en cours d'exercice, IFRS 5.
- Règles avancées : intérêts minoritaires, retraitements, variations de capital, répartition des résultats (cf. catalogue [`docs/REGLES_CONSO.md`](./docs/REGLES_CONSO.md) §10).
- Multi-phases (budget, prévision), TFT, annexe, dashboards.
- **Évolutions architecturales futures** (analyse dans [`docs/QUESTIONS_OUVERTES.md`](./docs/QUESTIONS_OUVERTES.md) §*Évolutions futures*) : dimensions dépendantes du temps [Q47], consolidation à la marge + locking [Q48], consolidation temps réel déclenchée à l'intégration [Q49], calcul hiérarchique sur les dimensions [Q50].
- **Formulaires configurables** (cahiers de saisie & restitution, pivot tables) [Q51].
- **Gestion des droits d'accès** (rôles, scopes, packages) [Q52].
- **Environnements multiples** (Dev / Intégration / Production, promotion paramétrage + données) [Q53].
- **Accessibilité API pour agents IA** (MCP, bulk master data, pagination, recherche) [Q54].

**Encore à trancher (TÔT) avant la 1ʳᵒ implémentation** : [Q6](./docs/QUESTIONS_OUVERTES.md) (mode complète/marge), [Q8](./docs/QUESTIONS_OUVERTES.md) (workflow validation), [Q9](./docs/QUESTIONS_OUVERTES.md) (granularité de clôture), [Q10](./docs/QUESTIONS_OUVERTES.md) (détection interco — utile au post-MVP), [Q12](./docs/QUESTIONS_OUVERTES.md) (cible de perf).
