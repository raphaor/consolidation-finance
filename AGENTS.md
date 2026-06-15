# AGENTS.md

## État du projet

Projet en **phase d'expression de besoins**. Aucun code, aucune stack installée, aucun test. Toute modification doit d'abord s'inscrire dans le cadre décrit par [`EXPRESSION_DE_BESOIN.md`](./EXPRESSION_DE_BESOIN.md), qui est la source de vérité fonctionnelle.

Langue de travail : **français** (docs, termes métier, commits). Conserver ce registre.

## Structure documentaire

- `EXPRESSION_DE_BESOIN.md` — doc principal, **court par intention**. N'y ajoutez pas de détails diluants.
- `docs/QUESTIONS_OUVERTES.md` — **registre des décisions à prendre**, priorisé par impact sur le POC (`BLOC` / `TÔT` / `POST` / `HORS`). Tout point non tranché va ici, pas dans le doc principal. Chaque question a un ID (`Qn`) référencé depuis l'EDB via un lien.
- `docs/MODELE_DONNEES.md` — **annexe modèle de données** : sémantique des champs CSV, catalogue des dimensions (attributs master data + traitements conso liés), tables satellites (Périmètre, Taux de change, Participations).
- `docs/TECHNIQUE.md` — **annexe stack technique** : architecture (engine/server/web), justifications des choix (Rust, DuckDB, Axum, React/Vite/TanStack Table).
- `docs/FLUX_CONSO.md` — **catalogue des flux de consolidation** (F00–F99) : sémantique et traitement générateur de chaque code de flux.
- Convention de travail : quand une exigence reste ouverte, **créer/éditer une entrée dans le registre** plutôt que d'éparpiller des `?` dans le texte. À l'inverse, quand une question passe à `TRANCHÉE`, **reporter la décision dans l'EDB** et conserver la ligne (historique).

## Stack (décidée)

- **Moteur de consolidation en Rust** (crate `engine`) : logique métier native (agrégation, conversion, méthodes, variations de périmètre).
- **Stockage : DuckDB embarqué** (analytique columnar, fichier local) — choisi pour la perf sur gros volumes.
- **Serveur web : Axum** (Rust, crate `server`) — API JSON + sert le frontend statique.
- **Frontend : React + Vite + TanStack Table** (TypeScript, npm).
- Hébergement **local**, mono-utilisateur (prototype). Pas de SaaS.
- Détails et justifications : [`docs/TECHNIQUE.md`](./docs/TECHNIQUE.md).

## Modèle de données — prototype

Format d'échange : **CSV** (pour le prototype uniquement, évolutif ensuite).

Champs en entrée (respecter l'ordre et la casse) :
`Scenario, Entity, Entry_period, Period, Account, Flow, Currency, Audit_id, Partner*, Share*, Analysis*, Amount`

Champs marqués `*` sont **optionnels**. Tout autre champ est obligatoire.

## Sémantique métier essentielle

- Méthode de consolidation : **par les flux** — chaque traitement génère des écritures taguées par un code de flux (`Flow`). Catalogue dans [`docs/FLUX_CONSO.md`](./docs/FLUX_CONSO.md) (F00 ouverture, F20 variation, F80/F81 conversion, F01/F98 périmètre, F99 clôture).
- Deux natures de traitements (la dichotomie B/C est abandonnée) :
  - **Natifs** (moteur) : agrégation, conversion multi-devises, méthodes de consolidation (globale / proportionnelle / équivalence), variations de périmètre.
  - **Construits via l'éditeur de règles** (module **post-MVP**) : écritures automatiques paramétrables (éliminations interco et participations, retraitements, variations de capital, répartition des résultats).
- L'utilisateur saisit les liasses **directement dans le plan de compte du groupe** (pas de mapping prévu dans cette version).
- Conversion de devises : **taux clôture pour le bilan, taux moyen (simple) pour le résultat**.

Ne pas inventer de règles de consolidation : tout traitement non listé comme **natif** dans `EXPRESSION_DE_BESOIN.md` §3.4 doit passer par l'éditeur de règles (post-MVP). Ne pas le coder en dur dans le moteur.

## Conventions de travail

- Statut du document de besoins : *ébauche à retravailler*. Avant de coder, vérifier [`docs/QUESTIONS_OUVERTES.md`](./docs/QUESTIONS_OUVERTES.md) : toute question `BLOC` ou `TÔT` non tranchée doit être soumise à l'utilisateur avant implémentation.
- Style de commit observé : `docs: <sujet court>` — garder ce format préfixé.
- Priorité actuelle : **prototype / POC mesurable**, pas système complet. Volumétrie cible = **large** (50+ entités, millions de lignes) — la performance est un critère de validation. Éviter toute architecture spéculative (sécurité, multi-format) tant que non listée comme objectif immédiat.
