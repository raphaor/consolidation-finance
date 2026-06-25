# Plan d'action — Renommage des codes (clés primaires éditables)

> Statut : **analyse + plan** (branche `feat/renommage-codes`). Pas encore implémenté.
> Objectif : pouvoir **changer le `code` d'un objet** (entité, compte, flux, devise,
> caractéristique, etc.) après création, pour réorganiser/renommer sans tout recréer.

## 1. Le problème

Presque toutes les master data utilisent leur `code` (ou `code_iso`) comme **clé
primaire textuelle**, et tous les liens du modèle pointent vers cette clé par
**valeur** (pas de FK technique). Conséquences :

- l'UI verrouille le champ `code` en édition (`MasterDataPage.tsx` :
  `locked = isEdit && col.pk`) → impossible de renommer ;
- même sans verrou, un simple `UPDATE … SET code = …` laisserait **orphelines**
  toutes les lignes qui référencent l'ancien code (le modèle n'a pas de FK dures
  DuckDB, donc aucun `ON UPDATE CASCADE`).

`dim_consolidation` est la **seule** exception : elle est déjà passée à une PK
technique `id INTEGER` + clé naturelle `UNIQUE` (cf. `schema.rs`). C'est le
précédent à garder en tête (§5, option B).

## 2. Décision structurante (à trancher)

Deux stratégies. **Recommandation : option A.**

| | A — Renommage en cascade | B — Clés techniques (`id`) partout |
|---|---|---|
| Principe | Le `code` reste la PK ; on ajoute une opération « renommer » qui propage le changement partout. | Chaque dimension gagne un `id` auto ; `code` devient un attribut mutable `UNIQUE` ; les FK pointent vers `id`. |
| Ampleur | Ciblée : 1 module générique + UI. | Massive : tout le graphe de références, le pipeline SQL, les JSON de règles/formules, l'UI, les CSV/seed. |
| Aligné avec l'existant | Oui (codes partout, UI « affiche les codes », règles raisonnent en codes). | Non — combat la conception data-driven actuelle. |
| Coût d'un renommage | Opération transactionnelle ponctuelle. | Gratuit (le code n'est plus une clé). |
| Risque | Faut couvrir **toutes** les surfaces (graphe + JSON + noms d'objets). | Faut migrer **tout** le code + données existantes. |

L'intention exprimée (« pouvoir changer d'avis sur un nom ») = renommage, pas
abandon des codes. **Le reste de ce plan détaille l'option A.** L'option B reste
documentée en §5 si on veut y revenir.

## 3. Cartographie des surfaces touchées par un renommage

Un code peut jouer **trois rôles** distincts. Un renommage robuste doit traiter
les trois.

### 3.1 Rôle 1 — Valeur référencée par une colonne FK (le graphe)

Couvert par `references.rs` : `all_references(con)` = statiques (`REFERENCES`) +
dynamiques (caractéristiques N1/N2, références directes patron B). Pour renommer
`X → Y` dans `dim_foo.code`, il faut `UPDATE … SET col = Y WHERE col = X` sur
**chaque** `(table, column)` dont `target_table = dim_foo` et `target_column = code`,
**plus** la ligne PK elle-même, **plus** les colonnes des écritures
(`stg_entry`, `fact_entry`) et des satellites (`sat_perimeter`,
`sat_exchange_rate`, `sat_flow_scheme_item`).

✅ **Bonne nouvelle** : ce graphe existe déjà et est la source de vérité.
L'opération de renommage doit le **réutiliser** (ne jamais coder en dur la liste).
Cas particuliers à gérer : **auto-références** (`dim_entity.entite_parent`,
`compte_parent`, `sat_flow_scheme_item.flux_de_report`…) où la table source = cible.

### 3.2 Rôle 2 — Identifiant inséré dans un **nom d'objet** (table/colonne)

Certains codes ne sont pas seulement des valeurs : ils nomment des objets SQL.
Un renommage doit faire du **DDL** (`ALTER TABLE … RENAME`), pas juste un UPDATE.

