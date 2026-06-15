# Analyse : ordre des reclassifications vs conversion (Q26)

> **Conclusion** : reclassification de périmètre **AVANT** conversion.
> Pipeline : `Corporate → Reclassification → Conversion → Consolidé` (4 niveaux au lieu de 3).

Simulation : [`simulations/consolidation_sim.py`](../simulations/consolidation_sim.py)

## Contexte

Q26 (FLUX_CONSO.md §9, lignes 19 et 100) posait la question : les reclassifications de périmètre (F01/F07/F70/F98) s'appliquent-elles **avant ou après** la conversion multi-devises ?

## Résultat de la simulation

Deux approches testées sur un groupe de 3 entités (Mère EUR + Filiale A USD entrante + Filiale B GBP sortante) :

| Approche | Pipeline | Résultat numérique |
|---|---|---|
| **A — Reclassification avant conversion** | Corporate → Reclassification (fonctionnel) → Conversion → Consolidé | ✓ Identité vérifiée |
| **B — Conversion avant reclassification** | Corporate → Conversion → Reclassification (EUR) → Consolidé | ✓ Identité vérifiée |

**Les totaux consolidés (compte × flux) sont identiques au centime près.**

Mais l'approche A est structurellement supérieure sur trois points.

## Pourquoi AVANT (Approche A)

### 1. Traçabilité — le problème des écarts orphelins

**Entrée de périmètre (F00 → F01)** :
- **Avant** : F01 hérite de F00 en devise fonctionnelle. L'écart F80 est généré contre F01. Chaîne : F00 social → F01 consolidé → F80.
- **Après** : F80 est calculé contre F00, puis F00 est reclassifié en F01. **F80 devient orphelin** — son flux parent n'existe plus.

### 2. Sortie de périmètre — le cas qui tue

**Sortie (F00 + F20 → F98)** :
- **Avant** : collapse en devise fonctionnelle → une écriture F98 propre → conversion au taux clôture (terminal, zéro écart).
- **Après** : la conversion a déjà éclaté F00 en F00\_conv + F80 et F20 en F20\_conv + F81. Pour collapsier vers F98, il faut **absorber les écarts** → détail perdu, F98 devient un fourre-tout. Ou bien on garde les écarts → orphelins.

### 3. Simplicité d'implémentation

- **Avant** : la reclassification est un simple relabeling de flux en devise fonctionnelle. Le moteur de conversion fonctionne ensuite uniformément.
- **Après** : le moteur doit reclassifier en devise de présentation ET gérer des écarts orphelins. Complexité inutile.

## Décision

Le pipeline devient **4 niveaux** :

| Niveau | Rôle | Traitements |
|---|---|---|
| **1. Corporate** | Agrège les données saisies (par entité, en devise fonctionnelle) | Agrégation des écritures source |
| **2. Reclassifié** | Reclassifications de périmètre (en devise fonctionnelle) | Entrées (F00→F01), sorties (collapse→F98), fusions (F07/F70 post-MVP) |
| **3. Converti** | Conversion multi-devises | F80/F81 générés sur les flux reclassifiés |
| **4. Consolidé** | Mécanismes de consolidation | Méthodes (globale / proportionnelle / équivalence) ; éditeur de règles (post-MVP) |
