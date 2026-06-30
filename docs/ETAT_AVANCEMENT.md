# État d'avancement

> Vue consolidée de **ce qui est implémenté**, de son **comportement**, et de **ce qui reste**.
> Pour le *pourquoi* d'une décision → [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) ;
> pour le détail fonctionnel → les docs thématiques liées ci-dessous.
> Dernière mise à jour : **2026-06-30**.

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

✅ Entrée (F00 → F01), sortie (miroir −F98) et variation de % d'intégration (F90/F95)
:**pilotées par règles** depuis la suppression du niveau `reclassified` (Q31 —
ces traitements ne sont plus natifs). L'utilisateur compose les opérations dans
l'éditeur de règles (cf. section dédiée) en scannant `sat_perimeter` ou le
snapshot N-1 de l'à-nouveau. Les anciens tests natifs sont `#[ignore` — ils
testaient un comportement aujourd'hui délégué aux règles.
→ [`FLUX_CONSO.md`](./FLUX_CONSO.md) §9, [`A_NOUVEAU.md`](./A_NOUVEAU.md).

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
→ Spec : [`A_NOUVEAU.md`](./A_NOUVEAU.md) / [`A_NOUVEAU_IMPL.md`](./archive/specs-livrees/A_NOUVEAU_IMPL.md).

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
✅ **Coefficient d'une opération = formule** (volet 1 du moteur de formules, voir ci-dessous) :
les anciens coefficients en dur (`pct_integration`, `elim_ic_corp_*`…) sont devenus des **formules
nommées** de la bibliothèque ; le menu Coefficient des règles liste natifs + formules utilisateur.
⬜ Catalogue métier à composer : interco avancées, intérêts minoritaires, retraitements,
variations de capital, répartition des résultats. → [`REGLES_CONSO.md`](./REGLES_CONSO.md) §10.

## Moteur de formules (coefficients & indicateurs) — [Q43]

✅ **Langage type Excel** (`prototype/rust/src/formula.rs`, moteur pur) : lexer → parser → AST,
`+ − × ÷`, `MIN`/`MAX`/`IF`/`ABS`/`ROUND`/`SAFE_DIV`, références `[ … ]`, séparateur `;`. Deux
cibles de compilation : **SQL** (exécution) et **f64** (preview live). Un seul moteur, **deux
catalogues d'opérandes** selon le contexte. Sécurité SQL identique aux règles. → [`FORMULES.md`](./FORMULES.md).

