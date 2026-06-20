# Règles de consolidation — Éditeur d'écritures automatiques

> Annexe détaillant le module **éditeur de règles** (Q24). Ce module était annoncé post-MVP dans [`EXPRESSION_DE_BESOIN.md`](../EXPRESSION_DE_BESOIN.md) §3.4 ; le présent document en spécifie la conception fonctionnelle.

---

## 1. Objectif

Permettre à l'utilisateur de **composer des écritures automatiques** pour les traitements de consolidation non couverts par les natifs du moteur : éliminations interco, éliminations de participations, intérêts minoritaires, retraitements, variations de capital, répartition des résultats.

Chaque règle sélectionne des écritures existantes dans la base, applique un facteur, et génère de nouvelles écritures vers une destination. Les règles s'exécutent **séquentiellement** — la sortie d'une règle peut être l'entrée de la suivante.

---

## 2. Modèle d'une règle

```
RÈGLE
├── Identité : nom, numéro d'ordre (réordonnançable)
├── Scope périmètre : conditions sur sat_perimeter (cf. §3)
└── Opérations : 1 à N (cf. §4), exécutées dans l'ordre
```

Une règle = **un scope périmètre** partagé + **N opérations**. Chaque opération a sa propre sélection de grains, son propre facteur, et sa propre destination. Les opérations d'une même règle partagent le même scope.

### 2.1 Ordre d'exécution

