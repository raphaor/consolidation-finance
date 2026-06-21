# À-nouveau — Report d'ouverture entre exercices

> Spec fonctionnelle du **report d'ouverture** (à-nouveau) : comment le solde de
> clôture d'une consolidation N-1 alimente l'ouverture d'une consolidation N.
> Annexe de [`FLUX_CONSO.md`](./FLUX_CONSO.md) (qui n'en posait que le principe en
> §4) et de [`REGLES_CONSO.md`](./REGLES_CONSO.md) (les traitements de périmètre
> deviennent des règles). Décisions tranchées le 2026-06-20.

---

## 1. Objet

La consolidation est **par les flux** : l'ouverture d'un exercice (flux F00) doit
égaler la clôture de l'exercice précédent (flux F99), **à chaque niveau
d'élaboration**, pour que le bilan consolidé soit continu d'un exercice à l'autre.

Aujourd'hui le pipeline est **mono-période** : un run lit un scénario, en dérive
N-1 *uniquement pour les taux*, et le F00 ne vient que de la source (liasse). Il
n'existe **aucun report** de la conso N-1. Ce document spécifie ce report.

**Principe directeur — généricité.** Le moteur ne code jamais en dur `F00` ni
`F99`. Le rôle « ouverture issue d'un à-nouveau » est déclaré par une donnée
(`flux_a_nouveau`), exactement comme le rôle « clôture » l'est par
`flux_de_report` (auto-référence). Ces attributs sont aujourd'hui portés par le
**schéma de flux** (`sat_flow_scheme_item`, résolu par compte via
`v_flow_behavior` — cf. [`FLUX_CONSO.md`](./FLUX_CONSO.md) §2 bis, [Q32](./QUESTIONS_OUVERTES.md)),
non plus par `dim_flow` ; une conséquence directe est que le **résultat ne
reporte pas** d'à-nouveau (schéma `RESULTAT` : `flux_a_nouveau` NULL), seul le
bilan le fait. Toute la logique en dérive.

---

## 2. Modèle de données

### 2.1 `dim_flow.flux_a_nouveau` (nouveau champ)

Symétrique de `flux_de_report`. Un flux de clôture C déclare l'**ouverture O**
dans laquelle son solde se reporte à la période suivante.

| Attribut | Rôle |
|---|---|
| `flux_a_nouveau` | Flux d'ouverture qui reçoit, à l'exercice suivant, le solde de ce flux. **Renseigné aujourd'hui pour F99 → F00 uniquement.** NULL pour tous les autres. |

- Un flux **cible d'un à-nouveau** = tout `O` tel qu'il existe un `C` avec
  `flux_a_nouveau(C) = O`. C'est l'ensemble `{F00}` aujourd'hui. Le moteur le
  calcule par requête sur `dim_flow`, jamais en dur.
- Un flux **source d'à-nouveau** = tout `C` avec `flux_a_nouveau(C)` non NULL.
  C'est `{F99}` aujourd'hui.

> La mécanique reste générique : déclarer un autre couple (clôture
> intermédiaire → ouverture intermédiaire) suffirait à activer un second
> à-nouveau, sans toucher au code.

### 2.2 `dim_scenario.a_nouveau_scenario` (nouveau champ)

La **définition d'une consolidation** (= `dim_scenario`) reçoit une référence
**facultative** vers la consolidation d'à-nouveau.

| Attribut | Rôle |
|---|---|
| `a_nouveau_scenario` | FK `dim_scenario` (nullable). Le run N-1 figé dont on reporte la clôture. **NULL = pas d'à-nouveau** (cf. §6). |

### 2.3 Snapshot figé (décision : à-nouveau = conso N-1 figée)

Le report lit le **F99 stocké** d'un run N-1 **déjà calculé et verrouillé**, par
niveau de stockage. Conséquences :

- Reporter depuis un scénario `ouvert` est **toléré** : on émet un simple
  **avertissement** (le snapshot pourrait encore bouger). Le refus dur ne se
  justifierait que dans un workflow de production — hors périmètre (pas de
  workflow au MVP, [Q8](./QUESTIONS_OUVERTES.md)).
