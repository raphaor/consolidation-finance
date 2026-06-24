# État d'avancement

> Vue consolidée de **ce qui est implémenté**, de son **comportement**, et de **ce qui reste**.
> Pour le *pourquoi* d'une décision → [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) ;
> pour le détail fonctionnel → les docs thématiques liées ci-dessous.
> Dernière mise à jour : **2026-06-24**.

**Légende** : ✅ implémenté & testé · 🟡 partiel / en cours · ⬜ reporté (post-MVP).

---

## Pipeline (moteur) — `prototype/rust/src/pipeline/`

✅ **3 niveaux de stockage** : `corporate` → `converted` → `consolidated` (le niveau
`reclassified` a été supprimé — le périmètre passe par des règles, cf. à-nouveau).
Tout est du **SQL ensembliste DuckDB** (une passe par règle métier), pas de calcul
ligne à ligne. Orchestration : `pipeline/mod.rs`.

| Étape | Comportement |
|---|---|
| **A. Agrégation** | Cumul des liasses par grain complet, en devise fonctionnelle. Filtré sur la **phase** + l'exercice de la consolidation + les entités du périmètre, isolé par `consolidation_id`. |
| **C. Conversion** | Multi-devises via **cross-rate** (devise pivot applicative), écarts de change F80/F81. |
| **D. Consolidation** | `× pct_integration` selon la méthode de l'entité. |
| **Clôtures** | F99 **reconstruite par identité** à chaque niveau (`materialize_closures`), pilotée par les données (`flux_de_report`), jamais en dur. |

