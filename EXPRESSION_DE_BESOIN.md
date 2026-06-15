# Expression de besoin — Outil de consolidation financière

> *Document de travail — ébauche à retravailler.*
> Objectif : cadrer le projet avant toute conception technique.

---

## 1. Vision

**Une phrase.** À quoi sert l'outil, pour qui, quelle douleur il résout.

*Exemple : « Un outil de consolidation multi-entités, rapide et lisible, qui passe de l'ERP au reporting consolidé en quelques clics, sans tableurs. »*

---

## 2. Contexte & problème

- Comment la consolidation est-elle faite **aujourd'hui** ? (Excel, SAP, Sage, autre…)
- Qu'est-ce qui coince ? (temps, erreurs, manuel, pas de piste d'audit, etc.)
- Pourquoi maintenant ? (croissance, nouvelles filiales, contrainte réglementaire…)

---

## 3. Périmètre fonctionnel — questions structurantes

### 3.1 Type de consolidation
- [ ] **Légale** (référentiel imposé : IFRS / French GAAP / US GAAP…)
- [ ] **Budgétaire / prévisionnelle**
- [ ] **De gestion** (KPI internes, non réglementaire)
- [ ] Multi-scénarios (réel vs budget vs prévious)

### 3.2 Structure du groupe
- Combien d'entités à consolider ? Fourchette acceptable.
- Hiérarchie simple (holding + filles) ou complexe (multi-niveaux, holdings intermédiaires) ?
- Intérêts minoritaires ? Co-entreprises ? Mises en équivalence ?
- Périmètre variable (entrées/sorties de périmètre dans l'année) ?

### 3.3 Référentiels & plans de compte
- Un référentiel cible unique ou plusieurs (IFRS + local) ?
- Mapping entre plans de compte source (par entité) et le référentiel consolidé.
- Méthode de conversion : réintégrations / retraites homogènes ou spécifiques par entité ?

### 3.4 Opérations de consolidation
- [ ] Cumul des comptes
- [ ] Élimination des opérations inter-compagnies (ventes, créances/dettes, marges en stock, dividendes)
- [ ] Conversion multi-devises (méthode clôture / moyennes pondérées)
- [ ] Retraitements (homogénéisation, amortissement survaleurs, écarts d'acquisition)
- [ ] Répartition des résultats (minoritaires / groupe)
- [ ] Variations de capital & de périmètre

### 3.5 Process & workflow
- Qui saisit ? Qui valide ? (DAF, comptables, chefs de filiale…)
- Calendrier : clôture mensuelle / trimestrielle / annuelle ?
- Jeu d'écritures : datas reçues à quelle date butoir dans le mois ?
- Validation par étapes (status : brouillon / soumis / validé / publié) ?

---

## 4. Sources de données

- Liste des ERP / systèmes actuels (SAP, Sage, Cegid, EBP, fichier Excel maison…)
- Format d'échange attendu : CSV / XLSX / API / connecteur direct ?
- Un seul format ou un par entité ?
- Volumétrie : combien de lignes d'écritures par clôture ?

---

## 5. Restitution & reporting

- Sorties attendues :
  - [ ] Bilan consolidé
  - [ ] Compte de résultat consolidé
  - [ ] Tableau de flux de trésorerie
  - [ ] Annexe / notes
  - [ ] Tableaux de bord analytiques
- Format : PDF, Excel, web interactive, export API ?
- Niveaux de détail par audience (Codir / DAF / filiale / auditeurs).

---

## 6. Exigences non-fonctionnelles

| Domaine | Question |
|---|---|
| **Performance** | Temps cible entre données reçues et reporting dispo ? |
| **Volumétrie** | Nb d'entités, de comptes, de lignes, d'années conservées ? |
| **Sécurité** | Qui voit quoi ? (par entité, par périmètre) RBAC ? |
| **Audit / traçabilité** | Chaque chiffre doit pouvoir être tracé jusqu'à sa source ? |
| **Disponibilité** | Saas cloud, on-premise, hybride ? |
| **Multi-utilisateurs** | Combien en simultané ? Rôles distincts ? |
| **Conformité** | RGPD, souveraineté des données, archivage légal ? |
| **Évolutivité**| Ajouter une filiale / un référentiel / un module doit être possible sans refonte ? |

---

## 7. Contraintes & préférences techniques

- Stack souhaitée ou à éviter ? (ex : Python, TS, Go…)
- Hébergement : cloud public / privé / on-prem ?
- Préférence pour l'open source vs licence propriétaire ?
- Budget cible (dev initial, TCO annuel) ?
- Délai / jalon business impératif (clôture annuelle 2026 ?) ?

---

## 8. Acteurs & governance

| Rôle | Qui | Intérêt |
|---|---|---|
| Sponsor | … | Décide / finance |
| DAF | … | Utilise au quotidien |
| Experts métier | … | Valident les règles de consolidation |
| Utilisateurs filiales | … | Saisissent / valident leurs données |
| IT / DevOps | … | Déploient / maintiennent |
| Auditeurs externes | … | Vérifient |

---

## 9. Critères de réussite (KPI)

- Temps de clôture passé de X jours à Y.
- Taux d'erreur manuelle réduit de X %.
- Adoption : % d'entités saisissant dans l'outil vs Excel.
- Toutes les entités consolidées en 1 clic ?

---

## 10. Hypothèses, risques & points ouverts

- **Hypothèses** : …
- **Risques** : qualité des données sources, adhésion des filiales, complexité des règles métier…
- **Points ouverts** : tout ce qui n'est pas encore tranché.

---

## 11. Glossaire (court)

- **Consolidation** : agrégation des comptes des entités d'un groupe en comptes uniques.
- **Périmètre de consolidation** : ensemble des entités incluses.
- **Interco** : opérations entre entités du groupe, à éliminer.
- **Retraitement** : ajustement pour homogénéiser la comptabilisation.
- **Minoritaires / intérêts hors groupe** : part des actionnaires non contrôlants.

---

## Prochaines étapes

- [ ] Compléter les sections 1 → 10 (réponses précises)
- [ ] Prioriser : MVP = quel sous-ensemble livrable en premier ?
- [ ] Décider : build vs buy vs open source à adapter
- [ ] Définir un prototype / POC mesurable