- Les lignes `fact_entry` du scénario figé doivent **survivre** au run N. C'est un
  prérequis d'implémentation fort : aujourd'hui un run fait `DELETE FROM
  fact_entry` global (`server.rs:509`). Il faudra **isoler la purge au scénario
  courant** (`DELETE … WHERE scenario = ?`) et préserver les scénarios figés.
  `fact_entry` porte déjà la dimension `scenario` (built-in propagée).

---

## 3. Mécanique moteur

### 3.1 Injection du report

À l'ouverture du run N, pour chaque flux source d'à-nouveau C (= F99) et son
ouverture cible O (= F00), on colle le solde du snapshot N-1. **Le report opère
au niveau corporate** (devise fonctionnelle) : le montant corporate du F00 N
**vient du F99 corporate N-1 du snapshot et écrase tout F00 issu de la liasse**
(`0LIASS`).

```
F00[N, corporate]  ←  F99[snapshot N-1, corporate]      (écrase le F00 de liasse)
```

Ce montant corporate est **autoritaire** et sert de base à tout le reste :

- les **écarts de conversion** (F80) en sont calculés (§3.3) ;
- le **report sur la clôture** F99 N en vient (reconstruction par `flux_de_report`).

Les niveaux supérieurs se **déduisent** de ce corporate par le pipeline normal —
il n'y a (presque) rien à coller au-dessus :

- **converti** : la conversion native du F00 corporate **reproduit exactement**
  le F99 converti N-1. F00 se convertit au taux de **clôture N-1** et, par
  l'identité de reconstruction `F99_converti = F99_fonctionnel × taux_clôture`,
  on a `F00_converti = F00_corporate × taux_clôture_{N-1} = F99_converti N-1`.
  → **aucune collecte ni exemption au converti.**
- **consolidé** : **seul niveau qui exige une collecte du snapshot** (+ exemption
  du `× pct` à l'étape D, §3.3), car le **% d'intégration a pu changer** entre
  N-1 et N — il ne peut pas être reproduit par le pipeline. La collecte fige le
  F00 consolidé au **% N-1** ; la variation vers le % N est une règle (§5.2).

  ```
  F00[N, consolidé]  ←  F99[snapshot N-1, consolidé]
  ```

- **Grain** : identique au grain de report (toutes dimensions propagées). Le
  scénario/période de destination sont ceux du run N ; le reste est hérité du
  snapshot.

### 3.2 Périmètre du report : entités consolidées en N-1 seulement

> « F99 N-1 collé sur F00 N **écrase tout, sauf si le package n'était pas dans la
> consolidation antérieure**. »

La distinction est binaire et **sans marqueur** sur les écritures : tous les F00
sont traités de la même façon, qu'ils viennent du report ou de la liasse. La
non-duplication est garantie en amont, à la source du F00 :

- Entité **consolidée en N-1** → son F00 N **vient du report** ; son F00 de
  liasse est **écrasé** (sinon double compte). Au corporate, on remplace **tout
  le F00 de l'entité** par le F99 corporate du snapshot, relabellisé F00.
- Entité **non consolidée en N-1** (nouvelle entrée) → **pas de report** : on
  laisse remonter son F00 de liasse, qui sera **reclassé en F01** par règle (§5).

#### Comment savoir qu'une entité était consolidée en N-1 ?

Source de vérité = **le snapshot lui-même**. Une entité était consolidée dans
l'à-nouveau **ssi le snapshot porte une clôture consolidée** (F99 au niveau
`consolidated`) pour elle :

```
consolidée_en_N1(E)  ⇔  EXISTS ( fact_entry
                                  WHERE scenario = <a_nouveau_scenario>
                                    AND entity   = E
                                    AND level    = 'consolidated'
                                    AND flow     = <flux de clôture> )
```

