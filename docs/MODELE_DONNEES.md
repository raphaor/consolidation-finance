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
| `Audit_id` | Référence | Traçabilité de l'écriture (saisie initiale + écritures auto) |
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
- **Master** : `code`, `libellé`, `type` (réel / budget / prévision), `exercice_référence`, `statut` (ouvert / verrouillé)
- **Conso** : sélectionne le jeu d'écritures consolidé ; pilotage multi-scénarios.

### `Entity`
- **Master** : `code`, `libellé`, `forme_juridique`, `pays`, `devise_fonctionnelle` (→ `Currency`), `entité_parent` (structure de groupe), `statut`
- **Conso** : unité de consolidation ; la devise fonctionnelle pilote la conversion.
- Note : méthode et % de conso **ne sont pas ici** (variants par période) → table *Périmètre*.

### `Period` (table unique des périodes)
- **Master** : `code`, `libellé`, `type` (mois / trimestre / année / exercice), `date_début`, `date_fin`, `exercice_rattaché`, `statut` (clôturé / ouvert)
- **Rôles** : `Period` (période impactée par l'écriture) et `Entry_period` (exercice en cours / clôture travaillée) sont **deux clés étrangères vers cette même table** ; `Entry_period` est contraint au `type = exercice`.
- **Conso** : axe temporel d'agrégation et de conversion devise.

### `Account`
- **Master** : `code`, `libellé`, `sens` (débit / crédit), `classe` (bilan / résultat / flux / hors-compte), `capitaux_propres` (booléen — identifie les comptes de capitaux propres, utilisés par la **mise en équivalence**), `compte_parent` (hiérarchie d'agrégation)
- **Conso** : cumul [B], agrégation hiérarchique pour les restitutions.

### `Flow`
- **Rôle** : code de flux identifiant **l'origine d'un montant**. Les automatismes de conso agissent sur les **flux de variation** ; F99 (clôture) est un solde **reconstruit** par identité à chaque niveau de stockage (il transite comme un flux ordinaire, voire saisi en mode formulaire bilan, puis `materialize_closures` le reconstruit/l'écrase) → **cœur de la consolidation par les flux** et traçabilité totale.
- **Master** : `code`, `libellé`, `taux_conversion` (type de taux), `flux_de_report` (défaut F99, pour **tous** les flux y compris les écarts), `flux_ecart_conversion` (flux d'écart associé ; null pour les écarts eux-mêmes — terminaux : taux clôture → écart propre = 0).
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
Liste, par `Entity × Scenario × Period`, les entités du scope avec leurs caractéristiques de conso :
- `méthode` (globale / proportionnelle / équivalence / IFRS5)
- `%_intérêt` et `%_intégration` (= % de contrôle)
- `entrée_sortie_mid_exercice` (booléen, lié à `Period`)
- `fusion` : `entité_absorbante` et `entité_absorbée` (le cas échéant)

Pilotage : variations de périmètre et mise en équivalence (natif MVP) ; intérêts minoritaires (règles, post-MVP).

**Calcul des variations de périmètre [B]** : obtenu par **comparaison du scope de l'exercice en cours vs. la consolidation d'ouverture** (N-1 consolidé). En règle générale, toute consolidation **repart de la consolidation d'ouverture** de l'année précédente ; la 1ʳᵉ consolidation fait exception (toutes les entités sont traitées comme entrantes).

Répond à [Q5](./QUESTIONS_OUVERTES.md).

### Taux de change
- Définit, par `Currency_source × Currency_cible × Period` : `taux_clôture` et `taux_moyen` (moyenne **simple** sur la période).
- **Application** (conversion multi-devises [B]) : `taux_clôture` pour les comptes de bilan, `taux_moyen` pour les comptes de résultat — règle dérivée de la `classe` du compte (cf. `Account`).
- **Saisie** : écran CRUD + import d'un fichier CSV (format à spécifier). Pas de récupération automatique externe au POC.

### Paramètres du groupe (configuration)
Réglages globaux du groupe, gérés via une **page de paramètres** (hors master data dimensionnelle). Notamment :
- `compte_équivalence_actif` : compte d'actif portant la contrepartie de la mise en équivalence (ex. `261E`).
- `compte_équivalence_résultat` : compte où se condense le P&L des entités à l'équivalence (ex. `880E`).

## 5. Points ouverts (renvoi au registre)

- [Q5] *(tranché)* — table *Périmètre* définie §4.

Tranchés ce tour : [Q4] (taux clôture + moyenne simple ; CRUD + import CSV), [Q20] (table `Period` unique, `Entry_period` = rôle), [Q21] (pas de table *Participations* ; `Share`/`Partner`/`Entity` = 3 rôles sur la liste d'entités), [Q22] (`Partner` → liste d'entités), [Q23] (CRUD complet pour toutes les dimensions + satellites).