Les règles s'enchaînent séquentiellement. Au sein d'une règle, les opérations s'exécutent dans l'ordre défini. **Les écritures générées par une règle sont visibles par les règles suivantes** (et par les opérations suivantes au sein de la même règle — à confirmer, voir [§7 Questions ouvertes](#7-questions-ouvertes)).

---

## 3. Scope périmètre

Définit **à quelles entités** la règle s'applique, par filtrage sur les attributs de `sat_perimeter` :

| Attribut | Exemples de conditions |
|-----------|----------------------|
| `methode` | = globale, = proportionnelle, IN (globale, proportionnelle) |
| `pct_interet` | > 0, = 0.5 |
| `pct_integration` | > 0, = 1.0 |
| `entree` | = true (entités entrantes) |
| `sortie` | = true (entités sortantes) |

**Scope croisé** : pour les éliminations interco, le scope peut porter sur **deux entités simultanément** — l'entité source et le partenaire. Ex : « l'entité ET son partenaire sont tous deux en méthode globale ». Nécessite un double join sur `sat_perimeter` (une fois pour `entity`, une fois pour `partner`).

**Scope sur les Titres** : lorsqu'une entité détient une participation dans une autre (dimension `Titres` / participations), le scope peut également filtrer sur les caractéristiques de cette relation — typiquement les **méthodes respectives** des deux entités liées. Ex : élimination des titres de participation quand la détentrice et la détenue sont toutes deux en méthode globale. Ce mécanisme est l'équivalent du scope croisé interco, mais via la dimension de participation (titres) plutôt que via la dimension `partner`.

**Articulation des conditions** : les conditions du scope sont combinées exclusivement par **ET** (conjonction codée en dur dans le moteur). Pour exprimer un **OU sur une même dimension**, utiliser l'opérateur `IN` (ex : `methode IN ('globale', 'proportionnelle')`). Le OU entre dimensions différentes n'est pas supporté aujourd'hui — il faut créer plusieurs règles ([Q30](./QUESTIONS_OUVERTES.md), statut OUVERTE, post-prototype).

---

## 4. Modèle d'une opération

Chaque opération a trois composantes : **Sélection → Facteur → Destination**.

### 4.1 Sélection

Cible un sous-ensemble de grains dans `fact_entry` en filtrant sur **toutes les dimensions** disponibles.

**Niveau de sélection** : la sélection se fait à un niveau de stockage donné (corporate, reclassified, converted, consolidated). Ce niveau détermine le niveau d'écriture des entrées générées (cf. §5).

**Modes de sélection** :
- **Par caractéristiques** : filtres sur les attributions dimensionnelles (ex : `account.classe = 'bilan'`, `flow = 'F99'`, `nature = '0LIASS'`, `partner IS NOT NULL`).
- **Par énumération** : liste explicite de valeurs (ex : `account IN ('100', '200', '300')`).

Les deux modes peuvent être combinés dans une même sélection.

**Grain sélectionné** : un grain = une combinaison unique de valeurs dimensionnelles au niveau courant. Le montant associé est le solde agrégé à ce grain.

### 4.2 Facteur

Le facteur appliqué au montant de chaque grain sélectionné est le produit de deux composantes :

```
facteur = coefficient × multiplicateur
```

| Composante | Description | Exemples |
|------------|-------------|----------|
| **Coefficient** | Valeur dynamique issue du périmètre ou d'un taux. Peut varier par grain. | `pct_integration`, `pct_interet`, taux futurs (à définir) |
| **Multiplicateur** | Constante, typiquement 1 ou −1. | 1 (reproduire), −1 (extourner / contre-passer) |

Cas particuliers :
- Si aucun coefficient n'est spécifié → coefficient implicite = 1.
- Si aucun multiplicateur n'est spécifié (clé absente) → multiplicateur implicite = 1.
- `multiplicateur: null` explicite est **rejeté** (défense contre les bugs de parsing côté client : `Number("")` produit `NaN` en JS, qui se sérialise en `null` en JSON — sans ce garde-fou, la règle s'exécuterait silencieusement avec 1.0 au lieu de la valeur saisie).
- Donc par défaut, facteur = 1 (copie à l'identique).

### 4.3 Destination

Définit où et comment écrire l'écriture générée. Pour **chaque dimension pilotable** de l'écriture destination, quatre modes possibles :

| Mode | Sémantique | Exemple |
|------|------------|---------|
| `inherit` | La valeur du grain source est conservée. | `partner` hérité pour l'audit. |
| `override` | La valeur est forcée à une constante saisie. | `nature` → `2ELI`. |
| `null` | La valeur est vidée (`NULL`). | `partner` vidé sur la ligne principale. |
| `map` | La valeur est **résolue en traversant une caractéristique** du membre source. `via` = la caractéristique N1, `attr` = l'attribut N2 dont la valeur surcharge la dimension. La dimension écrite doit **correspondre à la dimension cible de l'attribut** (validé au moteur). **INNER JOIN** : seuls les membres *classés* (ayant une valeur pour la caractéristique) génèrent une écriture. | `account` → map `comportement.compte_destination` ; `nature` → map `comportement.nature` (même caractéristique, **multi-cible**). |

> **Mode `map` — caractéristiques N1/N2.** Une *caractéristique N1* (ex. `comportement`) regroupe les membres d'une dimension de base (les comptes) ; chacune de ses valeurs porte des *attributs N2* typés pointant vers d'autres dimensions (`compte_destination → comptes`, `nature → natures`). En `map`, le moteur joint `e.<base> → master_data.<N1> → car_<N1>.<attr>`. C'est la réalisation générique du mapping par compte source de [R3](#7-questions-ouvertes) : la règle ne liste pas les comptes ni ne code en dur le compte de liaison — elle route selon le comportement attribué au compte. Définition et affectation des caractéristiques : onglet **Caractéristiques** de l'UI. Implémentation : `prototype/rust/src/characteristics.rs` + `rules.rs` (parsing/validation/`exec_operation`).

Les dimensions d'une écriture se répartissent en deux catégories :

**Dimensions toujours héritées** (non pilotables par les règles) :

| Dimension | Raison |
|-----------|--------|
| `scenario` | Pas de génération cross-scénario (R4) |
| `period` | Pas de génération cross-période (R4) |
| `entry_period` | Liée à period, même logique |
| `currency` | Même devise que le grain source |
| `analysis` | Non modifiable (R4) |

**Dimensions pilotables** (hériter / surcharger / vider) :

| Dimension | Exemples d'usage |
|-----------|-----------------|
| `entity` | Surcharger vers une entité de consolidation |
| `account` | Surcharger vers un compte de regroupement |
| `flow` | Surcharger vers F98, F99… |
| `nature` | Surcharger vers 2ELI |
| `partner` | Hériter (audit) ou vider (bilan) |
| `share` | Modifiable (R4) |

---

## 5. Interaction avec le pipeline

### 5.1 Niveau d'exécution

Le niveau où s'insère l'écriture automatique **est déterminé par le niveau de sélection** : on sélectionne à un niveau, on génère au même niveau.

### 5.2 Ordre d'exécution à un niveau

Pour un niveau donné du pipeline, l'ordre est :

1. **Automatismes du niveau** s'exécutent d'abord sur les données du pipeline (ex : conversion au niveau converted, reconstruction des clôtures F99).
2. **Puis les règles** sélectionnent sur ce niveau **achevé**.
3. Les écritures générées sont ajoutées à ce même niveau **sans re-déclencher les automatismes**.

Conséquence : si la sélection se fait au niveau *converted*, le montant généré est déjà en devise de présentation et **ne sera pas re-converti**. Il descend directement vers le niveau suivant (consolidated).

### 5.3 Reconstruction des clôtures

Les écritures générées par les règles participent à la reconstruction des clôtures (F99) au niveau où elles sont injectées — **si** elles portent un flux qui reporte à F99 (via `flux_de_report`). Une règle qui génère un flux constituant (ex : F00, F20) verra son montant repris dans le F99 reconstruit.

---

## 6. Exemple : élimination interco

Cet exemple illustre l'utilisation complète du modèle. Il sera l'une des premières règles implémentées.

### 6.1 Contexte

Une écriture interco est identifiée par la présence d'une valeur dans la dimension `partner` (qui référence une entité du groupe). Le solde interco doit être extourné au niveau consolidé, avec une contrepartie sur un compte de regroupement pour préserver l'équilibre du bilan.

### 6.2 Règle « Élimination interco »

**Scope périmètre** : `entity.methode = 'globale' AND partner.methode = 'globale'` (scope croisé — les deux entités doivent être en méthode globale).

**Niveau de sélection** : consolidated (après application des méthodes).

**Opérations** (4) :

| Op | Sélection | Coefficient | Multiplicateur | Destination |
|----|-----------|-------------|----------------|-------------|
| 1 | `partner IS NOT NULL` | `pct_integration` | −1 | nature → `2ELI`, partner → **hérité** |
| 2 | `partner IS NOT NULL` | `pct_integration` | −1 | nature → `2ELI`, partner → **vidé** |
| 3 | `partner IS NOT NULL` | `pct_integration` | +1 | nature → `2ELI`, account → compte de regroupement, partner → **hérité** |
| 4 | `partner IS NOT NULL` | `pct_integration` | +1 | nature → `2ELI`, account → compte de regroupement, partner → **vidé** |

**Lecture des opérations** :

- **Ops 1+2** : extournent le solde interco (× −1).
  - Op 1 garde le partenaire → piste d'audit détaillée.
  - Op 2 vide le partenaire → l'extourne remonte dans le bilan agrégé sans éclater par contrepartie.
- **Ops 3+4** : posent la contrepartie sur un compte de regroupement (× +1).
  - Op 3 garde le partenaire → piste d'audit.
  - Op 4 vide le partenaire → visible au bilan.
- Les ops 1+3 (partenaire conservé) sont des informations additionnelles pour l'audit.
- Les ops 2+4 (partenaire vidé) sont les écritures visibles dans le bilan consolidé.
- Le compte de regroupement (ex : `450`) est un compte de consolidation dédié, paramétrable.

### 6.3 Nature porteuse

Les écritures d'élimination sont portées par une nature dédiée (ex : `2ELI`) pour :
- Préserver la **piste d'audit** (on distingue le solde original `0LIASS` de l'extourne `2ELI`).
- Permettre le **filtrage** dans les restitutions (bilan, compte de résultat).
- Le préfixe `2` indique que l'écriture est injectée après reclassification (cf. [`FLUX_CONSO.md`](./FLUX_CONSO.md) « Staging » et [Q29](./QUESTIONS_OUVERTES.md)).

---

## 7. Questions ouvertes

| ID | Question | Priorité |
|----|----------|----------|
| R1 | Les opérations au sein d'une même règle sont-elles indépendantes (sélectionnent toutes sur le même état initial) ou en cascade (l'op 2 voit les écritures de l'op 1) ? | TÔT | **TRANCHÉ (2026-06-18)** : Indépendant au sein d'une règle (toutes les opérations voient le même état initial). Cascade entre règles (la règle N+1 voit les écritures de la règle N). |
| R2 | Le coefficient peut-il référencer un taux de change ? (ex : convertir vers une devise tiers) | POST | **TRANCHÉ (2026-06-18)** : Non pour l'instant. La conversion FX est du ressort du pipeline natif (étape C), pas des règles. |
| R3 | Quelle est la granularité du compte de regroupement interco ? Un compte unique ou un mapping par compte source ? | TÔT | **TRANCHÉ (2026-06-18)** : Mapping par compte source, géré par l'utilisateur dans sa master data (table de mapping). Hors périmètre de la conception des règles elles-mêmes. La règle pointe vers un compte de destination défini par l'utilisateur. |
| R4 | Les règles peuvent-elles générer des écritures sur un **autre scénario** ou **autre période** que celui de la sélection ? | POST | **TRANCHÉ (2026-06-18)** : Non. `scenario`, `period`, `entry_period`, `currency` sont toujours héritées (dimensions non pilotables). Voir §4.3 pour la liste complète des dimensions factorisées. |
| R5 | Faut-il un mécanisme de **test / simulation** d'une règle avant exécution complète (preview des écritures générées) ? | TÔT | **TRANCHÉ (2026-06-18)** : Très bonne idée, mais reporté à une évolution ultérieure (post-MVP ++). |
| R6 | Persistance des règles : format (table DuckDB dédiée, JSON, fichier externe) ? | TÔT | **TRANCHÉ (2026-06-18)** : Table DuckDB dédiée + concept de **jeux de règles** (rulesets). Les règles forment une bibliothèque centrale ; un jeu de règles est une collection **ordonnée de références** vers ces règles. Duplication d'un jeu → nouveau jeu pointant vers les mêmes règles. Pour modifier une règle, l'utilisateur crée une copie (nouveau nom) et la référence dans le nouveau jeu. La consolidation pointe vers un jeu précis. Versioning et audit implicites. |
| R7 | Le scope périmètre croisé (entity + partner) suffit-il pour l'interco, ou faut-il aussi un scope sur le partenaire du partenaire (chaînes d'interco) ? | POST | **TRANCHÉ (2026-06-18)** : Hors périmètre. Le scope croisé à deux entités suffit. Les chaînes d'interco se traitent par des règles successives, pas par une règle unique. |

---

## 8. Jeux de règles (rulesets)

Les règles ne sont pas exécutées individuellement mais assemblées dans des **jeux de règles**.

### 8.1 Modèle

- **Bibliothèque de règles** : ensemble central de règles, chacune avec un nom unique. Une règle est immuable dès lors qu'elle est référencée par un jeu.
- **Jeu de règles** : collection **ordonnée de références** vers des règles de la bibliothèque. Un jeu a un nom et implicitement une version (via son nom : « Interco v1 », « Interco v2 »…).
- **Duplication** : créer un nouveau jeu en copiant les références d'un jeu existant. Le nouveau jeu pointe vers les **mêmes règles** — ce ne sont pas des copies.
- **Modification** : pour changer une règle dans un nouveau jeu, l'utilisateur **crée une copie de la règle** (nouveau nom, ex : « Élim. ventes interco v2 ») et la référence dans le nouveau jeu à la place de l'ancienne. La règle originale reste intacte dans la v1.

### 8.2 Exécution

- La consolidation pointe vers un **jeu de règles précis**.
- À l'exécution, les règles du jeu sont résolues (via les références) et exécutées **séquententiellement dans l'ordre défini** par le jeu.
- Cascade entre règles : la règle N+1 voit les écritures générées par la règle N (cf. R1).

### 8.3 Interface

- **Bibliothèque de règles** : liste de toutes les règles disponibles (CRUD), indépendante des jeux.
- **Jeux de règles** : liste des jeux, avec pour chacun : nom, liste ordonnée des règles (drag-and-drop pour réordonner), action de duplication.
- Depuis un jeu : ajouter / retirer une règle de la bibliothèque, réordonner.

---

## 9. Interface utilisateur

### 9.1 Liste des règles

- Vue tabulaire : nom, description courte, ordre d'exécution, statut (active / inactive).
- **Réordonnancement** par glisser-déposer (drag-and-drop) — l'ordre d'exécution suit l'ordre de la liste.
- Actions : activer / désactiver, dupliquer, supprimer.
- L'ergonomie précise reste à travailler.

### 9.2 Éditeur de règle

Une règle se définit sur une page unique. Les déclencheurs de scope (conditions de périmètre) peuvent être sur une page secondaire ou en section repliable.

**Page principale** (une règle) :

```
┌─────────────────────────────────────────────────┐
│  Nom de la règle : [________________]            │
│  Scope périmètre  : [configurer →]  (page sec.)  │
│                                                   │
│  ── Opération 1 ──────────────────────────────   │
│  │ Sélection  : niveau [▼] + filtres dimensionnels│
│  │ Facteur    : coefficient [▼] × mult [___]      │
│  │ Destination: [hériter / surcharger / vider]    │
│  └─────────────────────────────────────────────   │
│  ── Opération 2 ──────────────────────────────   │
│  │ ...                                             │
│  └─────────────────────────────────────────────   │
│  [+ Ajouter une opération]                        │
└─────────────────────────────────────────────────┘
```

**Règle d'or** : sélection, facteurs et destination d'une opération doivent être visibles sur la même page. Le scope périmètre peut être mis à part car il est partagé entre toutes les opérations.

### 9.3 Sous-formulaire opération

Chaque opération est un sous-formulaire répétable :
- **Sélection** : choix du niveau + filtres (par caractéristiques et/ou par énumération sur chaque dimension).
- **Facteur** : choix du coefficient (liste déroulante : aucun, `pct_integration`, `pct_interet`, …) × multiplicateur (champ numérique, défaut 1).
- **Destination** : pour chaque dimension, trois choix (hériter / surcharger avec valeur / vider).

---

## 10. Catalogue des règles prévues

Ordre indicatif d'implémentation :

1. **Éliminations interco** — ventes, créances/dettes, marges en stock, dividendes (cf. §6).
2. **Éliminations des participations** — titres de participation / capitaux propres.
3. **Intérêts minoritaires** — quote-part hors groupe.
4. **Retraitements** — homogénéisation des méthodes comptables.
5. **Variations de capital** — entrées/sorties au capital.
6. **Répartition des résultats**.

Chaque règle sera spécifiée individuellement selon le modèle du §4 lorsqu'on entrera en implémentation.
