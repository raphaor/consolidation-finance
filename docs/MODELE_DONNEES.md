# Modèle de données — Dimensions

Annexe de [`EXPRESSION_DE_BESOIN.md`](../EXPRESSION_DE_BESOIN.md) §4.
Sémantique des champs du CSV et caractéristiques *master data* de chaque dimension.

---

## 1. Sémantique des champs (désambiguïsée)

| Champ | Nature | Sémantique |
|---|---|---|
| `Scenario` | Dimension | Scénario (réel / budget / prévision / plan) |
| `Entity` | Dimension | Entité juridique émettrice de l'écriture |
| `Entry_period` | Dimension | Exercice comptable en cours (la clôture travaillée) |
| `Period` | Dimension | Période impactée par l'écriture (ex. chaque année d'un plan à 3 ans) |
| `Account` | Dimension | Compte du plan de compte du groupe |
| `Flow` | Dimension | Code de flux identifiant l'origine du montant (F00 ouverture, F20 variation, F80/F81 conversion, F01/F98 périmètre, F99 clôture…) — voir [`FLUX_CONSO.md`](./FLUX_CONSO.md) |
| `Currency` | Dimension | Devise de l'écriture |
| `Nature` | Dimension | Nature de l'écriture (code identifiant l'origine et le niveau de chargement — ex. `0LIASS`, `1AJUST`). Obligatoire. Voir §3 `Nature`. |
| `Partner*` | Dimension | Contrepartie (interco si dans le périmètre, sinon tiers lié / externe) |
| `Share*` | Dimension | Participation visée par l'écriture (A détient B → `Share` identifie la relation et la cible B) |
| `Analysis*` | Dimension | Axe analytique libre (centre de coût, projet…) |
| `Analysis2*` | Dimension | Second axe analytique libre. **Ex-`Audit_id`** : cette dimension avait été créée pour la traçabilité, mais ce rôle n'est **pas** le sien — `analysis2` est un axe analytique générique (soumis à la sémantique « of which », cf. §4 bis). La trace de provenance d'une ligne vit dans `Source` (ci-dessous), **jamais** ici (l'y mettre ferait de chaque ligne un « dont »). |
| `Source*` | Métadonnée (non-dimensionnelle) | Référence de **provenance** d'une ligne (ex. réf. de liasse source `S-M-001`, plus tard règle/import générateur). **Hors registre des dimensions** : non propagée par le pipeline, hors grain de clôture, hors « of which ». Portée par `stg_entry` (colonne `source`). Il n'existe pas de colonne `audit_id` : c'est `Source` qui tient ce rôle. |
| `Amount` | Mesure | Montant |

`*` = optionnel.

## 2. Principes structurants

1. **Dimensions vs tables satellites** : les axes ci-dessus sont des *dimensions de saisie*. Les règles de consolidation vivent dans des *tables satellites* qui les référencent : **Périmètre de consolidation**, **Taux de change**.
2. **Liste centrale d'entités** : `Entity`, `Partner` et `Share` sont **trois rôles** pointant vers une même master data des entités (entités du groupe + tiers liés : associés, co-entreprises). L'appartenance au groupe se déduit du *Périmètre*.
3. **Une seule table des périodes** : `Entry_period` et `Period` sont deux rôles (clés étrangères) vers la même master data ; `Entry_period` est contraint au type « exercice ».
4. **Interface master data** : **écran CRUD complet** (créer / lister / éditer / désactiver) pour **chaque dimension et chaque table satellite**. Sans valeur enregistrée, la saisie d'écritures est impossible.

## 3. Catalogue des dimensions

Pour chaque dimension : *Master data* (attributs à gérer) · *Conso* (traitements alimentés).

### `Scenario`
- **Master** (objet composite v2) : `code`, `libellé`, `category` (réel / budget / prévision), `entry_period`, `presentation_currency`, `variant`, `ruleset_code` (nullable), `rate_set` (jeu de taux), `perimeter_set` (jeu de périmètre), `a_nouveau_scenario` (conso N-1 figée, nullable), `statut` (ouvert / verrouillé). La **devise pivot** est applicative (`app_config`), pas par scénario.
- **Conso** : agrège **toutes les références d'un run** (taux, périmètre, règles, à-nouveau) ; pilotage multi-scénarios.

### `Entity`
- **Master** : `code`, `libellé`, `forme_juridique`, `pays`, `devise_fonctionnelle` (→ `Currency`), `entité_parent` (structure de groupe), `statut`
- **Conso** : unité de consolidation ; la devise fonctionnelle pilote la conversion.
- Note : méthode et % de conso **ne sont pas ici** (variants par période) → table *Périmètre*.

### `Period` (table unique des périodes)
- **Master** : `code`, `libellé`, `type` (mois / trimestre / année / exercice), `date_début`, `date_fin`, `exercice_rattaché`, `statut` (clôturé / ouvert)
- **Rôles** : `Period` (période impactée par l'écriture) et `Entry_period` (exercice en cours / clôture travaillée) sont **deux clés étrangères vers cette même table** ; `Entry_period` est contraint au `type = exercice`.
- **Conso** : axe temporel d'agrégation et de conversion devise.

### `Account`
- **Master** : `code`, `libellé`, `sens` (débit / crédit), `classe` (bilan / résultat / flux / hors-compte), `sous_classe`, `flow_scheme` (schéma de flux → taux de conversion et flux d'écart **par flux**, cf. [`FLUX_CONSO.md`](./FLUX_CONSO.md) « Schémas de flux » ; NULL = défaut dérivé de la classe)
- **Attributs ajoutés à l'exécution** (plus codés en dur) : le regroupement par nature (ex. `capitaux_propres`, utilisé par la **mise en équivalence**) se déclare comme **caractéristique** ; le **compte parent** (hiérarchie d'agrégation) comme **référence directe** vers `Account` lui-même (cf. §4 ter et la page « Attributs de dimension »).
- **Conso** : cumul [B], agrégation hiérarchique pour les restitutions.

### `Flow`
- **Rôle** : code de flux identifiant **l'origine d'un montant**. Les automatismes de conso agissent sur les **flux de variation** ; F99 (clôture) est un solde **reconstruit** par identité à chaque niveau de stockage (il transite comme un flux ordinaire, voire saisi en mode formulaire bilan, puis `materialize_closures` le reconstruit/l'écrase) → **cœur de la consolidation par les flux** et traçabilité totale.
- **Master** : `code`, `libellé` seulement — `Flow` (`dim_flow`) est une **dimension nue**. Le comportement par flux (`taux_conversion`, `flux_ecart`, `flux_de_report`, `flux_a_nouveau`) est porté par le **schéma de flux** (`sat_flow_scheme_item`), résolu par compte via `v_flow_behavior` (cf. [`FLUX_CONSO.md`](./FLUX_CONSO.md) §2 bis et [Q32](./QUESTIONS_OUVERTES.md)).
- **Catalogue des valeurs + mécanique complète** : voir [`FLUX_CONSO.md`](./FLUX_CONSO.md) (F00 ouverture, F20 variation, F80/F81 écarts de conversion, F01/F98 périmètre, F99 clôture…).
- **Conso** : alimente la restitution « Bilan par flux » ; identité `F99 = F00 + Σ variations + Σ écarts` (tient avant et après conversion).

### `Currency`
- **Master** : `code_ISO`, `libellé`, `décimales`, `rôle` (fonctionnelle / présentation)
- **Conso** : conversion multi-devises [B], via la table *Taux de change*.

### `Nature`
- **Rôle** : identifie l'**origine et le niveau de chargement** d'une écriture. Le préfixe du code détermine à quelle étape du pipeline la nature est chargée.
- **Master** : `code`, `libellé`, `rules` (TEXT, JSON — pour le futur module de traitement automatique)
- **Convention de nommage** (préfixe → étape de chargement) :

  | Préfixe | Chargement | Exemple |
  |---|---|---|
  | `0` | Données de liasse (saisie brute) | `0LIASS` |
  | `1` | Avant reclassification | `1AJUST` |
  | `2` | Après reclass, avant conversion | *(à venir)* |
  | `3` | Après conversion, avant consolidation | *(à venir)* |
  | `4` | Après consolidation | *(à venir)* |

- **Conso** : la nature est **préservée à travers toutes les étapes** du pipeline. Deux écritures de natures différentes **ne sont jamais agrégées** — la nature entre dans le `GROUP BY` de chaque étape (agrégation, reclassification, conversion, consolidation) et dans le grain de reconstruction des clôtures (F99).
- **Valeurs de base** : `0LIASS` (liasse), `1AJUST` (ajustement).

### `Partner*`
- **Rôle** : contrepartie d'une opération. **Rôle** sur la liste centrale des entités (entité du groupe = interco ; tiers lié = associé / coentreprise).
- **Conso** : éliminations interco [C], mise en équivalence, déconso, dividendes.
- Note : l'appartenance au groupe se déduit du *Périmètre*, pas d'un attribut local.

### `Share*`
- **Rôle** : identifie la participation visée par l'écriture. **Rôle** sur la liste centrale des entités (pointe vers l'entité **détenue**).
- **Conso** : variations de capital [C], répartition des résultats [C], mise en équivalence [C].
- Note : les caractéristiques de la participation (% intérêt, % intégration, méthode, dates) sont portées par la table **Périmètre**, pas par `Share` lui-même.

### `Analysis*`
- **Master** : `code`, `libellé`, `parent` (hiérarchie optionnelle)
- **Conso** : aucun traitement propre — axe de filtrage / restitution uniquement.

## 4. Tables satellites (référencées par les dimensions)

### Périmètre de consolidation
Versionné comme les taux : un **jeu de périmètre** (`dim_perimeter_set`) est référencé par le scénario (`dim_scenario.perimeter_set`), symétrique de `rate_set` (cf. [Q35](./QUESTIONS_OUVERTES.md)). `sat_perimeter` est donc clé par `Perimeter_set × Entity × Period` (un même périmètre est réutilisable entre scénarios/variantes). Liste, par cette clé, les entités du scope avec leurs caractéristiques de conso :
- `méthode` (globale / proportionnelle / équivalence / IFRS5)
- `%_intérêt` et `%_intégration` (= % de contrôle)
- `entrée_sortie_mid_exercice` (booléen, lié à `Period`)
- `fusion` : `entité_absorbante` et `entité_absorbée` (le cas échéant)

Pilotage : variations de périmètre et mise en équivalence (natif MVP) ; intérêts minoritaires (règles, post-MVP).

**Calcul des variations de périmètre [B]** : obtenu par **comparaison du scope de l'exercice en cours vs. la consolidation d'ouverture** (N-1 consolidé). En règle générale, toute consolidation **repart de la consolidation d'ouverture** de l'année précédente ; la 1ʳᵉ consolidation fait exception (toutes les entités sont traitées comme entrantes).

Répond à [Q5](./QUESTIONS_OUVERTES.md).

### Taux de change
- **Versionné** : `dim_rate_set` (jeu de taux) référencé par `dim_scenario.rate_set`. `sat_exchange_rate` est clé par `(rate_set, currency_source, period)` et donne `taux_close` + `taux_moyen` (moyenne **simple** sur la période). Tous les taux convertissent vers une **devise pivot** applicative ; la conversion vers la devise de présentation se fait par **cross-rate** (cf. [Q34](./QUESTIONS_OUVERTES.md)).
- **Application** (conversion multi-devises [B]) : le **taux par flux** est piloté par le **schéma de flux** du compte (`dim_account.flow_scheme` → `sat_flow_scheme_item`), avec repli sur le défaut `dim_flow`. Un compte de **bilan** suit le défaut (`taux_clôture` à la clôture, écart F80/F81) ; un compte de **résultat** suit le schéma `RESULTAT` (`taux_moyen`, **sans écart**). Cf. [`FLUX_CONSO.md`](./FLUX_CONSO.md) « Schémas de flux » et [Q32](./QUESTIONS_OUVERTES.md).
- **Saisie** : écran CRUD + import d'un fichier CSV (format à spécifier). Pas de récupération automatique externe au POC.

### Paramètres du groupe (configuration)
Réglages globaux du groupe, gérés via une **page de paramètres** (hors master data dimensionnelle). Notamment :
- `compte_équivalence_actif` : compte d'actif portant la contrepartie de la mise en équivalence (ex. `261E`).
- `compte_équivalence_résultat` : compte où se condense le P&L des entités à l'équivalence (ex. `880E`).

## 4 bis. Catégories de dimensions et sémantique « of which »

Chaque dimension appartient à une **catégorie** (registre `engine/src/dimensions.rs`) qui détermine son comportement dans le pipeline :

| Catégorie | Propagée | Pilotable (règles) | Nullable | Grain de clôture | Dans les **totaux** |
|---|---|---|---|---|---|
| **Fixed** (scenario, entry_period, period, currency) | oui | non | non | oui | oui |
| **Active** (entity, account, flow, nature) | oui | oui | non | oui | oui |
| **Analytical** (partner, share, analysis, analysis2 + **custom**) | oui | oui | **oui** | **oui** | **non (voir ci-dessous)** |

**Sémantique « of which » (dont)** — règle centrale, décidée suite à l'audit du modèle :

> Une ligne dont une dimension **Analytical** est renseignée est un **« dont »** (of which) de la ligne de même grain où cette dimension est **NULL**. Elle ne s'additionne **jamais** au total.

Concrètement :
- **Totaux** (bilan, compte de résultat) : ne somment que les lignes **principales** — toutes les dimensions analytiques `IS NULL`. Le filtre est dérivé du registre (`dimensions::analytical_cols`) ; cf. `server.rs` (`/api/bilan`, `/api/compte-resultat`) et `report.rs`.
- **Clôtures** : les dimensions analytiques **font partie du grain de clôture** (`pipeline/materialize_closures.rs`). Chaque « dont » obtient **sa propre F99** (ex. la clôture `partner = B` = Σ de ses seuls constituants `partner = B`), et la clôture principale ne somme que les constituants principaux → **pas de double compte**.
- **Conversion / variations** : les lignes « dont » subissent **les mêmes automatismes** que les lignes principales (mêmes flux, même conversion, écarts F80/F81 hérités) à leur propre grain.

Conséquence pratique : **une ligne principale ne doit jamais porter de valeur analytique** (sinon elle disparaît des totaux). Exemple à proscrire : un identifiant d'audit posé sur `analysis2` de chaque écriture.

## 4 ter. Intégrité référentielle (graphe de références)

Le modèle n'a pas de FK dures (DuckDB, choix du proto). Les liens entre objets sont déclarés dans un **registre central** (`engine/src/references.rs`) : chaque `(table.colonne) → (table_cible.colonne)`. Il couvre notamment :

- `dim_scenario.{category, entry_period, presentation_currency, variant, ruleset_code, rate_set, perimeter_set}`
- `dim_entity.{devise_fonctionnelle, entite_parent}`, `dim_account.{sous_classe, flow_scheme}`, `dim_flow.{flux_ecart, flux_de_report}`, `sat_flow_scheme_item.{scheme, flow, flux_ecart}`
- **références dynamiques** (ajoutées à l'exécution) : caractéristiques N1/N2 et **références directes** (patron B, ex. `dim_account.compte_parent → dim_account.code`), fusionnées au graphe statique par `references::all_references`
- `sat_perimeter.{perimeter_set, entity, period, methode}`, `sat_exchange_rate.{rate_set, currency_source, period}`
- écritures : `{scenario, entity, entry_period, period, account, flow, currency, nature}` + `{partner, share}` → **`dim_entity`** (les trois rôles de la §2.2)
- `dim_ruleset_item.{ruleset_code, rule_code}`

**Validation à l'écriture** (rejet d'une référence inexistante, message explicite) :
- **Master data** : `create`/`update` (`masterdata.rs`) — avec tolérance d'auto-référence (`flux_de_report = F99` sur la ligne F99).
- **Imports CSV** : entries / rates / perimeter (`import.rs`) — anti-jointure du fichier contre les tables cibles **avant** insertion.
- **Définitions de règles** : valeurs de `selection` / `destination override` / `scope` (`rules.rs::validate_definition`, appelée à `POST`/`PUT /api/rules`).

Les dimensions **Analytical libres** (`analysis`, `analysis2`, custom) n'ont **pas** de référence → saisie libre.

## 4 quater. Export / import complet (sauvegarde-restauration)

`GET /api/export` produit un **paquet JSON unique** `{ table → [lignes] }` couvrant tout l'état persistant (config, dimensions, satellites, **écritures**, **règles + rulesets**, **dimensions custom**) — `fact_entry` exclue (dérivée). `POST /api/import/all` restaure ce paquet par **remplacement total** (DROP + CREATE, recréation des dimensions custom, réinsertion), sans relancer le pipeline. UI : boutons « Tout exporter » / « Importer un paquet… » de la vue Pipeline. À distinguer de `load_all` (qui ne charge que les 16 CSV de référentiels, sans les règles).

## 5. Points ouverts (renvoi au registre)

- [Q5] *(tranché)* — table *Périmètre* définie §4.

Tranchés ce tour : [Q4] (taux clôture + moyenne simple ; CRUD + import CSV), [Q20] (table `Period` unique, `Entry_period` = rôle), [Q21] (pas de table *Participations* ; `Share`/`Partner`/`Entity` = 3 rôles sur la liste d'entités), [Q22] (`Partner` → liste d'entités), [Q23] (CRUD complet pour toutes les dimensions + satellites).
