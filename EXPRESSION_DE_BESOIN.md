# Expression de besoin — Outil de consolidation financière

> *Document de travail — ébauche à retravailler.*
> Objectif : cadrer le projet avant toute conception technique.

---

## 1. Vision

** Un outil de consolidation multi-entités, multi devises, rapide, utilisant la méthode de consolidation par les flux.**

---

## 2. Contexte & problème

Les solutions de consolidation actuelle sont soit professionelles eet nécessitent un gros budget, soit doivent largement s'appuyer sur excel. On veut ici créer un outil facilement déployable, rapide et facile à maintenir.

---

## 3. Périmètre fonctionnel — questions structurantes

### 3.1 Type de consolidation
- [x] **Légale** (référentiel imposé : IFRS / French GAAP / US GAAP…)
- [x] **Budgétaire / prévisionnelle**
- [x] **De gestion** (KPI internes, non réglementaire)
- [x] Multi-scénarios (réel vs budget vs prévious)

### 3.2 Structure du groupe
- La structure du group est donné par une périmètre de consolidation (scope)
- Il contient la mère et les différentes entités, avec leurs méthodes de consolidation spécifiques
- Le calcul des intérêts minoritaires sera fait. Les méthodes sont globale, proportionnelles, équivalence, et traitements spéfiaux tels que IFRS 5 (Held for sale, discountinued operations)
- Gestion des entrées, sorties, fusions, en cours d'exercice ou en début de période

### 3.3 Référentiels & plans de compte
- Référentiel de comptes customisable.
- Dans cette version, on estime que l'entité saisie une liasse dans le plan de compte du groupe, pas de mapping [option d'évolution]
- Méthode de conversion : sur le principe de taux moyens appliqué sur la période

### 3.4 Opérations de consolidation
- [B] Cumul des comptes
- [B] Conversion multi-devises (méthode clôture / moyennes pondérées)
- [B] Variations de périmètre
- [C] Variations de capital
- [C] Répartition des résultats (minoritaires / groupe)
- [C] Retraitements (homogénéisation, amortissement survaleurs, écarts d'acquisition)
- [C] Élimination des opérations inter-compagnies (ventes, créances/dettes, marges en stock, dividendes)
Les opérations notées B (basic) seront traitées nativement par l'outil
Les opérations notées C (custom) devront être des écritures automatiques qui seront paramétrisées par l'utilisateur [section à prévoir]

### 3.5 Process & workflow
- Saisie par chargement de fichier [à retravaillé plus tard]
- Calendrier : possibilité de clôture mensuelle / trimestrielle / annuelle / prévisionnelles multi-year
- Jeu d'écritures : possiblité de passer des écritures manuelles par dessus l'import des liasses
- Validation par étapes : status des liasses brouillon / soumis ; les écritures suivent également ces status [possibilité d'évolution postérieure]
- Deux façon de consolider, soit complete, soit à la marge lorsqu'une nouvelle liasse ou écriture est soumise

---

## 4. Sources de données

- Format d'échange attendu : CSV [pour le prototype, sera un point à faire évoluer postérieurement]
- Champs des données en entrée: Scenario, Entity, Entry_period, Period, Account, Flow, Currency, Audit_id, Partner*, Share*, Analysis*, Amount (les champs optionnels sont suivi de l'asterix *)


---

## 5. Restitution & reporting

- Sorties attendues :
  - [ ] Bilan consolidé
  - [ ] Compte de résultat consolidé
  - [ ] Tableau de flux de trésorerie
  - [ ] Annexe / notes
  - [ ] Tableaux de bord analytiques
  Ces sorties seront revues plus tard, initialement:
  - Restitutions montrant toutes les lignes de la base, filtrable selon tous les champs
- Format : web interactive [autres formats pour extention future]

---

## 6. Exigences non-fonctionnelles

| Domaine | Question |
|---|---|
| **Performance** | Temps cible entre données reçues et reporting dispo ? |
| **Volumétrie** | Nb d'entités, de comptes, de lignes, d'années conservées ? |
| **Sécurité** | Ignoré initialement ; à completer une fois le prototy pfait
| **Audit / traçabilité** | Chaque opération doit être traçée: la saisie initiale dans la liasse et les écriture, manuelle ou automatiques sont traçées par une référence, à détailler. |
| **Évolutivité**| Ajouter une filiale / un référentiel / un module doit être possible sans refonte |

---

## 7. Contraintes & préférences techniques

- Stack souhaitée: moteur de consolidation en Rust, accessible via navigateur. Le reste est à proposer par le développeur
- Hébergement : local, accessible via une page internet
- Licence : pour le moment purement privé ; à revoir si le programme doit être rendu public

---

## 8. Glossaire (court)

- **Consolidation** : agrégation des comptes des entités d'un groupe en comptes uniques.
- **Périmètre de consolidation** : ensemble des entités incluses.
- **Interco** : opérations entre entités du groupe, à éliminer.
- **Retraitement** : ajustement pour homogénéiser la comptabilisation.
- **Minoritaires / intérêts hors groupe** : part des actionnaires non contrôlants.

---

## Prochaines étapes

- [ ] Prioriser : MVP = quel sous-ensemble livrable en premier ?
- [ ] Définir un prototype / POC mesurable