| Source du code | Objet nommé d'après le code | Action de renommage |
|---|---|---|
| `dim_characteristic.code` | table `car_<code>` **+** colonne `<code>` sur la dimension de base | `ALTER TABLE … RENAME TO` + `RENAME COLUMN` + MAJ registre + MAJ `dim_characteristic_attribute.characteristic_code` |
| `dim_value_list.code` | table `lst_<code>` | `ALTER TABLE … RENAME TO` + MAJ registre + MAJ `dim_characteristic_attribute.target_dimension` qui pointe la liste |
| `dim_custom_dimension.name` | colonne `<name>` sur `fact_entry` **et** `stg_entry` | `ALTER TABLE … RENAME COLUMN` ×2 + MAJ registre |
| `dim_custom_reference.column_name` | colonne `<column_name>` sur la dimension hôte | (nom choisi librement, pas un « code » d'objet — renommage = `RENAME COLUMN` + registre) |

⚠️ Ces colonnes/tables dynamiques apparaissent **aussi** comme cibles dans le
graphe (rôle 1). Le renommage doit synchroniser les deux (le registre pilote la
ré-application après reset via `reapply` / `apply_custom_columns`).

### 3.3 Rôle 3 — Code **enfoui dans du JSON / une expression** (hors graphe)

C'est le point dur : ces codes ne sont **pas** dans `references.rs`.

| Colonne | Contenu | Codes enfouis |
|---|---|---|
| `dim_rule.definition` | JSON scope + operations | valeurs de dimension (`SelectionCond.val`, `ScopeCond.val`, `Destination.value`), codes de coefficient (`Coefficient::Named`), codes de caractéristique (`via`), noms de réf. directe (`ref`), noms d'attribut N2 (`attr`), niveaux (`level`) |
| `dim_coefficient.expression` | formule type Excel | opérandes `[code.perspective]` → colonnes `sat_perimeter`, valeurs de dimension éventuelles |
| `dim_aggregate.definition` | JSON `{level, selection[]}` | valeurs de dimension, `via`/`ref`/`attr` |
| `dim_indicator.expression` | formule | codes de **poste** (`dim_aggregate.code`) référencés en `[…]` |
| `dim_indicator.grain` | JSON | noms de dimensions |
| `app_config.value` (`pivot_currency`) | valeur scalaire | un `code_iso` de devise |

Traiter ce rôle demande de **parser/réécrire** ces structures lors d'un
renommage (ou, a minima, de **bloquer** le renommage d'un code encore cité ici
avec un message clair). Voir §4, étape 4.

### 3.4 Codes « réservés » (renommage interdit ou à effet structurel)

Certains codes sont référencés **par valeur dans le code Rust / le DDL**, donc
non librement renommables sans modifier le moteur :

- **Schémas de flux par défaut** `RESULTAT` / `BILAN` : codés en dur dans la vue
  `v_flow_behavior` (`COALESCE(... 'RESULTAT' ... 'BILAN')`, `schema.rs`).
- **Flux d'affichage** `F00/F80/F81/F99` : ordre/picking codés dans `report.rs`.
- **Coefficients natifs** (`pct_integration`, `elim_ic_*`) : seedés depuis
  `coefficients::BUILTINS`, cités dans les règles natives et `report.rs`. De plus
  `pct_integration` est **aussi** une colonne de `sat_perimeter` (collision
  nom coefficient / nom colonne — l'expression `[pct_integration.entity]`).
- **Enums** `classe` (`bilan/resultat/flux`), `taux_conversion`
  (`close_n1/avg/close_n`) : contraintes `CHECK`, pas des master data.
- **Devise pivot** : renommer la devise désignée par `app_config.pivot_currency`
  exige de mettre à jour aussi `app_config`.

→ Le renommage doit **refuser** (ou avertir fortement pour) les codes natifs /
réservés, identifiables via `kind = 'builtin'`, `native = TRUE`, ou une liste de
réserves dérivée du code.

## 4. Plan d'implémentation (option A)

### Étape 0 — Garde-fous & tests d'abord
- Test d'intégration : créer un mini-jeu (compte référencé par une écriture +
  une règle), renommer le code, vérifier **zéro orphelin** (`/api/meta/health`)
  et que le pipeline produit le **même** résultat qu'avant.
- Ces tests vivent dans `tests/` (mécanique pure, cf. stratégie de tests).

### Étape 1 — Moteur de renommage générique (rôle 1)
- Nouveau module `rename.rs` : `rename_code(con, dimension, old, new)`.
- Transaction unique. Étapes :
  1. valider `new` (format, non vide, ≠ old, pas déjà pris dans la dimension) ;
  2. refuser si `old` est natif/réservé (§3.4) ;
  3. via `all_references(con)`, `UPDATE` chaque colonne référençant la cible
     (ordre : enfants d'abord, puis la ligne PK — ou différer les contraintes) ;
  4. `UPDATE` la ligne PK elle-même.
- Réutiliser `references::value_exists` et la résolution `dimension_master`.
- Attention aux **auto-références** (même table) : un seul UPDATE couvre les deux
  (la colonne FK et la PK ne sont pas la même colonne → OK), mais valider
  l'ordre pour ne pas violer la PK transitoirement.

### Étape 2 — Renommage des objets nommés (rôle 2)
- Étendre `rename.rs` (ou hooks par type) pour les cas `car_`/`lst_`/custom dim/
  custom ref : `ALTER TABLE … RENAME` + MAJ des registres (`dim_characteristic`,
  `dim_value_list`, `dim_custom_dimension`, `dim_custom_reference`,
  `dim_characteristic_attribute`).
- Vérifier l'interaction avec `reapply` / `apply_custom_columns` (les registres
  doivent refléter le **nouveau** nom pour que la ré-application post-reset soit
  cohérente).

### Étape 3 — Détection des codes enfouis (rôle 3)
- Fonction `references_in_json(con, dimension, code) -> Vec<usage>` qui scanne
  `dim_rule.definition`, `dim_coefficient.expression`, `dim_aggregate.definition`,
  `dim_indicator.expression`/`grain`, `app_config`.
- **Décision de portée** (à trancher) :
  - **3a (sûr, livrable d'abord)** : *bloquer* le renommage si le code est cité,
    avec un rapport « utilisé dans : règle X, indicateur Y… ». L'utilisateur
    nettoie d'abord. Simple, pas de réécriture fragile.
  - **3b (confort, plus tard)** : *réécrire* automatiquement les JSON/expressions
    (réutiliser les parsers de `rules.rs` / `formula.rs`, jamais de regex naïve).
- Recommandation : livrer **3a**, puis 3b si le besoin se confirme.

### Étape 4 — API
- `POST /api/md/{table}/rename` body `{ old, new }` (ou `PUT` dédié). Ne **pas**
  réutiliser le `update` générique (qui ignore la PK).
- Réponses : 409 si `new` pris, 422 si natif/réservé, 409 + rapport si code
  enfoui (mode 3a).
- Renvoyer un récapitulatif : nb de lignes mises à jour par table.

### Étape 5 — Frontend
- `MasterDataPage.tsx` : bouton/action « Renommer le code » (modal `old → new`),
  distincte de l'édition de ligne (qui garde la PK verrouillée).
- Afficher le rapport d'usages bloquants (mode 3a).
- `api.ts` : `masterData.rename(table, old, new)`.

### Étape 6 — Docs
- `docs/MODELE_DONNEES.md` : noter que les codes sont renommables + la sémantique.
- `docs/QUESTIONS_OUVERTES.md` : acter la décision A vs B et la portée 3a/3b.
- Mentionner les codes réservés (§3.4).

## 5. Annexe — Option B (clés techniques) si on y revient
Migrer chaque dimension vers `id` auto + `code UNIQUE` mutable ; repointer toutes
les FK (graphe, écritures, satellites) vers `id` ; adapter le pipeline SQL, les
JSON de règles/formules (qui devraient alors stocker des `id`, perte de lisibilité),
l'UI et les imports CSV (qui fournissent des codes). Très coûteux et en tension
avec la conception data-driven actuelle. Seul `dim_consolidation` l'a fait, parce
que son identité est une **clé naturelle composite** (pas un code unique).

## 6. Risques & points de vigilance
- **Transactionnalité** : un renommage partiel corrompt l'intégrité → tout en une
  transaction, rollback sur erreur.
- **Verrou serveur** : `Arc<Mutex<Connection>>` → l'opération est déjà sérialisée.
- **Codes enfouis** = principale source de bugs silencieux ; commencer par 3a.
- **Perf** : un renommage touche potentiellement `fact_entry` (millions de lignes)
  → un `UPDATE … WHERE col = old` sur colonne non indexée reste un scan, mais
  c'est une opération rare et hors chemin de consolidation. Acceptable.
- **Collision nom coefficient / colonne** (`pct_integration`) : bien cloisonner
  les espaces de noms lors de la détection rôle 3.