Plus robuste que relire le périmètre N-1 (`sat_perimeter`) : le snapshot est figé
et reflète ce qui a **réellement** été consolidé. Une entité en méthode non
consolidante (ex. équivalence) ou à `pct_integration = 0` n'a pas de F99
consolidé → elle est **traitée comme une nouvelle entrée**. Le report corporate
collecte ensuite le **F99 corporate** de ces mêmes entités.

### 3.3 F00 exempté des transforms natives des étapes inférieures

Une fois le F00 collé à chaque niveau, les étapes natives qui *produisent* ces
niveaux ne doivent **pas recalculer la valeur du F00** (sinon double compte avec
la valeur collée). Règle générale et **data-driven** : la branche
« value-producing » de chaque étape exclut les flux **cibles d'à-nouveau**
(`flow NOT IN (SELECT flux_a_nouveau FROM dim_flow WHERE flux_a_nouveau IS NOT NULL)`),
tout en laissant le F00 participer à ses **effets dérivés** (écart de conversion,
reconstruction de clôture). Concrètement :

| Étape | Effet sur F00 |
|---|---|
| **Conversion** | **Aucune exemption** : la conversion s'applique normalement au F00 corporate. Elle produit le *montant converti* (`F00_corporate × taux_clôture_{N-1}`) **et** l'écart **F80** = `F00_corporate × (taux_clôture_N − taux_clôture_{N-1})` (revalorisation de l'ouverture au taux de clôture N). Le converti obtenu **égale** le F99 converti N-1 (cf. §3.1, identité de reconstruction) — d'où l'inutilité d'une collecte au converti. → « la conversion s'applique normalement ». |
| **Consolidation** | `× pct_integration` sur **tous les flux sauf F00** : le F00 consolidé est collé du snapshot (§3.1) et donc figé au **% N-1**, l'étape D ne le re-multiplie pas. La **variation de % d'intégration** vers le % N est portée par une **règle** (§5.2), pas par le moteur. |
| **Reconstruction clôture** | F00 reporte à F99 normalement (`flux_de_report`), **depuis le montant corporate**. L'identité `F99 = F00 + Σ variations + Σ écarts` se referme à chaque niveau. → « le report F00 → F99 s'applique normalement ». |

---

## 4. Suppression de l'étape de reclassification native

> Décision 2026-06-20 : « C'est le bon moment pour retirer l'étape reclass, et
> laisser cette opération gérée par règle. On passera dorénavant de corporate à
> converti. »

Le niveau `reclassified` **disparaît du programme entier** (pas seulement de la
mécanique d'à-nouveau) : schéma, `validate`, `report`, `rules` (`ALLOWED_LEVELS`),
`staging` (préfixe `2`), stats serveur, docs. Le pipeline natif passe
**directement de corporate à converti**.

Les traitements de périmètre (F00→F01 pour les entrants, miroir −X sur F98 pour
les sortants), aujourd'hui **natifs** dans l'étape B (`reclassify.rs`),
deviennent des **règles de consolidation** au niveau **corporate**, à **créer par
l'utilisateur** (elles ne sont plus livrées en dur par le moteur).

### Pipeline cible

3 niveaux de stockage (corporate → converti → consolidé). Staging cible et
ordonnancement détaillés en [§4 bis](#4-bis-staging-cible-redéfinition-immédiate).

```
A. Corporate     agrégation stg_entry (préfixe 0,1) FILTRÉE au scope de conso
                 + injection à-nouveau (F00 collé, écrase la liasse)
                 + règles corporate (F00→F01 entrants, miroir F98 sortants)
                 + reconstruction des clôtures
C. Converti      injection préfixe 2 (avant mécanique)
                 + conversion corporate→converti + écarts F80/F81 + clôtures
                 + règles converti (variation de % F90/F95, interco…)
D. Consolidé     injection préfixe 3 (avant % d'intégration)
                 + % d'intégration (F00 exempté, cf. §3.3)
                 + injection préfixe 4 (après % d'intégration)
                 + règles consolidé + clôtures
```

- Le niveau `reclassified` **disparaît** (suppression franche, 2026-06-20). Les
  règles de périmètre sélectionnent au niveau corporate et y génèrent (mécanisme
  `run_pipeline_with_hook` + `after_level`, déjà en place).
- Le **corporate hérite du rôle de l'ex-étape B** : il devient un vrai point de
  traitement (injection à-nouveau + règles + reconstruction de clôtures), alors
  qu'aujourd'hui `AggregateStep` n'a ni staging ni `materialize_closures`.
- **Impact large** — `reclassified` est référencé dans : `schema.rs` (CHECK
  `level`), `validate.rs`, `report.rs`, `main.rs`, `rules.rs` (`ALLOWED_LEVELS`),
  `staging.rs`, `server.rs` (struct stats), `dump_pipeline.rs`, la doc
  (`FLUX_CONSO.md` §Niveaux). Les tableaux `LevelCounts = [usize;4]` et
  `[StepTiming;4]` passent à **3**.

---

## 4 bis. Staging cible (redéfinition immédiate)

> **Schéma intérimaire assumé** (décision 2026-06-20). S'appuyer sur le **préfixe
> du code de nature** comme point d'injection est **fragile** ; ce mécanisme sera
> retravaillé plus tard (porter le point d'injection par une vraie donnée plutôt
> que par une convention de nommage). En attendant, on redéfinit le mapping sur
> les **3 niveaux** restants.

### 4 bis.1 Mapping préfixe → point d'injection

| Préfixe | Niveau | Moment d'injection |
|---|---|---|
| `0`, `1` | **corporate** | Début, par l'agrégation (les deux préfixes sont fusionnés). Une règle sélectionnant corporate voit ces deux montants. |
| `2` | **converti** | Montant en **devise fonctionnelle**. Injecté **avant** la mécanique de report de flux et de calcul des écarts de conversion (F80/F81) → il **passe par la conversion** (montant converti **+** écarts). |
| `3` | **consolidé** | **Avant** la mécanique de taux → **se voit appliquer le `pct_integration`** (pourcentage d'**intégration**, comme le reste du consolidé — pas le `pct_interet`). |
| `4` | **consolidé** | **Après** la mécanique de taux → entre **déjà consolidé**, pas re-multiplié. |

- **Changement notable** : le préfixe `2` passe de `reclassified` (supprimé) à
  **converti, avant écarts**. Comme il est en **devise fonctionnelle**, il subit
  la conversion **et** le calcul des écarts F80/F81, puis la reconstruction de
  clôture au converti — c'est l'ancien comportement du préfixe `2`, simplement
  rebaptisé du niveau `reclassified` vers `converti`.
- **Impact recette** : la **règle de test des écarts de conversion** doit être
  remise en cohérence avec cette nouvelle codification du préfixe `2` (cf. tests
  Python — [[strategie-tests]]).
- **Priorité du traitement d'ouverture** : le F00 est gouverné par l'à-nouveau
  (§3). Un montant **F00 saisi dans une écriture de préfixe `3`** (consolidé) est
  **ignoré** — la collecte à-nouveau prime. Plus généralement, un flux **cible
  d'à-nouveau** ne peut pas être alimenté par le staging au consolidé.
- **À-nouveau ≠ préfixe 3** : la collecte à-nouveau du F00 au consolidé est
  **exemptée** du taux (figée au % N-1) ; le préfixe `3`, lui, **subit** le taux.

### 4 bis.2 Filtre de scope à l'agrégation corporate (nouveau)

L'agrégation corporate doit **filtrer les entités** selon leur appartenance au
**scope de consolidation** du run, **indépendamment de la méthode** :

- Sont **agrégées** les entités présentes dans le périmètre du run
  (`sat_perimeter` pour ce scénario/période), **quelle que soit leur méthode**.
- Les entités **entrantes et sortantes comptent comme faisant partie** du scope
  pour ce premier filtre.
- Les entités **hors scope** sont **exclues** du corporate (elles ne sont pas
  consolidées du tout).

C'est un ajout par rapport à `aggregate::step_a` actuel, qui agrège tout
`stg_entry` de préfixe `0`/`1` **sans filtre d'entité**. Le filtre se fait par
jointure sur `sat_perimeter` (présence d'une ligne, méthode indifférente).

> À préciser à l'implémentation : ce filtre de scope est-il distinct du filtre de
> **méthode consolidante** appliqué plus tard à l'étape D (`dim_method.consolidated`) ?
> Oui — le scope corporate inclut **toutes** les méthodes (même équivalence), le
> filtre méthode n'intervient qu'au passage consolidé.

---

## 5. Traitements de périmètre devenus des règles

### 5.1 Entrée de périmètre — F00 → F01

Une entité **absente du snapshot N-1** (donc sans report, §3.2) a un F00 issu de
sa liasse. Une règle au niveau corporate **reclasse ce F00 en F01** (l'ouverture
de l'entrant est isolée en F01 ; le F00 consolidé ne contient que le report du
périmètre existant). Scope : `entree = true` sur `sat_perimeter`.

> **Couplage à garantir (A3)** : le carry (§3.2) et cette règle doivent partager
> **une seule vérité** sur « qui est entrant ». `entree` est **dérivé** de
> l'absence au snapshot N-1 (`entree = NON consolidée_en_N1(E)`), pas saisi à la
> main. Sinon : risque de F00 à la fois reporté **et** reclassé, ou d'entrante
> orpheline.
>
> **Contrôle de cohérence (à ajouter dans `validate.rs`)** : même avec la
> dérivation, signaler le cas théorique où une entité **non considérée comme
> consolidée** porte malgré tout des montants dans le snapshot d'ouverture (ou
> l'inverse). Concrètement : alerter si `sat_perimeter.entree` saisi diverge de
> `NON consolidée_en_N1(E)`, ou si une entité absente du périmètre N courant a un
> F99 consolidé dans le snapshot. Cas très rare, mais on veut le **détecter**, pas
> le subir silencieusement.

### 5.2 Variation de % d'intégration — F90 / F95 (par règle)

> Décision 2026-06-20 : « à faire par règle, ne fait pas partie du moteur mais du
> standard de règles. » **Le moteur n'a pas à connaître le flux porteur** de
> cette variation (F90, F95 ou autre) : c'est un choix de **paramétrage de
> règle**. S'il en avait besoin, ce serait une erreur de conception.

Le F00 consolidé est figé au **% N-1** (collé au niveau consolidé). Une règle au
niveau converti aligne l'ouverture sur le **% N** en postant la variation, vers
le flux **que l'auteur de la règle choisit** :

```
<flux de variation>  =  F00_converti × (pct_integration_N − pct_integration_{N-1})
```

de sorte que `F00 + F90` au consolidé = `F00_converti × pct_N`. Le moteur doit
seulement rendre le F00 **issu d'un à-nouveau identifiable** (par son flux + le
fait qu'il provienne du report) pour que la règle le cible ; il ne calcule pas
lui-même la variation. `pct_integration_{N-1}` provient du snapshot N-1.

### 5.3 Sortie de périmètre — miroir F98

Inchangé fonctionnellement (cf. `FLUX_CONSO.md` §9), mais déplacé du natif vers
une règle au niveau corporate : chaque constituant X génère `−X` sur F98, donc
`F98 = −Σ(constituants)` et `F99 = 0` par identité.

---

## 6. Cas sans à-nouveau : tout est entrant

> « Si la consolidation d'à-nouveau n'existe pas dans la définition, on s'attend à
> ce que **tous les packages soient entrants** et que leur F00 soit reclassé sur
> F01. Le scope de consolidation doit le refléter. »

Quand `dim_scenario.a_nouveau_scenario` est NULL :

- Aucune entité n'a de report → **toutes** sont traitées comme entrantes → tous
  les F00 → F01 (règle §5.1, scope `entree = true`).
- **Cohérence à garantir** : `sat_perimeter.entree` doit valoir `true` pour
  toutes les entités du scope. À décider (§8) : le moteur le **force/dérive**
  automatiquement quand l'à-nouveau est absent, ou bien c'est une **contrainte de
  saisie** du périmètre (validée, pas dérivée). Recommandé : dériver l'« entrant
  par défaut » de l'absence de report plutôt que de dépendre d'un flag saisi à la
  main, pour éviter les incohérences.

---

## 7. Synthèse des changements

| Élément | Nature | Détail |
|---|---|---|
| `dim_flow.flux_a_nouveau` | Schéma | Nouveau champ nullable (F99 → F00). |
| `dim_scenario.a_nouveau_scenario` | Schéma | FK nullable vers le run N-1 figé. |
| Isolation des runs par scénario | Moteur | `DELETE … WHERE scenario = ?` + préservation des scénarios figés. |
| Injection du report | Moteur | Coller F99[N-1] → F00[N] au **corporate** (écrase la liasse) et au **consolidé** ; le converti se déduit par conversion normale. Entités présentes en N-1 seulement. |
| Exemption F00 | Moteur | **Consolidation seule** : `× pct` exclut les flux cibles d'à-nouveau. **Conversion : aucune exemption** (elle reproduit le F99 converti N-1 + F80 depuis le corporate). |
| Suppression étape B | Moteur | Pipeline corporate → converti ; niveau `reclassified` **retiré du programme entier**. |
| F00→F01, miroir F98 | Règles | Migrés du natif vers des règles niveau corporate, **à créer par l'utilisateur**. |
| Variation de % (F90/F95) | Règles | Standard de règles, calculée d'après le F00 d'à-nouveau. |
| Staging redéfini (3 niveaux) | Moteur | `0`,`1`→corporate ; `2`→converti (avant écarts) ; `3`→consolidé (avant %) ; `4`→consolidé (après %). Intérimaire (préfixe fragile). Voir §4 bis. |
| Filtre de scope au corporate | Moteur | Agrégation limitée aux entités du périmètre du run (toutes méthodes ; entrantes/sortantes incluses). Voir §4 bis.2. |
| Règle de test des écarts | Recette | À remettre en cohérence avec le préfixe `2` désormais au converti. |

---

## 8. Points encore ouverts

| # | Question |
|---|---|
| A1 | **RÉSOLU (2026-06-20)** — Base de l'écart d'ouverture F80 = le **montant corporate** collé (fonctionnel × delta de taux). Exact, pas une approximation : par l'identité de reconstruction `F99_converti = F99_fonctionnel × taux_clôture`, la conversion native du F00 corporate reproduit le F99 converti N-1 + F80. Pas de collecte au converti. |
| A2 | **RÉSOLU (2026-06-20)** — Niveau `reclassified` : **suppression franche** du programme entier (§4). Reste un sous-point d'implémentation : sort du **préfixe de staging `2`**. |
| A3 | **RÉSOLU (2026-06-20)** — Entrant = **dérivé** de l'absence au snapshot N-1 (vérité unique pour le carry et la règle F00→F01). **+ contrôle de cohérence dans `validate.rs`** : détecter le cas (théorique) où une entité non consolidée porte des montants au snapshot, ou divergence entre `entree` saisi et présence au snapshot (§5.1). |
| A4 | **RÉSOLU (2026-06-20)** — Hors moteur. Le flux porteur de la variation de % est un **paramétrage de règle** ; le moteur n'a pas à le connaître (sinon erreur de conception). |
| A5 | **RÉSOLU (2026-06-20)** — Statut `ouvert` **toléré** (avertissement suffisant). Refus dur réservé à un workflow de production, hors périmètre (Q8). |
| A6 | **RÉSOLU (2026-06-20)** — Pas de marqueur : tous les F00 sont traités pareil. La non-duplication est garantie **à la source** (carry pour les entités consolidées en N-1, liasse sinon — §3.2), pas par une distinction sur l'écriture. |