⬜ Mode « à la marge » (aujourd'hui : recalcul total). ⬜ Staging multi-points par préfixe de
nature (couplé aux règles).

## Dimensions (data-driven) — `dimensions.rs`, `references.rs`

✅ **Registre central** : 12 dimensions built-in + dimensions **custom** (toujours Analytical).
Trois catégories (`Fixed` / `Active` / `Analytical`) dérivent propagation, nullabilité et grain
de clôture.
✅ **Sémantique « of which »** : une ligne dont une dimension analytique est renseignée est un
*dont* de la ligne où elle est NULL — exclue des totaux, mais avec sa propre clôture.
✅ **Caractéristiques N1/N2** et **références directes** (patron B, ex. `compte_parent`) :
définition + UI + consommation par les règles — **quatre points de consommation** :
destination `map` (caractéristique), destination `map_ref` (référence directe), sélection
`via` (filtre par valeur N1), sélection `ref` (filtre par référence directe).
✅ **Graphe de références** : validation à l'écriture (master data, import CSV, règles) +
endpoint « santé des données ».

→ Détail : [`MODELE_DONNEES.md`](./MODELE_DONNEES.md).

## Flux & schémas de flux — [Q32]

✅ `dim_flow` est une **dimension nue** (`code`, `libellé`). **Tout le comportement** d'un flux
(taux de conversion, flux d'écart, flux de report de clôture, flux d'à-nouveau) vit dans les
**schémas de flux** (`dim_flow_scheme` / `sat_flow_scheme_item`), articulation **complète** par
schéma, résolue **par compte** via la vue `v_flow_behavior`.

| Schéma | Comportement |
|---|---|
| `BILAN` (défaut) | Taux du flux (clôture N-1 / moyen / clôture N) **avec** écarts F80/F81 ; report F99 → F00 à l'à-nouveau. |
| `RESULTAT` | Tout au **taux moyen**, **sans écart**, **sans à-nouveau** (un P&L ne s'ouvre pas en N+1). |

Le compte choisit son schéma via `dim_account.flow_scheme` (NULL = défaut dérivé de la classe :
`resultat` → `RESULTAT`, sinon `BILAN`). **Invariant** : un schéma doit être complet.

→ Détail : [`FLUX_CONSO.md`](./FLUX_CONSO.md) §1–2 bis.

## Méthodes de consolidation — [Q33]

✅ `dim_method` **pilotable** (CRUD, flag `consolidated`) — plus de liste en dur. **Globale** et
**proportionnelle** (`× pct_integration`) natives. Méthode `MERE` pour **cibler la mère** via le
scope d'une règle (`methode = 'MERE'`).
⬜ **Mise en équivalence** (post-MVP : démarrera comme une proportionnelle, spécificités par
règles). ⬜ **Intérêts minoritaires** (par règles).

## Variations de périmètre

🟡 Entrée (F00 → F01) et sortie (miroir −F98) : **repensées en règles** depuis la suppression du
niveau `reclassified`. Les tests natifs correspondants sont `#[ignore]` en attendant les règles
de périmètre. → [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9, [`A_NOUVEAU.md`](./A_NOUVEAU.md).

## Périmètre versionné — [Q35]

✅ `dim_perimeter_set` + `sat_perimeter` clé par `(perimeter_set, entity, period)` +
`dim_consolidation.perimeter_set`. Un même périmètre est **réutilisable** entre consolidations/variantes
(symétrique des jeux de taux). Résolution `consolidation → perimeter_set` en SQL, période lue à
`perimeter_period` (défaut = exercice).

## Taux de change versionnés — [Q34]

✅ `dim_rate_set` + `sat_exchange_rate` clé par `(rate_set, currency_source, period)` +
`dim_consolidation.rate_set`, période lue à `rate_period` (défaut = exercice). CRUD + import CSV.
Taux clôture & moyen, conversion vers une **devise pivot** applicative. **Taux d'ouverture**
(`sat_exchange_rate.taux_ouverture` = clôture N-1, **portée par N**) consommé par la branche
F00/F01 de la conversion — plus de dépendance à une période N-1 (`prev_period` supprimé).

## Consolidation (v3) — `dim_consolidation` [Q41]

✅ Objet composite, **redesign identité** (`dim_scenario`→`dim_consolidation`) : PK technique `id`
auto + **clé naturelle UNIQUE** `(phase, exercice, perimeter_set, variant, presentation_currency)`.
`code` disparaît ; `category`→`phase`, `entry_period`→`exercice`, `a_nouveau_scenario`→
`a_nouveau_consolidation_id`. **Périodes explicites** `perimeter_period` + `rate_period` (défaut =
exercice), ruleset, `rate_set`, `perimeter_set`. Les saisies (`stg_entry`) sont au grain
**phase + entry_period**, donc **partagées** entre consolidations ; chaque run est isolé par
`fact_entry.consolidation_id`. → spec d'origine archivée `SPEC_SCENARIO_V2.md` (supersédée par Q41).

## À-nouveau (report d'ouverture) — [Q31]

✅ Report de la clôture **N-1 figée** (snapshot) sur l'ouverture **N**, piloté par les données
(`flux_a_nouveau` du schéma + `dim_consolidation.a_nouveau_consolidation_id`). Collé au **corporate** et au
**consolidé** (fige le % N-1) ; le converti se déduit par conversion normale. **Garde par
compte** : seul le bilan reporte (le résultat non). Contrôle de cohérence dans `validate`.
→ Spec : [`A_NOUVEAU.md`](./A_NOUVEAU.md) / [`A_NOUVEAU_IMPL.md`](./A_NOUVEAU_IMPL.md).

## Éditeur de règles de consolidation — [Q24]

✅ **Exécuteur générique** (`rules.rs`) : `scope` (conditions sur `sat_perimeter`) + `operations`
(sélection à un niveau × coefficient × multiplicateur → écriture avec `destination` par
dimension : `inherit` / `override` / `null` / `map` / `map_ref`). Rulesets ordonnés. API REST +
UI React. Sécurité SQL (identifiants validés contre des whitelists, valeurs paramétrées).
✅ **Sélection étendue** : filtres indirects par **attribut traversé** — `via` (caractéristique
N1, ex. `comportement = VENTES_IC`) ou `ref` (référence directe patron B, ex. `compte_parent = 60`),
en plus du filtre direct sur la dimension. INNER JOIN : un membre non classé / sans valeur de
référence n'est pas sélectionné.
✅ **UI riche** (`web/src/pages/RulesPage.tsx`) : dropdown « Traverser » dans la sélection ;
multi-select repliable pour l'opérateur `IN` (tous cas : direct, via N1, ref) avec cases à cocher
et fermeture au clic extérieur ; dropdowns adaptatifs pour les 5 modes de destination.
⬜ Catalogue métier à composer : interco avancées, intérêts minoritaires, retraitements,
variations de capital, répartition des résultats. → [`REGLES_CONSO.md`](./REGLES_CONSO.md) §10.

## Restitutions

✅ Table consolidée **filtrable**, **bilan par flux**, **compte de résultat** — filtrables par
nature, avec **détail par nature** dépliable. Les totaux excluent les « of which ».
⬜ Bilan mis en forme, tableau de flux de trésorerie, annexe, dashboards.

## Master data & échanges

✅ **CRUD générique** pour chaque dimension et table satellite (`/api/md/{table}`). Import CSV
(liasses, taux, périmètre). **Export / import** d'un paquet JSON complet (sauvegarde-restauration).
🟡 Édition encore « à plat » (ligne par ligne) pour les satellites versionnés ; un écran « objet »
(ouvrir un jeu, y insérer ses lignes) reste souhaitable.

## Saisie manuelle d'écritures — [Q36]

✅ **Vue dédiée « Saisie »** (`web/src/pages/SaisiePage.tsx`, onglet nav) — alternative à l'import
CSV pour saisir des écritures unitaires ou par lot dans `stg_entry` (niveau `raw`). Trois endpoints
REST dédiés (`prototype/rust/src/entries.rs`) : `POST /api/entries` (batch), `PUT
/api/entries/{id}`, `DELETE /api/entries/{id}`.

| Aspect | Comportement |
|---|---|
| **Cible** | `stg_entry` (saisie brute, niveau `raw`). Pipeline non relancé automatiquement — l'utilisateur déclenche `/api/run`. |
| **Schéma** | `stg_entry` gagne une **PK `id`** auto-incrémentée (seq dédiée `seq_stg_entry`) ; `get_entries?level=raw` renvoie le vrai id et la colonne `source`. |
| **Marqueur** | `source = 'MANUAL'` forcé à l'INSERT (champ existant non propagé par le pipeline). |
| **Protection** | PUT/DELETE refusés si `source ≠ MANUAL` (anti-écrasement des imports CSV). Insert-only sur le POST (jamais d'écrasement). |
| **Validation** | Champs obligatoires + cohérence référentielle (FK), transaction atomique au POST (lot entier valide ou rien). |
| **En-tête commun** | 6 champs factorisés (Phase, Entité, Exercice, Période, Devise, Nature) en haut du batch, pré-remplissent chaque nouvelle ligne. Bouton « ↧ Appliquer partout » pour propager aux lignes existantes. Grille allégée par défaut (Account, Flow, Partner, Titre, Analysis×2, Amount) avec toggle pour afficher les colonnes communes (override au cas par cas). |
| **Distinction visuelle** | `EcrituresPage` surligne les lignes `source=MANUAL` (classe `row--manual`) pour la traçabilité. |

⚠️ Le schéma `stg_entry` ayant évolué (`id` PK), un `POST /api/reset` ou `CONSO_FORCE_RESEED=1`
est nécessaire après rebuild pour reconstruire la base.

## Libellés & UX — [Q37], [Q38]

✅ **Dimension `share`** : libellé renommé « **Titre** » (au lieu de « Quote-part » qui était une
traduction ambiguë). Le nom technique `share` est inchangé.

✅ **Dropdowns au format `code - libellé`** dans toute l'UI (Rules, Saisie, Filters, Master data,
Pipeline) via le helper central `formatOptionLabel(code, libelle)` (`web/src/utils/format.ts`).
Le hook `useDimValues` expose désormais `{code, libelle}[]` au lieu de `string[]`. Les dropdowns
techniques (level, opérateur, type de coefficient…) restent en code seul.

## Recette (config ≠ moteur)

✅ Le **moteur** est une mécanique pure (couverte par les tests Rust). La **justesse comptable**
d'une configuration donnée (interco, équivalence…) relève de la **recette** : smoke tests Python.
→ [`RECETTE_PYTHON.md`](./RECETTE_PYTHON.md). 🟡 Golden interco à porter en smoke test Python.

## Qualité

✅ Suite de tests Rust **verte** : `tests/pipeline.rs`, `tests/rules.rs`, `tests/a_nouveau.rs` +
tests unitaires de lib. ✅ Build web (tsc) vert. Performance : critère de validation (benchmark
`conso-bench` sur gros volumes).

---

## Reste à trancher (avant 1ʳᵉ implémentation élargie)

Questions `TÔT` encore ouvertes — voir [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) :
[Q6] mode complète/marge · [Q8] workflow de validation · [Q9] granularité de clôture ·
[Q12] cible de performance chiffrée.
