# Flux de consolidation

Annexe de [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) (dimension `Flow`).

La consolidation est **par les flux** : chaque traitement de consolidation agit sur des **flux de variation** et génère des écritures taguées avec un code de flux. Le code de flux explicite l'origine de chaque montant → **traçabilité totale**. Le flux de clôture (F99) n'est jamais saisi : c'est un solde **reconstruit** par identité.

---

## Niveaux d'élaboration de la consolidation

La consolidation s'élabore en **3 niveaux successifs**, chacun stocké. Ces niveaux sont les **étapes du pipeline** du moteur.

| Niveau | Rôle | Traitements |
|---|---|---|
| **1. Corporate** | Agrège les données saisies (par entité, en devise fonctionnelle) | Agrégation des écritures sources |
| **2. Converti** | Application des règles de conversion | Conversion multi-devises (F80/F81) ; reclassifications de périmètre/fusion (F01/F07/F70/F98) |
| **3. Consolidé** | Application des mécanismes de consolidation | Méthodes (globale / proportionnelle / équivalence) ; éditeur de règles (interco, participations, minoritaires…) |

- Les reclassifications de périmètre (F01/F07/F70/F98) s'appliquent **avant ou juste après la conversion**, mais leur **résultat est stocké au niveau converti**. Position exacte à préciser ([Q26](./QUESTIONS_OUVERTES.md)).
- L'**éditeur de règles** (post-MVP) interviendra surtout sur les niveaux **converti** et **consolidé** (ex. élimination interco classique au niveau *converti*). À reprendre plus tard ([Q24](./QUESTIONS_OUVERTES.md)).

---

## 1. Modèle des flux (master data `Flow`)

Table créable/éditable via CRUD. Attributs de chaque flux :

| Attribut | Rôle |
|---|---|
| `code`, `libellé` | Identification (F00, F20, F99…) |
| `taux_conversion` | Type de taux à appliquer pour la conversion (clôture N, clôture N-1, moyen…). Référence aux taux de change. |
| `flux_de_report` | Flux dans lequel celui-ci s'agrège lors de la reconstruction de la clôture (défaut : **F99**). **Tous les flux reportent à F99**, y compris les écarts. |
| `flux_ecart_conversion` | Flux d'écart qui recevra la différence de conversion de ce flux. **Null pour les écarts eux-mêmes** (terminaux : leur `taux_conversion` = clôture → écart propre = 0). |

## 2. Mécanique de conversion

Tous les flux sont saisis en **devise fonctionnelle** et convertis via leur `taux_conversion`. Pour un flux X (montant `A_X` en devise fonctionnelle, taux `r_X`) :

- Montant converti = `A_X × r_X`
- **Écart de conversion** = `A_X × (r_clôture − r_X)`, posté sur le `flux_ecart_conversion` de X
- Les flux d'écart reportent à F99 comme les autres ; en **devise fonctionnelle ils valent 0** (la conversion n'existe pas en fonctionnel), en **devise de présentation ils valent l'écart calculé** du flux parent. Leur `taux_conversion` = clôture → leur propre écart = 0 (pas de récursion).

**Cas particuliers** (qui retrouvent F80 / F81) :

| Flux | Taux | Écart = `A × (r_clôture − r_flux)` |
|---|---|---|
| F00 (ouverture) | clôture **N-1** | `A × (r_clôture_N − r_clôture_{N-1})` → posté sur **F80** |
| F20 (variation) | **moyen** | `A × (r_clôture − r_moyen)` → posté sur **F81** |
| F99 (clôture) | clôture **N** | `0` |

## 3. Identité de reconstruction (par les flux)

**Symétrique** : tient à l'identique en devise fonctionnelle et en devise de présentation.

- `F99 = Σ(tous les autres flux)` — via `flux_de_report`.
- En devise fonctionnelle : les écarts (F80/F81) y sont à 0 → `F99 = F00 + Σ variations`.
- En devise de présentation : les écarts y valent l'écart calculé → `F99_conv = F00_conv + Σ variations_conv + Σ écarts`.

## 4. À-nouveau

À la clôture, **F99 (clôture N) se reporte sur F00 (ouverture N+1)**.

## 5. Principe « consolidation par les flux »