**Volet 1 — Coefficients** (`coefficients.rs`) : bibliothèque `dim_coefficient` (natifs **seedés
comme formules** + coefficients utilisateur, survit au reset). Une formule compile vers
`(SQL, CoeffJoins)` — point d'insertion exact de l'ancien `coefficient_expr`. Opérandes = valeurs
de `sat_perimeter` aux **4 perspectives** (`entity`/`partner`/`entity_n1`/`partner_n1`), défaut **0**
(décision F3 : pas de neutralité magique, `SAFE_DIV` à la charge de l'utilisateur). API REST
`/api/coefficients` (+ `/operands`, `/preview`). UI : `web/src/pages/CoefficientsPage.tsx` (éditeur
+ opérandes insérables + preview live).

**Volet 2 — Indicateurs / KPI** (`indicators.rs`) : **postes** (`dim_aggregate` = sélection nommée
sur `fact_entry`, traversées `via`/`ref`/`attr` comprises) + **indicateurs** (`dim_indicator` =
formule combinant des postes, à un **grain** de restitution). Compilé en **une** requête au grain :
`SUM(amount) FILTER (WHERE …)` par poste, **LEFT JOINs partagés** (un poste ne filtre pas les lignes
des autres), renvois entre indicateurs avec détection de cycle. **Non-additif** (calculé au grain,
jamais sommé) et **jamais réinjecté dans `fact_entry`**. API REST `/api/aggregates`,
`/api/indicators` (+ `/operands`, `/preview`). UI : `web/src/pages/IndicatorsPage.tsx` (sous-onglets
Postes + Indicateurs, éditeur de sélection, formule + grain + preview live).

⬜ Colonnes KPI directement dans les rapports, dashboard de cartes, intelligence temporelle N-1
(`[CA · N-1]` via l'à-nouveau, prévue par opérande).

## Restitutions

✅ Table consolidée **filtrable**, **bilan par flux**, **compte de résultat** — filtrables par
nature, avec **détail par nature** dépliable. Les totaux excluent les « of which ».
✅ **Indicateurs / KPI** calculés à un grain (page Indicateurs, voir ci-dessus).
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

## MCP — pilotage par agent IA - [Q54] ✅

✅ **Serveur MCP intégré** au binaire `conso-server` (SDK `rmcp`), exposant **10
outils typés** pour les agents IA (opencode, Claude…) : `describe_model`,
`list_master_data`, `upsert_master_data`, `import_entries`, `get_entries`,
`run_consolidation`, `run_controls`, `get_bilan`, `get_compte_resultat`,
`get_indicator`. Cœur métier partagé HTTP↔MCP (`conso_engine::reports`).

✅ **Deux modes** de transport :
- **stdio** (`conso-server --mcp`) : opencode spawn le process, idéal pour une
  session ad-hoc sans serveur HTTP. Process séparé → base DuckDB dédiée
  (bac à sable) ou exclusive (verrou si même fichier que l'UI).
- **HTTP** (route `/mcp` sur le serveur Axum existant) : l'agent se connecte en
  MCP remote, **même process que l'UI** → même base DuckDB partagée, **UI et
  agent simultanés sans verrou**. Mode recommandé pour le travail sur données
  réelles.

✅ **REST bulk/pagination/recherche** (Q54 phase 1) : `PUT/DELETE
/api/md/{table}/bulk`, `?limit&offset` (enveloppe opt-in), `?search=` (ILIKE
`libelle`), filtres dynamiques `?{col}=`, `?enrich=true` (FK + libellé).

✅ **Smoke test** stdio automatisé (`tests/mcp_smoke.ps1`, 19 checks) + recette
via opencode (4 scénarios : lecture, saisie+run, rapports, contrôles).

→ Détail : [`MCP.md`](./MCP.md).

## Libellés & UX — [Q37], [Q38]

✅ **Dimension `share`** : libellé renommé « **Titre** » (au lieu de « Quote-part » qui était une
traduction ambiguë). Le nom technique `share` est inchangé.

✅ **Dropdowns au format `code - libellé`** dans toute l'UI (Rules, Saisie, Filters, Master data,
Pipeline) via le helper central `formatOptionLabel(code, libelle)` (`web/src/utils/format.ts`).
Le hook `useDimValues` expose désormais `{code, libelle}[]` au lieu de `string[]`. Les dropdowns
techniques (level, opérateur, type de coefficient…) restent en code seul.

## Recette (config ≠ moteur)

✅ Le **moteur** est une mécanique pure (couverte par les tests Rust). La **justesse comptable**
d'une configuration donnée (interco, équivalence, variations de périmètre par règles) relève de
la **recette** — validée end-to-end sur un cas réel complet (saisies + pipeline + ruleset
interco + à-nouveau + UI). Les anciens scripts Python (`golden_test.py` / `rules_test.py` /
`smoke_test.py`) ont été retirés lors du chantier migration CSV→JSON (cf.
[`PLAN_MIGRATION_CSV_JSON.md`](./archive/specs-livrees/PLAN_MIGRATION_CSV_JSON.md)).

## Performance — `conso-bench` ([Q12], [Q3])

✅ **Mesurée** sur 3 volumétries via `conso-bench` (binaire `src/bin/bench.rs`). Jeu généré :
60 entités × 200 comptes × 5 devises × F00/F20, périmètre avec méthodes mixtes + entrantes/
sortantes. Pipeline mesuré sur DuckDB **fichier** (cas réel). Le bench référence un ruleset
d'élimination interco (`RS_BENCH_INTERCO`, partenaire rempli sur 30 % des écritures) pour
mesurer aussi le hook règles — flag `--no-rules` pour comparer en natif.

### Natif (sans règles)

| `stg_entry` | corporate | converted | consolidated | Total | Débit global |
|---:|---:|---:|---:|---:|---:|
| 100 k | 0,49 s | 0,95 s | 1,22 s | **2,66 s** | 38 k/s |
| 1 M | 3,17 s | 10,1 s | 10,2 s | **23,4 s** | 43 k/s |
| 5 M | 14,7 s | 44,7 s | 52,0 s | **111 s** | 45 k/s |

### Avec ruleset interco (`RS_BENCH_INTERCO`, 2 opérations)

| `stg_entry` | consolidated (lignes) | Total | Surcoût hook |
|---:|---:|---:|---:|
| 1 M | 3,51 M (+1,03 M écritures 2ELI) | **27,6 s** | +4,2 s (+18 %) |
| 5 M | 17,5 M (+5,14 M écritures 2ELI) | **128 s** | +17 s (+15 %) |

**Lecture** :
- Étape **A (corporate / agrégation)** : la plus rapide — **630–700 k lignes/s** (DuckDB vectorisé).
- Étapes **C (convert) et D (consolidate)** : ~245–280 k lignes/s — **les goulots natifs**
  (2 branches d'écriture en C, JOINs `v_flow_behavior` + `sat_exchange_rate`, ×2,5 plus de
  lignes qu'en A). Le coût est structurel, peu optimisable sans casser la sémantique.
- **Hook règles pas critique** : +15–18 % pour produire 5 M lignes d'élimination au niveau
  consolidated (snapshot + INSERT par opération). Scale linéairement, ~3 µs/ligne 2ELI.
- **Débit global stable ~40–45 k lignes stg/s** sur gros volumes. 5 M lignes traitées en
  ~2 min en natif, ~2 min 8 s avec ruleset — conforme à l'obligation de moyens ([Q12]).
- Validation clôtures + invariants F80/F81 tenus à toutes les échelles, avec ou sans ruleset.

### Optimisations testées (2026-06-29)

- **Index secondaires** (`fact_entry (consolidation_id, level)`, `(account, flow)`,
  `stg_entry (phase, entry_period)`) : **rejetés**. Sur DuckDB columnar + écriture massive
  d'un niveau complet par étape, le coût de maintenance de l'ART dépasse le gain en lecture
  (1M : 22 s → 53 s, ×2,4 plus lent). Les zone-maps suffisent car `consolidation_id` est
  physiquement groupé (1 valeur par run, insertion par étape).
- **`PRAGMA preserve_insertion_order=false`** : adopté dans le bench (neutre, bonne pratique).
- **Retrait du `COUNT` final dans `materialize_closures`** (valeur non utilisée) : adopté.

→ Détails dans `prototype/rust/src/bin/bench.rs` ; recette via
`cargo run --release --bin conso-bench -- --rows 1000000` (ajouter `--no-rules` pour le natif).

## Qualité

✅ Suite de tests Rust **verte** : `tests/pipeline.rs`, `tests/rules.rs`, `tests/a_nouveau.rs`,
`tests/loader.rs` + tests unitaires de lib (185 lib + 19 integration). ✅ Build web (tsc) vert.
✅ `conso-bench` vert (identités de clôture + invariants F80/F81 tenus jusqu'à 5 M lignes).

---

## Backlog technique (TODO)

Tâches techniques identifiées, non bloquantes mais à ne pas perdre. Priorité
entre `()`.

- **(HAUTE)** Ajouter un test de régression pour la **conversion triangulaire
  cross-currency** (USD→GBP via la devise pivot EUR). Le comportement est prouvé
  en runtime (2026-06-29 : `presentation_currency=GBP`, saisie USD 1000 →
  F20 = 805,08 GBP = 1000 × `taux_moyen(USD) 0,95 / taux_moyen(GBP) 1,18`) mais
  n'est pas gardé par un test automatisé → risque de régression silencieuse.
  Cas à couvrir dans `tests/pipeline.rs` (ou un nouveau `tests/convert.rs`) :
  - construire une consolidation avec `presentation_currency` = une devise
    **non-pivot** (GBP), pivot = EUR ;
  - importer une écriture en USD (entité dont la devise fonctionnelle = USD),
    flux F20 (avg) puis F00 (close) ;
  - `run_pipeline` puis assert au niveau `converted` :
    `amount × (taux(USD→EUR) / taux(GBP→EUR))` pour chaque schéma (avg/close) ;
  - vérifier aussi l'écart F81 = `amount × (cross_report − cross_flux)`.
  - Réutiliser le seed `tests/fixtures/seed.json` (désormais pourvu de
    `flow_scheme` + taux USD/GBP vs EUR dans le rate_set `RATES`).
  - Réf. formule : `prototype/rust/src/pipeline/convert.rs:155-170` (CTE `conv`).

---

## Reste à trancher (avant 1ʳᵉ implémentation élargie)

Questions `TÔT` encore ouvertes — voir [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) :
[Q6] mode complète/marge · [Q8] workflow de validation · [Q9] granularité de clôture.
