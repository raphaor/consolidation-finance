# Questions ouvertes

Registre des décisions à prendre pour avancer sur le prototype / POC.
L'`EXPRESSION_DE_BESOIN.md` reste la source fonctionnelle ; ce fichier trace ce qui reste flou et les arbitrages rendus.

## Légende

- **Priorité** : `BLOC` (empêche d'écrire le POC) · `TÔT` (à trancher avant la 1ʳᵉ version) · `POST` (après le POC) · `HORS` (explicitement hors périmètre prototype).
- **Statut** : `OUVERTE` · `EN RÉFLEXION` · `TRANCHÉE` (alors remplir *Décision* et dater).

---

## Blocantes (à traiter en premier)

| ID | Question | Contexte | Priorité | Statut | Décision |
|---|---|---|---|---|---|
| Q1 | Quel **périmètre du MVP** livrer en premier ? (sous-ensemble d'opérations, de scénarios, d'entités) | EDB *Prochaines étapes* | BLOC | TRANCHÉE | **MVP = réel seul** + traitements **natifs** (agrégation, conversion multi-devises, méthodes globale/proportionnelle/équivalence, variations de périmètre) + restitutions (table filtrable, bilan par flux, compte de résultat) + CRUD master data complet + import CSV. Volumétrie large, perf = critère. Éditeur de règles, multi-scénarios, fusions/IFRS5 = post-MVP. Voir EDB *MVP / POC*. |
| Q2 | Quelles **restitutions** le POC doit-il produire exactement ? (lignes filtrables ? bilan ? tous ?) | EDB §5 | BLOC | TRANCHÉE | **3 sorties POC** : (1) table filtrable ; (2) bilan par flux (comptes en lignes, flux en colonnes) ; (3) compte de résultat (flux ouverture + clôture). TFT / annexe / dashboards reportés post-POC. Voir EDB §5. |
| Q3 | **Volumétrie cible** : nb d'entités, nb de comptes, nb de lignes, nb de périodes conservées ? | EDB §6 | BLOC | TRANCHÉE | **Large** : 50+ entités, milliers de comptes, millions de lignes. La performance est un critère de validation du POC → test sur gros volumes (fait monter [Q12] en priorité). |
| Q4 | **Source et saisie des taux de change** : qui les fournit, sous quel format, à quelle fréquence (clôture / moyenne pondérée) ? | EDB §3.3 — opé B « conversion multi-devises » | BLOC | TRANCHÉE | **Taux clôture + taux moyen (moyenne simple sur la période)**, stockés par `Currency × Period`. Application : clôture → bilan, moyen → résultat. **Saisie : CRUD + import CSV** (pas de fetch auto au POC). Voir [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §4. |
| Q5 | Comment est **représenté le périmètre de consolidation** (mère / filiales, méthode par entité, % d'intérêt, entrées/sorties datées) ? | EDB §3.2 | BLOC | TRANCHÉE | Table *Périmètre* : `méthode`, `%_intérêt`, `%_intégration`, `entrée_sortie_mid_exercice`, `fusion` (absorbante/absorbée). Variations calculées par comparaison scope N vs. consolidation d'ouverture (N-1). **Clé révisée (Q35, 2026-06-21)** : versionnée par `perimeter_set` → `(perimeter_set, entity, period)`, le scénario référence le jeu. Voir [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §4. |

## À trancher tôt (avant la 1ʳᵉ version du POC)

| ID | Question | Contexte | Priorité | Statut | Décision |
|---|---|---|---|---|---|
| Q6 | **Mode de consolidation** pour le POC : `complète` seule, ou `à la marge` aussi ? | EDB §3.5 | TÔT | TRANCHÉE | **Complète seule** (recalcul total à chaque soumission). `À la marge` reporté post-MVP. |
| Q7 | Les écritures C (retraitements, interco, variations de capital, répartition résultats) : dans le MVP ou reportées ? | EDB §3.4 | TÔT | TRANCHÉE | **Reportées** : construites via l'**éditeur de règles de consolidation** (module post-MVP, [Q24]). Le MVP ne contient que les traitements **natifs** (agrégation, conversion multi-devises, méthodes de conso, variations de périmètre). Dichotomie B/C abandonnée au profit de **natif vs éditeur de règles**. |
| Q8 | Le **workflow de validation** (brouillon / soumis) est-il nécessaire au POC, ou toutes les liasses sont-elles considérées validées ? | EDB §3.5 | TÔT | TRANCHÉE | **Aucun workflow au MVP** : toute liasse/écriture est immédiatement intégrée à la conso. Workflow reporté en évolution. |
| Q9 | **Granularité de clôture par défaut** pour le POC : mensuel, trimestriel, annuel ? (et prévisionnel multi-années : oui/non ?) | EDB §3.5 | TÔT | TRANCHÉE | **Annuel seul** au MVP. Mensuel/trimestriel = post-MVP (le moteur les gère déjà via `Period`, c'est surtout des données). Prévisionnel multi-années lié au scénario budget → post-MVP. |
| Q10 | Comment les **opérations interco** sont-elles détectées ? (via le champ `Partner*` ? règle de matching ?) | EDB §3.4 / §4 — champ `Partner*` présent mais sémantique implicite | TÔT | TRANCHÉE | **Via le champ `Partner` + l'éditeur de règles** (pas de matching automatique en dur). Une règle scope sur `entity`/`partner` (joints à `sat_perimeter`) et sélectionne sur `partner IS NOT NULL` ; cf. `prototype/rust/src/rules.rs` (exemple d'élimination interco intégré). Répondue de fait avec l'implémentation de l'éditeur de règles ([Q24], 2026-06-19 ; statut acté 2026-06-20). |
| Q11 | **Types de consolidation couverts au POC** : réel seul, ou réel + budget + prévision ? | EDB §3.1 | TÔT | TRANCHÉE | **Réel seul** au MVP. Budget / prévision / multi-scénarios en post-MVP. |
| Q20 | `Entry_period` : **dimension distincte** ou sous-type de `Period` (type « exercice ») ? Éviter la redondance. | [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 | TÔT | TRANCHÉE | **Une seule table `Period`** ; `Entry_period` et `Period` sont deux rôles (clés étrangères) vers elle, `Entry_period` contraint au type « exercice ». |
| Q21 | `Share` : participe à la table *Périmètre*, ou pointe vers une table **Participations** dédiée ? Risque de duplication des % / méthodes. | [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3-§4 | TÔT | TRANCHÉE | **Pas de table Participations séparée.** `Entity`, `Partner`, `Share` = 3 rôles sur une liste centrale d'entités (groupe + tiers liés). Détail des participations (%, méthode, dates, fusion) porté par le *Périmètre*. |
| Q22 | `Partner` : référence **directement une `Entity`** (quand c'est une entité du groupe) ou via une **table de tiers** séparée ? | [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 | TÔT | TRANCHÉE | **Référence la liste centrale d'entités** (rôle `Partner` sur la même master data). |
| Q23 | **Portée de l'interface master data** dans le MVP : quelles dimensions/tables satellites ont un écran de gestion dès le POC ? | [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §2 | TÔT | TRANCHÉE | **Écran CRUD complet pour chaque dimension et chaque table satellite.** |
| Q29 | **Dimension Nature** : modèle, règles d'agrégation, convention de nommage. | [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 — champ `Nature` obligatoire | TÔT | TRANCHÉE | **Dimension `Nature` obligatoire** sur toutes les écritures. Table `dim_nature` (`code`, `libellé`, `rules`). Convention de nommage : préfixe `0`/`1`/`2`/`3`/`4` = étape de chargement (liasse → avant reclass → après reclass → après converti → après cons). **Jamais agrégée** entre natures : `nature` entre dans le `GROUP BY` de chaque étape du pipeline et dans le grain de reconstruction F99. Valeurs de base : `0LIASS`, `1AJUST`. Champ `rules` (JSON) réservé au futur module de traitement automatique. **Implémentée le 2026-06-17** (commit `3e46316`) : table, champ obligatoire, agrégation séparée, filtre sur toutes les restitutions, smoke test 59/59. **Staging (routing par préfixe)** documenté dans [`FLUX_CONSO.md`](./FLUX_CONSO.md) « Staging — Injection par nature » ; reporté avec le module de règles ([Q24](./QUESTIONS_OUVERTES.md)). Voir [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 `Nature`. |

## Élaboration (mécanismes à détailler avant implémentation)

Pas des décisions de scope, mais des **deep-dives de conception** nécessaires pour coder correctement le moteur. À traiter avant / pendant la conception technique.

| ID | Sujet | Contexte | Statut | Détail |
|---|---|---|---|---|
| Q25 | **Conversion multi-devises × flux** : mécanique des flux de conversion **F80** (écart ouverture→clôture) et **F81** (taux moyen → clôture). Taux appliqués, formule de l'écart, imputation. | [`FLUX_CONSO.md`](./FLUX_CONSO.md) · [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3-§4 | TRANCHÉE | Mécanique complète : `écart = A × (r_clôture − r_flux)`. F00→F80, F20→F81, F01→F80 (clôture N-1), F98 terminal (clôture N). Voir FLUX_CONSO §2. |
| Q31 | **À-nouveau (report d'ouverture)** : comment la clôture N-1 alimente l'ouverture N. | [`FLUX_CONSO.md`](./FLUX_CONSO.md) §4 | TRANCHÉE (spec, à implémenter) | **Spec : [`A_NOUVEAU.md`](./A_NOUVEAU.md)** (2026-06-20). Décisions : (1) à-nouveau = conso N-1 **figée** (snapshot), lue par niveau — impose d'isoler la purge `fact_entry` par scénario ; (2) piloté par `dim_flow.flux_a_nouveau` (F99→F00, générique) + `dim_scenario.a_nouveau_scenario` (FK nullable) ; (3) F99 N-1 collé sur F00 N **au corporate** (montant autoritaire, écrase la liasse ; base de l'écart F80 et du report F99) **et au consolidé** (fige le % N-1) — le **converti se déduit par conversion normale**, entités présentes en N-1 seulement ; seule la consolidation exempte le F00 du `× pct` ; (4) **suppression de l'étape reclass native** → pipeline corporate→converti, le niveau `reclassified` disparaît ; (5) F00→F01, miroir F98 et variation de % (F90/F95) deviennent des **règles**. A1/A2/A3/A6 résolus (écart F80 = montant corporate × delta exact ; `reclassified` supprimé du programme entier ; entrant **dérivé** de l'absence au snapshot N-1 + **contrôle de cohérence** dans `validate.rs` ; pas de marqueur sur les F00 — non-duplication garantie à la source via la présence au snapshot N-1) ; A4 hors moteur (flux de variation % = paramétrage de règle) et A5 (statut `ouvert` toléré, simple avertissement) résolus — **tous les points A1–A6 tranchés**. **Question clé tranchée** : « entité consolidée en N-1 » = présence d'un F99 consolidé pour elle dans le snapshot de l'à-nouveau (plus robuste que relire `sat_perimeter` N-1). Les natures d'ouverture (y compris natures d'élimination) sont **conservées** dans le F00 reporté (continuité + piste d'audit). **Staging redéfini** (intérimaire, préfixe de nature jugé fragile, à retravailler) sur 3 niveaux : `0`/`1`→corporate, `2`→converti (avant écarts — la règle de test des écarts est à recaler), `3`→consolidé avant %, `4`→consolidé après %. **Nouveau filtre de scope** à l'agrégation corporate (entités du périmètre, toutes méthodes, entrantes/sortantes incluses). Cf. `A_NOUVEAU.md` §4 bis. |
| Q26 | **Élaboration de la consolidation** : mécanismes détaillés au fil de l'eau. | EDB §3.4 | TRANCHÉE (partiel) | **Reclassifications de périmètre AVANT conversion** (décision 2026-06-15, simulation dans [`ANALYSE_RECLASSIFICATION_CONVERSION.md`](./archive/analyses/ANALYSE_RECLASSIFICATION_CONVERSION.md)). **4 niveaux de stockage** (corporate → reclassifié → converti → consolidé), chacun persisté. La reclassification (B) et la conversion (C) sont des étapes distinctes produisant chacune leur niveau de stockage. Le niveau *reclassifié* (devise fonctionnelle, après périmètre) est conservé car utile pour l'audit et la re-conversion. Raisons du choix : (1) traçabilité — évite les écarts F80/F81 orphelins ; (2) sortie de périmètre propre en devise fonctionnelle ; (3) implémentation simple. **Capturé** : modèle des flux + écarts F80/F81 ; variations de périmètre F01/F98 ; application des méthodes (globale, proportionnelle). **Fusion (F07/F70) reportée hors moteur de consolidation**, vers la construction du jeu de règles ([Q24]/[Q27], décision 2026-06-20) : spec conservée dans [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9 mais pas implémentée en natif. **Sortie de périmètre (précisée 2026-06-16)** : la sortante garde ses constituants F00/F20 à l'identique + génère un miroir −X sur F98 par constituant (générique via `flux_de_report`, pas de liste en dur), donc `F98 = −Σ(constituants)` et `F99 = 0` par identité — le solde ne fuit pas dans F99 (en fonctionnel comme en présentation). **Mise en équivalence reportée au post-MVP** (décision 2026-06-16 : alignement de l'EDB sur le code, qui ne traite que globale/proportionnelle ; spec conservée dans [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9). **Niveau d'application des règles** : spécifié dans [`REGLES_CONSO.md`](./REGLES_CONSO.md) §5 — les règles sélectionnent au niveau où elles s'appliquent ; les automatismes du niveau s'exécutent **avant** les règles, et les écritures générées **ne re-déclenchent pas** les automatismes. |

## Évolutions en cours (décidées 2026-06-21)

Quatre besoins remontés en lot. Fil rouge : trois d'entre eux sont le **même patron** « jeu versionné nommé, référencé par le scénario » déjà amorcé par `dim_rate_set`. Implémentés **dans l'ordre** Q33 → Q32 → Q35 → Q34.

| ID | Sujet | Contexte | Statut | Décision |
|---|---|---|---|---|
| Q33 | **Méthodes de consolidation pilotables + trigger de règles sur la mère.** Besoin : créer librement des méthodes ; déclencher des règles seulement sur certaines entités (ex. la consolidante). | `consolidate.rs`, `dim_method`, `rules.rs` | IMPLÉMENTÉE (2026-06-21, suite verte) | `dim_method` **est déjà** la master data et `step_d` est déjà data-driven (`JOIN dim_method WHERE consolidated`). Seul blocage : le `CHECK (methode IN (…))` en dur sur `sat_perimeter.methode` (`schema.rs`) duplique `dim_method` → **supprimé**, l'intégrité reste assurée par le graphe de références (`sat_perimeter.methode → dim_method`). « Trigger sur la mère » : **aucun changement moteur** — le scope d'une règle filtre déjà sur `methode` ; on donne à la consolidante une méthode dédiée (ex. `MERE`, `consolidated=true`, même mécanique `×pct`) et on scope les règles « mère » sur `methode = 'MERE'`. Toutes les méthodes appliquent le même `× pct_integration` ; la différenciation métier passe par les règles. |
| Q32 | **Taux moyen sur le P&L → schémas de flux.** La conversion applique un taux **par flux** (`dim_flow.taux_conversion`), or un compte de résultat veut F20/F99 au **taux moyen sans écart**, un compte de bilan veut F20 moyen **+ écart F81** vers la clôture. | `convert.rs`, `dim_flow`, `dim_account` | IMPLÉMENTÉE (2026-06-21, suite verte) | **Modèle (révisé 2026-06-21)** : `dim_flow` redevient une **dimension nue** (`code`, `libellé`) ; **tout** le comportement d'un flux (`taux_conversion`, `flux_ecart`, `flux_de_report`, `flux_a_nouveau`) est **déporté dans le schéma de flux**, qui est une **articulation complète** des flux (pas une surcharge éparse). Tables : `dim_flow_scheme (code, libelle)` + `sat_flow_scheme_item (scheme, flow, taux_conversion, flux_ecart, flux_de_report, flux_a_nouveau)`. Le compte porte `dim_account.flow_scheme` (NULL = défaut dérivé de la classe : `resultat` → `RESULTAT`, sinon `BILAN`). La résolution **par compte** se fait via la **vue `v_flow_behavior(account, flow, …)`**, consommée par `convert` (taux + écart), `materialize_closures` (report de clôture, résolu par compte car `account` est dans le grain) et `a_nouveau` (report d'ouverture, avec **garde par compte** : seul le bilan reporte F99 → F00, le résultat non). Schéma `BILAN` = ex-`dim_flow` (iso-comportement, suite golden verte) ; `RESULTAT` = tout au taux moyen, sans écart, **sans à-nouveau**. **Invariant** : un schéma doit être complet (tous les flux de ses comptes). **Décision comptable** : l'écart de conversion du résultat **naît sur les capitaux propres « résultat » au bilan** (config/recette), **pas** sur le P&L (moteur). |
| Q35 | **Périmètres versionnés (période + version).** Réutiliser un même périmètre entre scénarios/variantes, comme les taux. | `sat_perimeter`, `dim_scenario`, `consolidate.rs` | IMPLÉMENTÉE (2026-06-21, suite verte) | Appliquer le **patron `rate_set`** : `dim_perimeter_set (code, libelle)` + re-clé `sat_perimeter` en `(perimeter_set, entity, period)` + `dim_scenario.perimeter_set`. Le run résout scénario → `perimeter_set` → lignes (symétrique à `rate_set`). Impacts : `schema.rs`, `seed.rs`, `references.rs`, `load_params`, `JOIN sat_perimeter` de `step_d`, `import.rs`, CRUD/UI. Un scénario devient proprement *la composition de jeux versionnés nommés* (`rate_set` ✓, `perimeter_set` nouveau, `ruleset` ✓, `variant` ✓). |
| Q34 | **Plusieurs tables de taux par période + version.** | `dim_rate_set`, `sat_exchange_rate` | TRANCHÉE (déjà en place) | **Déjà le modèle actuel** : `dim_rate_set` = table des versions, **période dans la PK** de `sat_exchange_rate (rate_set, currency_source, period)`, `dim_scenario.rate_set` choisit la version. Rien à construire côté moteur — **vérifier seulement** que le CRUD/UI de `dim_rate_set` et l'import CSV par `rate_set` sont exposés. |

## Évolutions en cours (décidées 2026-06-22)

| ID | Sujet | Contexte | Statut | Décision |
|---|---|---|---|---|
| Q36 | **Saisie manuelle d'écritures** : permettre l'ajout / édition / suppression unitaire d'écritures depuis l'IU, sans passer par l'import CSV. Protection anti-écrasement des données existantes. | EDB §3.3 (saisie des liasses) · page Écritures jusqu'ici strictement en lecture | IMPLÉMENTÉE (2026-06-22) | **Vue dédiée « Saisie »** (option B) — `web/src/pages/SaisiePage.tsx` (onglet `saisie` dans le nav). Trois endpoints : `POST /api/entries` (batch), `PUT /api/entries/{id}`, `DELETE /api/entries/{id}` (handlers dans `prototype/rust/src/entries.rs`). Cible : `stg_entry` (niveau `raw`), avec **PK auto-incrémentée `id`** (seq dédiée `seq_stg_entry`, distincte de `seq_entry` pour éviter les collisions) — donc `get_entries` niveau `raw` renvoie maintenant le vrai id au lieu d'un `ROW_NUMBER()` synthétique. Marqueur de provenance : **`source = "MANUAL"`** forcé à l'INSERT (réutilise le champ existant, non propagé par le pipeline —cf. [Q13]). **Pipeline non relancé** automatiquement (l'utilisateur déclenche `/api/run` lui-même). **Protection anti-écrasement** : PUT/DELETE refusés sur les lignes dont `source ≠ MANUAL` (protection des imports CSV). Insert-only sur le POST (jamais d'écrasement). Validation : champs obligatoires + cohérence référentielle (FK) au POST/PUT, dans une transaction au POST (lot atomique). Nouveau param `source` sur `GET /api/entries` pour filtrer. Distinction visuelle : `EcrituresPage` surligne les lignes `source=MANUAL` (classe `row--manual`). Hook `useDimValues` et `DimRefContext` factorisés dans `web/src/hooks/useDimValues.tsx` (partagés RulesPage ↔ SaisiePage). **Impact schéma** : `stg_entry` gagne `id` PK — un `POST /api/reset` ou `CONSO_FORCE_RESEED=1` est nécessaire après rebuild pour reconstruire le schéma. **Évolution UX (complément même journée)** : en-tête commun factorisé (Scénario, Entité, Exercice, Période, Devise, Nature) en haut du batch, qui pré-remplit chaque nouvelle ligne ajoutée ; grille allégée par défaut (seulement Account, Flow, Partner, Share, Analysis, Analysis2, Amount) avec toggle pour afficher les colonnes communes (override au cas par cas) ; bouton « ↧ Appliquer partout » pour propager l'en-tête aux lignes existantes du batch. Pré-remplissage depuis les filtres partagés `ecritures.*` (scenario/entity/entry_period/period) + nouveaux états persistés `saisie.currency` et `saisie.nature`. |
| Q37 | **Libellé de la dimension `share`** : « Quote-part » est une traduction ambiguë de l'anglais *share* (qui désigne ici un titre / une part juridique, pas une quote-part au sens financier). | `dimensions.rs::builtin_dims()` · libellé affiché dans Rules + Saisie + Ecritures | TRANCHÉE (2026-06-22) | **Renommé « Titre »** dans `prototype/rust/src/dimensions.rs` (source serveur), `web/src/pages/RulesPage.tsx` (fallback) et `web/src/pages/SaisiePage.tsx` (SHORT_LABELS). Le nom technique `share` reste inchangé (pas d'impact modèle de données). Les libellés de comptes PCG contenant « Quote-part » (ex. 655, 755) ne sont pas touchés (libellés métier du PCG, pas la dimension). |
| Q38 | **Affichage des dropdowns** : afficher `code` seul rend l'identification visuelle difficile pour l'utilisateur métier (le code technique est opaque). | Tous les `<select>` alimentés par master data : Rules, Saisie, Filters, Master data, Pipeline | TRANCHÉE (2026-06-22) | **Format uniforme `code - libellé`** dans tous les dropdowns alimentés par une dimension master data. Helper central `formatOptionLabel(code, libelle)` dans `web/src/utils/format.ts` (renvoie juste `code` si libellé vide ou identique au code). `useDimValues` (`web/src/hooks/useDimValues.tsx`) modifié pour exposer `{code, libelle}[]` au lieu de `string[]` (charge le `libelle` de chaque master data, cache module-level mis à jour). Consommateurs mis à jour : `SaisiePage.DimCell`, `RulesPage.ValueField` + `OverrideValueField`, `FieldInput` (master data), `Filters` (5 dropdowns), `PipelinePage` (scénarios, anciennement `libelle (code)` inversé). Les dropdowns **techniques** (level, opérateur, type de coefficient, méthodes de destination…) restent en code seul car ils exposent des enums techniques sans libellé métier. |
| Q39 | **Éditeur de règles — sélection par attribut et mode `map_ref`** : la sélection ne permettait que les filtres directs sur la valeur d'une dimension (`account = "1000"`), et la destination `map` n'existait que pour les caractéristiques N1/N2 (avec table intermédiaire `car_<code>`). Pas de mécanisme équivalent pour les **références directes** (patron B, ex. `compte_parent`) — ni en sélection, ni en destination. | Évolutions de [Q24] ; cf. [`REGLES_CONSO.md`](./REGLES_CONSO.md) §4.1 (sélection étendue), §4.3 (mode `map_ref`) et §9 (UI) | TRANCHÉE (2026-06-22) | **Trois extensions livrées** (ratrapant le retard du patron B sur le patron A) : (1) **Sélection étendue** — filtre indirect par attribut traversé, via `via` (caractéristique N1 : `comportement = VENTES_IC`) ou `ref` (référence directe patron B : `compte_parent = 60`), mutuellement exclusives. INNER JOIN : un membre non classé / sans valeur de référence n'est pas sélectionné. Validation runtime + à l'enregistrement (cible référentielle adaptée). (2) **Destination `map_ref`** — 5ᵉ mode pour traverser une référence directe (`ref`), symétrique de `map` mais avec **un seul JOIN** sur la master data (pas de table intermédiaire). Auto-référence requise (`host = target = dim` écrit) + INNER JOIN + `IS NOT NULL` sur la colonne. (3) **UI** : dropdown « Traverser » (optgroups N1/ref) + multi-select repliable pour `IN` (tous cas : direct, via, ref) avec cases à cocher et fermeture au clic extérieur. Implémentation : `rules.rs` (parsing/validation/`exec_operation`), `custom_references.rs::target_of` (helper), `RulesPage.tsx` (composants `MultiSelectDropdown` + `SelectionValueField`), `useDimValues.tsx` (hooks `useCharacteristicValues` + `useSelectionValues`). Tests Rust : 6 unitaires (parsing) + 3 intégration (`destination_map_ref_traverse_reference_directe`, `selection_via_n1_filtre_par_valeur_de_caracteristique`, `selection_via_ref_filtre_par_reference_directe`). |
| Q40 | **Coefficients d'élimination IC au plus faible taux d'intégration** : l'éditeur n'offrait que `pct_integration` / `pct_interet` (lus sur la seule entité). Besoin d'éliminer l'interco au prorata `Min(1, INTEG_PA / INTEG_EN)` des **deux** entités liées, et de la **variation N vs N-1** (lié à l'à-nouveau). Question de modélisation : où lire le taux N-1 ? | Évolutions de [Q24] ; cf. [`REGLES_CONSO.md`](./REGLES_CONSO.md) §4.2 ; lien à-nouveau [Q31]/[`A_NOUVEAU.md`](./A_NOUVEAU.md) | TRANCHÉE (2026-06-22) | **3 nouveaux coefficients** `elim_ic_corp_n` = `Min(1, INTEG_PA_N / INTEG_EN_N)`, `elim_ic_corp_n1` (idem N-1), `elim_ic_corp_var` = `n − n1`. **Décision N-1 (tranchée avec l'utilisateur)** : le taux N-1 est lu via le **lien à-nouveau existant** (`dim_scenario.a_nouveau_scenario` → son `perimeter_set` à son `entry_period`), **pas** stocké dans le fait — réutilise le snapshot N-1 du carry (`pipeline/a_nouveau.rs`), zéro schéma/donnée en plus, cohérence garantie. Écarté : dénormaliser le `pct_integration` dans `fact_entry`. Dégradations : entité/partenaire absent du périmètre N-1 (entrant) ou scénario sans à-nouveau → taux N-1 = 0 (`var = n`) ; `INTEG_EN = 0` → coefficient 0 (pas de division par zéro). Coefficient **agnostique au niveau** (le `Min(1, PA/EN)` corporate redevient `Min(INTEG_EN, INTEG_PA)` après le × de l'étape D). Implémentation : `rules.rs` (`Coefficient::ElimIcCorp*`, `coefficient_expr` → `CoeffJoins`, helper `min_ratio` évitant `LEAST` qui ignore NULL sous DuckDB, JOINs `p_part` / `p_ent_n1` / `p_part_n1`), `RulesPage.tsx` (`COEFF_TYPES`). **Bug latent corrigé au passage** : le coefficient n'était pas parenthésé dans `e.amount * {coeff} * {mult}` → un coefficient à opérateur de tête (la soustraction de `var`) était mal associé ; désormais `({coeff})`. Tests : 2 unitaires (parsing + besoins de JOIN) + 2 intégration (`coefficient_elim_ic_corp_n_n1_var`, `coefficient_elim_ic_corp_sans_a_nouveau_n1_nul`). |

## Évolutions en cours (décidées 2026-06-23)

Refonte du modèle d'identité (scenario → consolidation) et du taux d'ouverture. Deux temps livrés et validés (`cargo test` 115 verts, `npm run build` OK, dump_pipeline et smoke serveur OK).

| ID | Sujet | Contexte | Statut | Décision |
|---|---|---|---|---|
| Q41 | **Identité : scenario → consolidation + remontée.** Le `code` scénario ne portait pas de sens ; la saisie était rattachée à un scénario au lieu d'une remontée ; la période du périmètre et des taux était implicite. | `dim_scenario`, `stg_entry.scenario`, `ConvertParams` | IMPLÉMENTÉE (2026-06-23) | **Nouveaux concepts** : (1) **Remontée** = maille élémentaire des saisies = `Phase` + `Exercice` (pas de table dédiée — portée par `stg_entry`). (2) **Consolidation** = `dim_consolidation` (ex `dim_scenario`), PK technique `id` auto (l'ancien `code` disparaît), **clé naturelle UNIQUE** `(phase, exercice, perimeter_set, variant, presentation_currency)` ; `category`→`phase`, `entry_period`→`exercice`, `a_nouveau_scenario`→`a_nouveau_consolidation_id` (entier). (3) **Périodes explicites** : `perimeter_period` + `rate_period` (défaut = exercice) remplacent l'`entry_period` implicite. (4) `stg_entry.scenario`→`phase` (saisies au grain remontée, partagées entre consolidations) ; `fact_entry.scenario`→`phase` (dim propagée) **+ `consolidation_id`** (col. technique, isole chaque run). (5) Isolation du pipeline par `consolidation_id` ; filtre remontée `phase`+`entry_period` ; grain de clôture = `consolidation_id` ++ grain. `dim_scenario_category` **conservé** (catalogue des phases). API : `/api/scenarios`→`/api/consolidations`, `/api/run` prend `consolidation_id` (entier). Export bump `conso-export-v2`. **Reset requis** (schema incompatible). |
| Q42 | **Taux d'ouverture porté par N (fin du `prev_period`).** Un run exigeait une période N-1 dans `dim_period` pour dériver `close_n1` → bloquait toute 1ʳᵉ consolidation et tout run sans à-nouveau (ex. erreur *« Query returned no rows »* sur REEL_2023). | `convert.rs` (close_n1), `ConvertParams.prev_period`, `sat_exchange_rate` | IMPLÉMENTÉE (2026-06-23) | **Nouvelle colonne `sat_exchange_rate.taux_ouverture`** (= clôture N-1) **portée par la période N**. La branche `close_n1` de `convert.rs` (F00/F01) lit `taux_ouverture` au lieu du taux N-1 dérivé. **Suppression de `ConvertParams.prev_period`** et de sa requête de dérivation : aucune période antérieure requise → 1ʳᵉ consolidation possible, avec ou sans à-nouveau. `taux_ouverture(N) ≡ taux_close(N-1)` : économie préservée (dump F00 A/USD = 4600, F80 = −100). L'à-nouveau lui-même n'utilisait pas `prev_period` (il lit le snapshot) — seule la conversion était concernée. Supède les mentions `prev_period`/close_n1 dérivé de Q25/Q31. |

## Évolutions en cours (décidées 2026-06-24)

| ID | Sujet | Contexte | Statut | Décision |
|---|---|---|---|---|
| Q43 | **Moteur de formules (coefficients utilisateur & indicateurs)** : besoin d'un créateur de formules type Excel, ergonomique (inspiration Pigment), pour exprimer des calculs. Domaine large : KPI et coefficients. | EDB §3.1 (conso de gestion / KPI) · [`REGLES_CONSO.md`](./REGLES_CONSO.md) §4.2 (coefficients en dur) · pending-improvements | **VOLETS 1 (coefficients) ET 2 (indicateurs/KPI) IMPLÉMENTÉS (2026-06-24)** | **Spec : [`FORMULES.md`](./FORMULES.md).** Cadrage tranché avec l'utilisateur : **un seul moteur de formules** (lexer/parser/AST/éditeur), **deux catalogues d'opérandes** selon le contexte. **Volet 1 prioritaire — coefficients utilisateur** : généralise l'enum `rules.rs::Coefficient` en dur (les `elim_ic_corp_*` sont déjà des formules manuscrites) ; opérandes = valeurs de `sat_perimeter` aux 4 perspectives (`EN`/`PA`/`EN_N1`/`PA_N1`, réutilise `CoeffJoins`) ; compile vers `(expr_sql, CoeffJoins)` — **point d'insertion exact de `coefficient_expr`** ; bibliothèque `dim_coefficient` (natifs seedés + utilisateur), immutabilité façon règles. **Volet 2 (phase 2) — indicateurs/KPI** : postes (agrégats nommés réutilisant `SelectionCond`) + indicateurs (formules), compilés en SQL au grain ; **non-additivité** des ratios (pendant du « of which ») ; **jamais réinjectés dans `fact_entry`**. Langage Excel (`+ − × ÷`, `MIN`/`MAX`/`IF`/`ABS`/`ROUND`/`SAFE_DIV`, références `[ … ]`, séparateur d'args `;`). Ergonomie : barre de formule autocomplétée, panneau de références, **preview live**, validation inline (miroir `validate_definition`). Sécurité SQL : identifiants whitelistés, seules les constantes émises en littéral. Questions de conception **F1–F5 toutes tranchées (2026-06-24)** : **F1** précision `f64` (pas de `DECIMAL` ; le produit `montant × coeff` reste `Decimal`) ; **F2** bibliothèque nommée `dim_coefficient` (+ `constant` inline) ; **F3** défaut **uniforme = 0** pour tout taux de périmètre absent (pas de neutralité magique, pas de protection auto contre `/0` — `SAFE_DIV` disponible mais à la charge de l'utilisateur ; **change** le comportement actuel où `pct_integration`/`pct_interet` solo valaient `1.0`) ; **F4** coefficient **modifiable en place** (réglage vivant, pas de versioning de formule — simplicité POC) ; **F5** N-1 = **opérande nommé** résolu via l'à-nouveau (pas de `PREV()`, même patron que le N-1 des coefficients ; zéro impact phase 1). Détail dans `FORMULES.md` §8. |

## Évolutions en cours (décidées 2026-06-25)

Data-driving des **valeurs natives structurantes** codées en dur dans le moteur. Fil rouge : ces valeurs bloquent les flips B1 de `sous_classe` / `flow_scheme` / `currency` (chantier codes-renammables) et verrouillent des codes master data. Vérification préalable : les **codes flux** (`F00`/`F99`…) et les **méthodes** (`globale`…) ne sont **pas** en dur dans la logique moteur (déjà data-driven via `sat_flow_scheme_item` / flag `consolidated`) — reste seulement les 3 cas ci-dessous + l'enum `taux_conversion` (close_n1/avg/close_n → colonnes de taux, structurel par nature, immuable).

| ID | Sujet | Contexte | Statut | Décision |
|---|---|---|---|---|
| Q44 | **Sens comptable user-driven** : retirer le dur `SENS_CASE` des rapports (bilan/P&L). | `server.rs:93` (CASE sur `actif`/`passif`/`charges`/`produits` → C/D) · bloque le flip B1 de `sous_classe` | TRANCHÉE (2026-06-25) | Colonne **`sens` (`C`/`D`/NULL) sur `dim_sous_classe`**, éditable en master data. Les rapports signent via `JOIN dim_sous_classe … sens` (plus de CASE en dur). La **`classe`** (enum immuable `bilan`/`resultat`/`flux`) reste attribut de compte mais **n'alimente plus les rapports** — son rôle « sens » est supprimé au profit de `sous_classe.sens` (user-driven). Prérequis au flip B1 de `sous_classe` (plus de dur à préserver). **Prototype à valider** (cf. `docs/archive/specs-livrees/PLAN_RENOMMAGE_CODES.md`). |
| Q45 | **`flow_scheme` sans défaut** : la vue `v_flow_behavior` perd son `COALESCE(…, CASE classe → RESULTAT/BILAN)`. | `schema.rs:225` (défaut hardcoded) · bloquait le flip B1 de `flow_scheme` | TRANCHÉE + IMPL (2026-06-26) | `flow_scheme` est **100 % user-driven** : la vue joint directement `sat_flow_scheme_item ON scheme = a.flow_scheme`. **Sous-choix (b) tranché** (voir [`FLOW_SCHEME_EXPLICITE.md`](./archive/specs-livrees/FLOW_SCHEME_EXPLICITE.md)) : compte sans `flow_scheme` **toléré mais exclu** silencieusement (vue `LEFT JOIN`, pas de validation bloquante). Le seed peuple `flow_scheme` sur tous les comptes (bilan/flux → BILAN, resultat → RESULTAT) → golden **stable** (pas de changement de comportement). **Flip B1 livré** : `dim_account.flow_scheme` + `sat_flow_scheme_item.scheme` en ids → `flow_scheme` = **5ᵉ dimension renommable**. |
| Q46 | **`pivot_currency` attribut de `rate_set`** : quitte `app_config` (singleton global). | `convert.rs` lit `app_config.pivot_currency` · bloque la renommabilité pleine de `currency` (étape 4) | TRANCHÉE (2026-06-25) | `dim_rate_set` gagne **`pivot_currency`** (→ `pivot_currency_id` après flip B1). `load_params` lit le pivot depuis le `rate_set` du run ; `app_config.pivot_currency` supprimé. Découple la renommabilité de `currency` du singleton. Comportement préservé si le `rate_set` porte `EUR` (cas du seed). Supplante le pivot global de `SPEC_SCENARIO_V2`. |

## Reportables (post-prototype)

| ID | Question | Contexte | Priorité | Statut | Décision |
|---|---|---|---|---|---|
| Q12 | **Performance** : temps cible entre données reçues et reporting disponible ? | EDB §6 | TÔT | TRANCHÉE + MESURÉE (2026-06-29) | **Obligation de moyens, pas de cible chiffrée.** Rust confirmé + **stockage à bien dimensionner** (guidance à venir). La performance est un objectif de conception qui oriente les choix techniques, notamment la base de données. **Mesurée au 2026-06-29 via `conso-bench`** (cf. [`ETAT_AVANCEMENT.md`](./ETAT_AVANCEMENT.md) § Performance) : ~48 k lignes stg/s sur gros volumes, 5 M lignes traitées en 103 s, identités de clôture tenues. Goulots identifiés : étapes C (convert) et D (consolidate) ~2,5× plus lentes que A (corporate). |
| Q13 | **Audit / traçabilité** : format de la référence d'audit (`Audit_id`) et chaîne de traçabilité des écritures auto ? | EDB §6 | POST | TRANCHÉE | **`Audit_id` abandonné (trop flou), remplacé par le champ `Source`** (décision 2026-06-20). `Source` est une **métadonnée non-dimensionnelle** de provenance (réf. de liasse, plus tard règle/import générateur), portée par `stg_entry`, hors registre des dimensions et hors grain de clôture. Voir [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 `Source*` / `Analysis2*` (ex-`Audit_id`). |
| Q14 | **Évolutivité** : critères concrets (ajouter entité / référentiel / module sans refonte) ? | EDB §6 | POST | TRANCHÉE | **Largement acquise par construction** (décision 2026-06-20) : registre data-driven des dimensions, dimensions custom via l'API, règles en JSON, flux pilotés par `dim_flow.flux_de_report` — le code est délibérément dynamique. Traitée comme une **guidance générale de conception à continuer de suivre**, pas comme un objectif à implémenter à ce stade. |
| Q24 | **Éditeur de règles de consolidation** : modèle de composition des écritures automatiques (déclencheur, conditions, contrepassation, imputation). Démarre par éliminations interco + participations. | EDB §3.4 | POST | TRANCHÉE | **Implémenté dans le prototype le 2026-06-19** (anticipation du post-MVP). Triple couche : moteur Rust (`prototype/rust/src/rules.rs`, `run_ruleset`), API REST (`prototype/rust/src/bin/server.rs`, 11 endpoints `/api/rules` + `/api/rulesets` + exécution), UI React (`web/src/pages/RulesPage.tsx` : éditeur visuel scope/opérations/destination + jeux de règles ordonnés + exécution + rapport). Spec dans [`REGLES_CONSO.md`](./REGLES_CONSO.md) (2026-06-18) ; questions R1–R7 toutes tranchées. Schéma BDD dédié : `dim_rule`, `dim_ruleset`, `dim_ruleset_item`. **Tests automatisés Rust** : 37 unitaires (module `rules::tests`, parsing + helpers SQL) + 13 d'intégration (`tests/rules.rs`, exécution sur DuckDB in-memory). Reste post-MVP : intérêts minoritaires, retraitements, variations de capital, répartition des résultats (catalogue [`REGLES_CONSO.md`](./REGLES_CONSO.md) §10). Évolutions fonctionnelles récentes tracées en [Q39]. |
| Q27 | **Mode de fusion** : privilégier F07 (à l'ouverture, `= −F00`) ou F70 (en cours d'exercice, `= −F99`), ou garder les deux ? | [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9 | POST | TRANCHÉE | **Trop avancé à ce stade, aucun besoin spécifique** (décision 2026-06-20). La fusion sera **probablement traitée via l'éditeur de règles de consolidation** ([Q24]), pas en natif dans le moteur. Pas d'implémentation native prévue ; spec F07/F70 conservée dans [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9 pour mémoire. |
| Q28 | **Reconstruction F99 selon `flux_de_report`** : actuellement F99 = Σ(tous les autres flux). Évolution prévue : utiliser `flux_de_report` de chaque flux pour déterminer dans quel flux il s'agrège (un flux peut reporter ailleurs que F99). | [`FLUX_CONSO.md`](./FLUX_CONSO.md) · `dim_flow.flux_de_report` | POST | TRANCHÉE | **Implémenté le 2026-06-16** dans le moteur Rust. Un flux auto-référentiel (`flux_de_report(C) = C`) est une **clôture reconstruite** comme `C = Σ(X | flux_de_report(X) = C et X ≠ C)`. Aujourd'hui seule F99 est auto-référentielle, mais la logique est générique (plus aucun littéral `'F99'` dans le moteur). (1) `materialize_closures` (ex-`materialize_f99`) reconstruit par `DELETE` ciblé + `INSERT` → **sémantique d'écrasement** : la valeur reconstruite est autoritaire au grain dimensionnel et n'écrase pas une clôture d'un autre grain. (2) `validate` est data-driven (lecture de `dim_flow`, plus de listes en dur). (3) Le **grain** de reconstruction est documenté dans `materialize_closures.rs` : `(scenario, entity, entry_period, period, account, currency)` — à chaque ajout de dimension (ex. `Nature`), se demander si elle entre dans le grain (sinon écrasement trop large). Prototype Python laissé en l'état (legacy divergent). Voir [`FLUX_CONSO.md`](./FLUX_CONSO.md) §3. |
| Q30 | **Articulation ET/OU du scope et de la sélection** : les conditions sont aujourd'hui combinées exclusivement par ET (conjonction codée en dur dans `rules.rs`). Faut-il supporter le OU (groupes imbriqués) pour éviter de multiplier les règles ? | [`REGLES_CONSO.md`](./REGLES_CONSO.md) §3 | POST | TRANCHÉE | **Reporté — le besoin est couvert à ce stade par le OU intra-condition via l'opérateur `IN`** (décision 2026-06-20). Les groupes ET/OU imbriqués restent une évolution possible plus tard, sans urgence. |

## Évolutions futures (post-prototype — à approfondir)

Ces 4 pistes ont été identifiées comme améliorations significatives. Chacune fait l'objet d'une première analyse (difficulté, conséquences, décisions fonctionnelles requises) pour préparer leur intégration éventuelle.

| ID | Sujet | Priorité | Difficulté | Statut |
|---|---|---|---|---|
| Q47 | **Dimensions dépendantes du temps** (time-dependent dimensions) | POST | Élevée | EN RÉFLEXION |
| Q48 | **Consolidation à la marge** (incrémentale, avec locking) | POST | Élevée | EN RÉFLEXION |
| Q49 | **Consolidation temps réel** (déclenchée à l'intégration de liasse) | POST | Moyenne–Élevée | EN RÉFLEXION |
| Q50 | **Calcul hiérarchique sur les dimensions** (avec ou sans stockage) | POST | Moyenne | EN RÉFLEXION |

### Q47 — Dimensions dépendantes du temps

**Besoin** : certains attributs de dimensions changent au cours du temps (ex. `devise_fonctionnelle` d'une entité, `compte_parent` dans le plan de comptes, `% d'intégration` dans le périmètre). Le modèle actuel porte ces valeurs comme attributs courants — pas de historisation natif.

**Difficulté** : **Élevée** — touche au modèle de données fondamental.
- Le modèle actuel (§3 `MODELE_DONNEES.md`) considère les attributs master data comme des **snapshots courants**. Ajouter la dimension temporelle implique de **versionner chaque attribut par période** (ou par couple `(début, fin)`).
- Deux approches : (a) **SCD Type 2** (colonnes `valid_from`/`valid_to` sur chaque satellite) — robuste mais explosion du nombre de lignes et complexité des JOINs ; (b) **snapshots par période** (chaque période porte une copie complète des attributs) — simple mais redondant.
- Impact sur **tous les consommateurs** d'attributs : `sat_perimeter` (déjà versionné par période, mais `%_intégration` est fixe), `v_flow_behavior` (schéma de flux par compte), `rules.rs` (coefficients lus sur le périmètre), `convert.rs` (devise fonctionnelle).

**Conséquences fonctionnelles** :
- **Pro forma** : si les attributs changent rétroactivement (ex. une devise fonctionnelle modifiée en cours d'exercice), les consolidations déjà réalisées deviennent invalides. Faut-il un mode **pro forma** qui simule la conso « comme si » l'attribut avait toujours eu la nouvelle valeur ?
- **À-nouveau** : le snapshot N-1 porte les attributs de l'époque. Si un attribut change entre N-1 et N (ex. entité change de devise fonctionnelle), la conversion de l'à-nouveau utilise le taux N-1 dans l'ancienne devise — cohérent mais nécessite une résolution explicite.
- **Grain de clôture** : si un attribut entre dans le grain (ex. `currency` via la devise fonctionnelle), un changement en cours d'exercice crée deux lignes de clôture distinctes → F99 se scinde.

**Décisions à trancher** :
1. Quels attributs ont besoin de versionnement temporel ? (Tous ? Seulement `devise_fonctionnelle`, `%_intégration`, `compte_parent` ?)
2. Approche SCD2 vs snapshots par période ?
3. Besoin d'un mode pro forma (rejouer la conso avec des attributs hypothétiques) ?
4. Que faire des consolidations déjà réalisées quand un attribut change rétroactivement ?

**Estimation** : chantier **structurant** — à planifier comme évolution majeure post-MVP, probablement après avoir stabilisé le workflow de validation (Q48/Q49).

---

### Q48 — Consolidation à la marge (incrémentale + locking)

**Besoin** : au lieu de recalculer toute la consolidation à chaque soumission (mode « complète » actuel, cf. Q6), ne recalculer que les **entités impactées** par la modification. S'associe à un **système de locking** des consolidations (une conso verrouillée ne peut pas être recalculée).

**Difficulté** : **Élevée** — remet en cause l'architecture du pipeline.
- Le pipeline actuel (`pipeline.rs`) est un **batch séquentiel** (agrégation → conversion → consolidation) qui purge et reconstruit `fact_entry` pour une `consolidation_id`. Passer au incrémental nécessite de **tracer les dépendances** : quelles écritures sources alimentent quelles écritures consolidées ?
- Le **grain d'invalidation** doit être identifié : une modification d'une liasse de l'entité X ne devrait recalculer que X + les entités qui dépendent de X (via interco, participations). Le graphe de dépendances est **transitif** (X → interco avec Y → Y dépend de X).
- Le **locking** implique un état sur `dim_consolidation.statut` (déjà existant : `brouillon`/`ouvert`/`verrouillé`) + une gestion de conflits (que se passe-t-il si on modifie une liasse qui alimente une conso verrouillée ?).

**Conséquences fonctionnelles** :
- **Granularité de recalcul** : entité seule ? écriture seule ? Le choix impacte la complexité et les performances.
- **Cohérence interco** : si l'entité A modifie une écriture interco avec B, la conso de B doit être recalculée aussi → propagation en cascade.
- **Règles** : les règles qui scope sur plusieurs entités (ex. élimination IC) sont-elles re-exécutées en entier ou seulement pour les entités modifiées ?
- **Locking** : verrouillage au niveau conso (globale) ou par entité (fine-granularité) ? Durée : jusqu'à la clôture suivante, ou déverrouillable ?

**Décisions à trancher** :
1. Grain d'invalidation : entité ? écriture ? sous-ensemble de comptes ?
2. Propagation interco : automatique ou avertir l'utilisateur ?
3. Locking : conso globale ou par entité ? Verrouillage manuel ou automatique à la validation ?
4. Que faire des dépendances transitives profondes (A→B→C→…) ?

**Estimation** : **chantier majeur** — nécessite probablement un graphe de dépendances matérialisé + un moteur de propagation. À ne pas démarrer avant que le pipeline batch soit parfaitement stable.

---

### Q49 — Consolidation temps réel déclenchée à l'intégration de liasse

**Besoin** : distinguer deux étapes du workflow — la **saisie/chargement** (导入 de données brutes, état `brouillon`) et l'**intégration** (validation qualité + passage au statut `soumis`, qui déclenche automatiquement la consolidation). La consolidation temps réel se déclenche à chaque intégration, pas à chaque saisie.

**Difficulté** : **Moyenne–Élevée** — principalement un sujet de workflow, mais avec des implications moteur.
- Le modèle actuel (Q8 tranchée) n'a **aucun workflow** : toute liasse est immédiatement intégrée. Il faut donc implémenter un cycle de vie des écritures (`brouillon` → `soumis` → `verrouillé`).
- La **différenciation saisie/intégration** nécessite deux endpoints ou deux phases : `POST /api/entries` (saisie, reste en `brouillon`) vs `POST /api/entries/integrate` (passe à `soumis` + déclenche la conso).
- Le **déclenchement automatique** de la conso à l'intégration implique un mécanisme d'événement (hook après intégration) — simple en architecture monolithe (appel direct du pipeline après le changement de statut).

**Conséquences fonctionnelles** :
- **Contrôles qualité pré-intégration** : quels checks avant de permettre l'intégration ? (cohérence référentielle, complétude, balances de contrôle ?) — à définir.
- **Granularité de déclenchement** : intégration d'une seule entité → recalcule-t-on toute la conso ou seulement cette entité ? (lien direct avec Q48 — les deux évolutions sont **complémentaires** : Q49 déclenche, Q48 détermine quoi recalculer.)
- **Multi-consolidation** : si la remontée intégrée alimente N consolidations, les N sont-elles recalculées en cascade ? Avec quel ordre de priorité ?
- **Statuts** : le modèle `brouillon/soumis/verrouillé` existe déjà sur `dim_consolidation.statut` — il faut l'étendre aux **écritures** (`stg_entry.statut`).

**Décisions à trancher** :
1. Contrôles qualité pré-intégration : lesquels ? Obligatoires ou optionnels ?
2. Granularité de déclenchement : conso complète ou à la marge (lié à Q48) ?
3. Multi-consolidation : recalcul séquentiel ou parallèle ? Priorité ?
4. Timeout / annulation : que se passe-t-il si le recalcul échoue en cours ?

**Estimation** : **évolution modérée** si elle reste limitée au workflow (ajout de statuts + endpoint d'intégration + hook de déclenchement). Elle devient **élevée** couplée avec Q48 (consolidation à la marge).

---

### Q50 — Calcul hiérarchique sur les dimensions (avec ou sans stockage)

**Besoin** : certaines dimensions ont une **hiérarchie naturelle** (ex. `compte_parent` sur `Account`, `entite_parent` sur `Entity`). Les restitutions (bilan, P&L) doivent afficher des **sous-totaux hiérarchiques** (ex. total d'une classe de comptes). Question : calcule-t-on ces agrégats **à la volée** (requête récursive) ou les **stocke-t-on** (table d'agrégat matérialisé) ?

**Difficulté** : **Moyenne** — DuckDB supporte les CTE récursives, mais le choix a des conséquences sur la volumétrie et la cohérence.

**Option A — Calcul à la volée (pas de stockage)** :
- Requête CTE récursive sur `dim_account.compte_parent` pour reconstruire l'arbre, puis `GROUP BY` sur le nœud parent.
- **Avantages** : zéro espace de stockage supplémentaire, cohérence garantie (les données source sont toujours à jour), pas de maintenance.
- **Inconvénients** : performance sur de grands arbres (des milliers de comptes × profondeur 5-6) ; la requête est ré-exécutée à chaque restitution ; complexité SQL pour les pivots multi-niveaux.

**Option B — Stockage matérialisé** :
- Table `hier_account (ancestor, descendant, depth)` pré-calculée par un script de maintenance, consommée par les restitutions via un simple JOIN.
- **Avantages** : requêtes de restitution rapides (JOIN direct, pas de récursif) ; réutilisable par les règles (ex. « tous les comptes descendants de 60 »).
- **Inconvénients** : nécessite un **recalcul** à chaque modification de la hiérarchie ; risque d'incohérence si le recalcul est oublié ; espace de stockage (O(n × profondeur)).

**Option C — Hybride** :
- Calcul à la volée pour les restitutions (pas de stockage), mais **cache en mémoire** côté serveur (TTL ou invalidation à la modification de la hiérarchie). Les règles utilisent le cache.
- **Avantages** : performance sans stockage persistant, cohérence par invalidation.
- **Inconvénients** : complexité du cache (invalidation, multi-instance si un jour distribué) ; perte du cache au redémarrage.

**Conséquences fonctionnelles** :
- **Impact sur les règles** : les règles de consolidation utilisent déjà les hiérarchies via `map_ref` (sélection par `compte_parent`). Si la hiérarchie est stockée, les règles peuvent directement JOIN la table d'agrégat — plus simple et plus rapide.
- **Profondeur variable** : les hiérarchies de comptes ont typiquement 4-6 niveaux ; les hiérarchies d'entités 2-3 niveaux. Le calcul récursive est plus coûteux sur les arbres profonds.
- **Multi-dimensions** : le même mécanisme s'applique à `Entity` (structure de groupe), `Analysis` (hiérarchie de centres de coût), etc. → un **framework générique** serait préférable à du cas par cas.
- **Règles vs restitutions** : les deux usages ont des besoins différents (règles = filtre « tous les descendants », restitution = affichage arborescent avec sous-totaux).

**Décisions à trancher** :
1. Approche : volée (A), stockée (B), hybride (C) ?
2. Quelles dimensions ont une hiérarchie à gérer ? (Account, Entity, Analysis ?)
3. Les sous-totaux hiérarchiques sont-ils dans le grain de clôture (stockés dans `fact_entry`) ou calculés uniquement à la restitution ?
4. Framework générique ou cas par cas par dimension ?

**Estimation** : **évolution modérée** si limitée au calcul à la volée pour les restitutions. L'option B (stockage) est plus propre mais nécessite un mécanisme de maintenance. L'option C (hybride) est le meilleur compromis mais ajoute de la complexité serveur.

---

### Q51 — Formulaires configurables (cahiers de saisie & restitution)

**Besoin** : remplacer les pages fixes (Saisie, Écritures, Rapports) par un système de **formulaires configurables** organisés en **cahiers** (workbooks). Chaque formulaire est soit de la **saisie** (écriture/modification/suppression au niveau raw), soit de la **restitution** (lecture seule au niveau demandé). Les formulaires de restitution fonctionnent comme des **tableaux croisés dynamiques** basés sur les indicateurs existants.

**Difficulté** : **Élevée** — couche d'abstraction significative au-dessus des indicateurs existants.

**Ce qui existe déjà et se réutilise** :
- `SaisiePage` : grille inline CRUD au niveau raw (`stg_entry`, source=MANUAL).
- `RapportsPage` : bilan/compte de résultat avec pivot account×flow (read-only).
- `IndicatorsPage` : postes (sélections agrégées) + indicateurs (formules) avec grain.
- Le moteur de formules (`formula.rs`) et la compilation SQL au grain.

**Ce qui est nouveau** :

1. **Cahier** (`dim_workbook`) : un cahier = un ensemble de feuilles. Chaque feuille est un formulaire. Le cahier porte un nom, un type (`saisie` ou `restitution`), et éventuellement une consolidation cible.

2. **Feuille / formulaire** (`dim_sheet`) : une feuille = un pivot table configuré avec 3 axes :
   - **Axe document** (multi-feuille) : dimension qui scinde en onglets (ex. une feuille par entité, une feuille par nature).
   - **Axe lignes** : dimension en lignes (ex. comptes, partenaires).
   - **Axe colonnes** : dimension en colonnes (ex. flux, périodes).
   - Pour la **restitution** : les cellules sont des indicateurs (formules) ou des postes (agrégats).
   - Pour la **saisie** : les cellules sont des montants éditables liés à `stg_entry`.

3. **Mode saisie** : la grille affiche les montants existants (niveau raw) pour le grain (entity × account × flow × …). L'utilisateur peut :
   - **Modifier** un montant existant (PUT).
   - **Ajouter** une ligne absente (POST).
   - **Supprimer** une ligne (DELETE).
   - Protections identiques à `SaisiePage` : source=MANUAL uniquement.

4. **Mode restitution** : la grille affiche les valeurs calculées (indicateurs) au grain configuré. Read-only. Le pivot est compilé en SQL par le moteur d'indicateurs existant.

**Architecture proposée** :

```
dim_workbook (code, libelle, type: saisie|restitution)
  └── dim_sheet (code, libelle, workbook, ordre,
        axe_document_dim, axe_document_value?,   -- ex. entity = "M"
        axe_lignes_dim, axe_lignes_hier?,         -- ex. account via compte_parent
        axe_colonnes_dim,                         -- ex. flow
        source: aggregate|indicator|entry,        -- ce qui remplit les cellules
        source_code?,                             -- code de l'indicateur ou du poste
        level?)                                   -- pour les entrées : raw/corporate/...
```

**Compilation en SQL** (restitution) : pour un indicateur `I` au grain `(axe_lignes, axe_colonnes)` :
```sql
SELECT <axe_lignes>, <axe_colonnes>, <formule_I compilée>
FROM fact_entry
WHERE consolidation_id = ? AND level = ?
GROUP BY <axe_lignes>, <axe_colonnes>
```
→ Exactement le même mécanisme que `indicators.rs::compile_indicator`, avec un grain à 2 dimensions au lieu de 1.

**Compilation en SQL** (saisie) : requête de lecture des montants existants au grain de la feuille :
```sql
SELECT <axe_lignes>, <axe_colonnes>, SUM(amount)
FROM stg_entry
WHERE phase = ? AND entry_period = ? AND <axe_document = valeur>
GROUP BY <axe_lignes>, <axe_colonnes>
```
→ La grille affiche le résultat, l'utilisateur édite, les modifications sont des PUT/POST/DELETE unitaires.

**Conséquences fonctionnelles** :
- **Héritage des indicateurs** : un formulaire de restitution est un indicateur vu sous l'angle pivot. Le moteur d'indicateurs existant fait le gros du travail ; la couche cahier ajoute l'organisation (multi-feuille) et le pivot (2 axes au lieu d'un grain 1D).
- **Saisie = inverse de la restitution** : la même grille, mais en mode écriture. La structure (axes, dimensions) est identique ; seul le mode (read-only vs CRUD) diffère.
- **Filtres globaux** : consolidation, level, period — partagés entre toutes les feuilles d'un cahier.
- **Sous-totaux** : si l'axe lignes est hiérarchique (ex. compte_parent), les sous-totaux sont calculés à la volée (lié à Q50).
- **Formatage** : devise, %, nombre — porté par l'indicateur ou configuré sur la feuille.

**Décisions à trancher** :
1. Les formulaires de saisie éditent-ils `stg_entry` directement (grain raw) ou faut-il un buffer intermédiaire ?
2. Les axes sont-ils limités aux dimensions existantes ou supportent-ils les caractéristiques (N1/N2) ?
3. Un cahier de saisie peut-il mélanger des feuilles de saisie et des feuilles de restitution (ex. une feuille de saisie + une feuille de contrôle) ?
4. Les sous-totaux hiérarchiques sont-ils affichés dans le pivot (lié à Q50) ?
5. Comment gérer les cellules vides (absence de données) : afficher 0, laisser vide, ou autoriser la saisie ?

**Estimation** : **chantier majeur** — la couche cahier/feuille est un nouveau niveau d'abstraction UI + données. La restitution est plus simple (réutilise les indicateurs) ; la saisie nécessite un mode CRUD sur la grille pivot. Phasage recommandé : restitution d'abord (lecture seule), saisie ensuite.

---

### Q52 — Gestion des droits d'accès (rôles & permissions)

**Besoin** : contrôler qui peut voir et faire quoi dans l'application. Trois niveaux de rôles typiques :
- **Contributeur de liasse** : saisie/modification de ses propres écritures (raw), consultation de ses données. Pas d'accès à la consolidation ni aux règles.
- **Contrôleur de consolidation** : consultation de tous les niveaux, exécution de la consolidation, gestion des règles et des indicateurs. Pas de modification des données master data ni des paramètres système.
- **Administrateur** : accès total (master data, paramètres, utilisateurs, consolidation, import/export).

Possibilité de **restreindre l'accès à certains packages** (c'est-à-dire des ensembles de données ou de fonctionnalités) pour certains utilisateurs ou groupes. Ex. : le contributeur de la filiale US ne voit que les écritures de la filiale US.

**Difficulté** : **Élevée** — touche à l'ensemble de l'application (API, UI, modèle de données).

**État actuel** : la sécurité est explicitement **ignorée** (Q15, EDB §6). L'application est locale, mono-utilisateur, sans authentification. Tout est accessible à tout le monde.

**Ce qu'il faut construire** :

1. **Authentification** : identifier l'utilisateur. Options :
   - **Locale** (login/mot de passe stocké en base) — simple, adapté au mode local/mono-utilisateur.
   - **SSO/LDAP/Azure AD** — nécessaire si l'application est partagée dans un réseau d'entreprise.
   - **JWT token** — stateless, adapté à une API REST.

2. **Autorisation** : définir les permissions. Modèle proposé :
   - **Rôle** : `contributeur`, `contrôleur`, `admin` (enum fixe ou configurable).
   - **Permission** : couple `(ressource, action)` ex. `(entries, read)`, `(entries, write)`, `(consolidation, execute)`, `(masterdata, write)`.
   - **Rôle → permissions** : mapping configurable (ex. `contributeur` → `entries:read/write` sur ses entités uniquement).
   - **Scope** : restriction par dimension (ex. `entity IN ('US', 'UK')` pour un contributeur filiale). Modélisable comme un filtre additionnel injecté dans les requêtes.

3. **Groupes** : un utilisateur appartient à un ou plusieurs groupes. Les permissions s'héritent (union). Ex. : un contributeur du groupe « Europe » accède aux entités EU.

4. **Packages** : un package = un ensemble de ressources (entités, comptes, règles…). L'accès est restreint par package. Ex. : le package « Consolidation France » regroupe les entités FR + les règles associées.

**Impact architectural** :
- **API** : chaque endpoint doit vérifier les permissions (middleware Axum). Le scope (filtre par entité/partenaire) est injecté automatiquement dans les requêtes SQL.
- **UI** : masquer/afficher les menus et actions selon le rôle. Navigation conditionnelle.
- **Modèle** : tables `dim_user`, `dim_group`, `dim_role`, `dim_permission`, `dim_package` + tables de jonction.
- **DuckDB** : pas de mécanisme d'authentification natif. L'auth se fait côté serveur Rust.

**Décisions à trancher** :
1. Authentification : locale (login/mdp) ou SSO ? Ou les deux (locale par défaut, SSO en option) ?
2. Rôles fixes (3 niveaux) ou configurables par l'admin ?
3. Scope par dimension (filtre sur entity, account…) ou par package (groupe de ressources) ?
4. Les permissions s'appliquent-elles aussi aux données d'export (un contributeur peut-il exporter tout le périmètre) ?
5. Multi-utilisateur local (plusieurs comptes sur la même machine) ou uniquement réseau (plusieurs postes) ?

**Estimation** : **chantier très structurant** — chaque endpoint et chaque page sont concernés. À ne pas démarrer avant que le périmètre fonctionnel soit stable. Phasage recommandé : (1) authentification + rôles de base (admin/contributeur/contrôleur), (2) scope par entité, (3) packages.

---

### Q53 — Environnements multiples (Développement / Intégration / Production)

**Besoin** : disposer de 3 bases distinctes (Développement, Intégration, Production) avec un mécanisme de **promotion** du paramétrage et des données entre les environnements. Typiquement : on paramètre en Dev, on teste en Intégration, on déploie en Production.

**Difficulté** : **Moyenne–Élevée** — principalement un sujet d'infrastructure, mais avec des implications sur le modèle de données.

**État actuel** : une seule base DuckDB locale (`consolidation.duckdb`), un seul environnement. L'export/import JSON (`/api/export` + `/api/import/all`) existe déjà et permet un transfert complet de l'état.

**Ce qu'il faut construire** :

1. **Isolation des bases** : 3 fichiers DuckDB distincts (`dev.duckdb`, `integ.duckdb`, `prod.duckdb`). Le serveur cible la base via une variable d'environnement (`CONSO_DB_PATH`).

2. **Promotion du paramétrage** : transférer les **définitions** (master data, règles, indicateurs, cahiers) d'un environnement à l'autre, **sans les données de saisie**. L'export JSON actuel (`/api/export`) exporte tout — il faudrait un mode **paramétrage seul** (exclure `stg_entry` et `fact_entry`).

3. **Promotion des données** : transférer les **données de saisie** (liasses) d'un environnement à l'autre. Utile pour alimenter Intégration avec les données de Prod (jeu de test réaliste) ou pour initialiser un nouvel environnement.

4. **Versioning du paramétrage** : savoir quel paramétrage est en Prod vs Dev. Options :
   - **Numéro de version** incrémenté à chaque promotion.
   - **Tag Git** : le paramétrage est versionné dans le repo (fichiers JSON exportés), la promotion = un commit + un déploiement.
   - **Audit trail** : qui a promu quoi, quand.

5. **Migrations de schéma** : quand le schéma évolue (nouvelle table, colonne), les 3 bases doivent être migrées. DuckDB n'a pas de mécanisme de migration natif — il faut un script applicatif (ex. numéro de version de schéma + scripts SQL séquentiels).

**Architecture proposée** :

```
┌─────────────┐     export paramétrage      ┌─────────────┐
│  Dev         │ ──────────────────────────▶ │  Intégration │
│  dev.duckdb  │     import JSON              │  integ.duckdb│
└─────────────┘                              └──────┬──────┘
                                                    │
                                          export paramétrage
                                                    ▼
                                             ┌─────────────┐
                                             │  Production  │
                                             │  prod.duckdb │
                                             └─────────────┘
```

**Décisions à trancher** :
1. Le paramétrage est-il versionné dans Git (fichiers JSON) ou géré par un mécanisme applicatif (numéro de version en base) ?
2. La promotion est-elle manuelle (export/import via l'UI) ou automatisée (script CI/CD) ?
3. Faut-il un mécanisme de **diff** (comparer le paramétrage Dev vs Prod avant promotion) ?
4. Les données de saisie sont-elles promues (Dev → Integ → Prod) ou chaque environnement a ses propres données ?
5. Comment gérer les **conflits** (un objet modifié à la fois en Dev et en Prod) ?

**Estimation** : **évolution modérée** si limitée à l'isolation des bases + export/import sélectif (paramétrage vs données). Le versioning dans Git est le levier le plus simple. La gestion des conflits et le diff sont des extensions plus complexes.

---

### Q54 — Accessibilité API pour agents IA (MCP & opérations en masse)

**Statut** : **TRANCHÉE** (2026-06-29) — implémenté. Spéc de réalisation dans
[`archive/specs-livrees/PLAN_Q54_API_MCP.md`](./archive/specs-livrees/PLAN_Q54_API_MCP.md) (livré), guide d'usage dans
[`MCP.md`](./MCP.md).

**Décision** :
1. **REST** — 6 améliorations livrées : pagination (`?limit&offset`, enveloppe
   `{total,rows}` opt-in), recherche (`?search=` ILIKE sur `libelle`), filtres
   dynamiques (`?{col}=valeur` validés), bulk upsert (`PUT /api/md/{table}/bulk`),
   bulk delete (`DELETE /api/md/{table}/bulk`), enrichissement (`?enrich=true`).
   Rétrocompat préservée (array plat par défaut).
2. **MCP** — serveur **intégré au binaire**, 10 outils curatés, transport `rmcp`.
   **Deux modes** : stdio (`conso-server --mcp`, process séparé) **et** HTTP
   (route `/mcp` sur le serveur Axum existant → UI + agent simultanés sur la
   même base DuckDB, sans verrou). Le mode HTTP lève la contrainte
   mono-processus du mode stdio.
3. **SDK** : `rmcp` (officiel Rust).
4. **Surface** : sous-ensemble curaté (saisie, run conso, contrôles, rapports
   bilan/P&L, indicateurs, lecture/écriture master data, `describe_model`).
5. **Auth** : aucune (local, prototype).
6. **Contrainte** DuckDB mono-processus : UI (`conso-server`) XOR agent
   (`conso-server --mcp`) sur le même `.duckdb`.

Cœur métier extrait dans `conso_engine::reports` (partagé HTTP ↔ MCP).

---

**Besoin** : rendre l'application facilement pilotable par des agents IA. Deux axes : (1) améliorer l'API REST existante pour les cas d'usage agent (bulk, recherche, pagination), et (2) envisager un **serveur MCP** (Model Context Protocol) qui encapsule l'API en outils nommés et typés pour les LLM.

**Difficulté** : **Moyenne** — l'API REST existe déjà et est fonctionnelle. Les améliorations sont incrémentales.

**État actuel** :
- API REST complète (Axum) : CRUD master data (`/api/md/{table}`), entrées (`/api/entries`), consolidation (`/api/run`), règles, indicateurs, coefficients, contrôles, export/import.
- **Entrées** : supportent déjà le **batch** (`POST /api/entries` avec un array de lignes).
- **Master data** : CRUD **unitaire** uniquement (`POST/PUT/DELETE` un objet à la fois). Pour modifier 50 comptes → 50 PUT. C'est le principal goulot pour un agent.
- **Pas de pagination** : les listes (`GET /api/md/{table}`, `GET /api/entries`) renvoient tout.
- **Pas de recherche/filtre** : pas de `?search=` ou `?filter=` sur les listes master data.
- **Pas de MCP** : aucune couche d'abstraction pour les LLM.

**Ce qui est déjà bien** :
- API JSON propre, stateless, RESTful.
- Batch sur les entrées.
- Export/import JSON complet (`/api/export`, `/api/import/all`).
- Pipeline déclenchable par API (`POST /api/run`).
- Schéma auto-documenté (`/api/md/{table}/schema`, `/api/meta/references`).

**Améliorations API REST (indépendantes du MCP)** :

| Amélioration | Impact | Difficulté |
|---|---|---|
| **Bulk master data** : `PUT /api/md/{table}/bulk` (array d'objets) | Critique pour les agents | Faible — même logique que `create_entries`, boucle en transaction |
| **Bulk delete** : `DELETE /api/md/{table}/bulk` (array de PKs) | Idem | Faible |
| **Pagination** : `?limit=N&offset=N` sur les listes | Important pour les gros volumes | Faible — clause SQL `LIMIT/OFFSET` |
| **Recherche** : `?search=texte` sur les listes master data | Utile pour trouver un compte par nom | Moyenne — `ILIKE` sur les colonnes libellé |
| **Filtres** : `?field=value` sur les listes (ex. `?classe=bilan`) | Ergonomie agent | Faible — `WHERE` dynamique |
| **Réponses enrichies** : inclure le `libellé` dans les FK des réponses (pas seulement le `code`) | Lisibilité agent | Moyenne — JOINs dans les SELECT |

**MCP (Model Context Protocol)** :

Un serveur MCP expose des **outils nommés** (avec description, paramètres JSON Schema, retour structuré) qu'un agent LLM peut appeler. Avantages par rapport à l'API REST brute :

| Aspect | API REST brute | Serveur MCP |
|---|---|---|
| Découverte | L'agent doit connaître les endpoints | Les outils sont décrits (nom, params, description) |
| Composition | L'agent fait N appels pour N opérations | Un outil peut combiner plusieurs étapes |
| Validation | Erreurs HTTP brutes | Messages adaptés au contexte agent |
| Sécurité | L'agent a accès à tout (ou rien) | Permissions par outil |

**Outils MCP proposés** :

| Outil MCP | Équivalent API | Gain agent |
|---|---|---|
| `list_accounts(filter?)` | `GET /api/md/dim_account` | + pagination, recherche, filtre |
| `create_accounts(accounts[])` | `PUT /api/md/dim_account` × N | **Bulk en 1 appel** |
| `import_entries(csv_or_json)` | `POST /api/entries` (batch) | Même chose, mais décrit pour l'agent |
| `run_consolidation(consolidation_id)` | `POST /api/run` | Identique |
| `get_bilan(consolidation, filters?)` | `GET /api/bilan` | + filtres nommés |
| `get_indicator_value(code, grain?)` | `GET /api/indicators/{code}/preview` | Grain paramétrable |
| `search_entities(query)` | Nouveau | Recherche full-text |
| `describe_schema()` | `/api/meta/references` + `/api/md/{table}/schema` | Un seul appel pour comprendre le modèle |

**Architecture** :
```
Agent LLM (Cursor, Claude, etc.)
    │ MCP (stdio ou HTTP)
    ▼
┌──────────────────────┐
│  conso-mcp (Rust)    │  ← nouveau binaire, ou plugin du conso-server
│  outils typés        │
└──────────┬───────────┘
           │ appels internes (mêmes fonctions Rust)
           ▼
┌──────────────────────┐
│  conso-engine (lib)  │  ← la logique existante
└──────────────────────┘
```

Le serveur MCP peut être un **binaire séparé** (`conso-mcp`) qui appelle les mêmes fonctions Rust que le serveur HTTP, ou un **mode** du serveur existant (`conso-server --mcp`). Le MCP communique en **stdio** (pour les agents locaux comme Cursor) ou en **HTTP/SSE** (pour les agents distants).

**Décisions à trancher** :
1. MCP : binaire séparé (`conso-mcp`) ou mode du serveur existant (`--mcp`) ?
2. Bulk master data : priorité avant le MCP ou en même temps ?
3. Pagination : limit/offset simple ou curseur (plus robuste pour les gros volumes) ?
4. Le MCP doit-il exposer **tous** les endpoints ou seulement un sous-ensemble (outils « sûrs » pour l'agent) ?
5. Authentification du MCP : clé API, token, ou pas d'auth (local) ?

**Estimation** : **évolution modérée** — le bulk master data est le quick win (quelques heures). Le MCP est un wrapper autour de l'existante (quelques jours). La pagination et la recherche sont des ajouts incrémentaux.

---

## Hors périmètre prototype (à ne pas traiter maintenant)

| ID | Sujet | Rappel |
|---|---|---|
| Q15 | **Sécurité** | EDB §6 : « Ignoré initialement ». À revoir après POC. |
| Q16 | **Licence** | EDB §7 : privé pour l'instant. |
| Q17 | **Mapping de comptes** | EDB §3.3 : saisie directe dans le plan groupe, mapping en option d'évolution. |
| Q18 | **Formats d'échange autres que CSV** | EDB §4 : évolutif. |
| Q19 | **Exports hors web** | EDB §5 : extension future. |

---

## Comment utiliser ce fichier

1. Répondre d'abord aux **BLOC** (Q1–Q5) : sans elles, le POC ne peut pas démarrer.
2. À chaque réponse : passer le **Statut** à `TRANCHÉE`, remplir **Décision** (idéalement datée et justifiée), puis reporter la décision dans `EXPRESSION_DE_BESOIN.md`.
3. Ne pas supprimer les lignes **TRANCHÉES** : elles servent d'historique des arbitrages.