Les **automatismes de consolidation** (conversion, méthodes, variations de périmètre, et plus tard l'éditeur de règles) agissent sur les **flux de variation** (F20, F01, F07, F95, F98…), **jamais sur F99**. F99 est un solde reconstruit, pas un solde saisi.

---

## 6. Catalogue des flux

| Code | Libellé | Taux conversion | Écart → | Généré par | MVP |
|---|---|---|---|---|---|
| **F00** | Ouverture | clôture N-1 | F80 | Report d'ouverture (à-nouveau de F99 N-1) | MVP |
| **F01** | Entrée de consolidation | *(à définir)* | *(à définir)* | Variation de périmètre — entrée | MVP |
| **F07** | Fusion à l'ouverture | *(à préciser)* | *(à préciser)* | Extourne F00 (`F07 = −F00`) | post-MVP |
| **F70** | Fusion en cours d'exercice | *(à préciser)* | *(à préciser)* | Extourne F99 (`F70 = −F99`) | post-MVP |
| **F20** | Variation de bilan | moyen | F81 | Saisie source / agrégation | MVP |
| **F80** | Écart de conversion (ouverture → clôture) | clôture N (terminal) | — | Conversion (écart de F00) | MVP |
| **F81** | Conversion taux moyen → clôture | clôture N (terminal) | — | Conversion (écart de F20) | MVP |
| **F95** | Variation de taux d'intérêt | *(à définir)* | *(à définir)* | Règles de consolidation (éditeur) | post-MVP |
| **F98** | Sortie de périmètre | *(à définir)* | *(à définir)* | Variation de périmètre — sortie | MVP |
| **F99** | Clôture | clôture N | — (0) | Reconstruction par identité | MVP |

> Cases « à définir » : taux et écart des flux de périmètre (F01/F98) — F01 suit la logique ouverture (clôture N-1 ?), F98 suit la logique clôture (clôture N ?). À confirmer. Mécanique de reclassification détaillée §9.

## 7. Logique de numérotation

- **F0x** — ouverture, périmètre d'entrée
- **F2x** — variations de la période
- **F7x** — fusion (F07 à l'ouverture, F70 en cours d'exercice)
- **F8x** — écarts de conversion
- **F9x** — variation de %, sortie, clôture

## 8. Restitution « Bilan par flux »

Comptes en lignes × flux en colonnes. Par construction, la colonne F99 = F00 + Σ(variations) + Σ(écarts) — l'identité de reconstruction rendue visible.

---

## 9. Traitements de consolidation par flux

Les variations de périmètre et la fusion se traduisent par des **reclassifications de flux** au niveau consolidé, afin de préserver la continuité **`F00 consolidé = F99 consolidé de N-1`** (le périmètre existant se reporte sans être pollué par les entrées/sorties).

### Entrée de périmètre → F01
Une entité qui entre a, au niveau **social**, un montant sur F00 (ouverture). En consolidation, **ce F00 est déplacé vers F01**. Ainsi le F00 consolidé ne contient que le report du périmètre existant (= F99 consolidé N-1) ; les ouvertures des entités entrantes sont isolées en F01.

### Sortie de périmètre → F98
Symétriquement, une entité qui sort a son **F99 social déplacé vers F98** au niveau consolidé. Le F99 consolidé exclut donc l'entité sortante ; son solde est isolé en F98.

### Fusion → F07 / F70 (post-MVP)
Entité **absorbée**. Deux modes selon le moment de la fusion ; dans les deux cas l'absorption est **saisie manuellement par l'entité absorbante** (pas d'automatisme).

- **F07 — fusion à l'ouverture** : `F07 = −F00`, F00 inchangé → `F99 = 0`. Fusion effective en début d'exercice (ou rétrospective au début) : pas d'activité à isoler.
- **F70 — fusion en cours d'exercice** : `F70 = −F99` (extourne du solde à la date de fusion) → permet de prise en compte une activité antérieure à la fusion au cours de la période.

> **À trancher** (post-MVP) : quel mode privilégier, ou garder les deux disponibles.

### F95 — Variation de taux d'intérêt (post-MVP)
Calculée par les **règles de consolidation** (éditeur de règles). Pas de particularité à ce stade.

### F20 — Variation standard
Flux standard de variation (mouvements saisis en source). Aucune spécificité de consolidation : simplement agrégé.

### Application des méthodes de consolidation (natif MVP)

La méthode de l'entité (issue du *Périmètre*) détermine comment ses flux sont agrégés :

- **Intégration globale** : agrégation des flux à **100%**.
- **Intégration proportionnelle** : agrégation des flux au **`% d'intégration`** (la globale en est le cas particulier `% = 100%`). Pas de flux minoritaire : la part non détenue n'est pas consolidée.
- **Mise en équivalence** : les flux hors capitaux propres ne sont **pas agrégés**.
  - Les **comptes de capitaux propres** (identifiés par un flag sur le compte) sont consolidés au `% d'intégration`.
  - **Contrepartie** postée sur un **compte d'actif paramétrable** (ex. `261E`).
  - Le **P&L est condensé** sur un **compte paramétrable** (ex. `880E`), au `% d'intégration`.
  - Comptes paramétrables renseignés via une **page de paramètres** du groupe.

### Intérêts minoritaires (règles — post-MVP)
Calculés par les règles de consolidation, comme la **différence `% d'intégration − % d'intérêt`** (les deux taux sont portés par la table *Périmètre*). S'applique sous globale, proportionnelle et équivalence.
