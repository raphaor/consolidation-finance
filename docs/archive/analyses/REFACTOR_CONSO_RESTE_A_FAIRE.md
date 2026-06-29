# Refactor consolidation — reste à faire

> **ARCHIVÉ (2026-06-29)** — ce document suit le chantier `scenario → consolidation`
> livré en 2026-06-23 (Q41/Q42). Tous les points ouverts sont aujourd'hui clos :
> les variations de périmètre passent par les règles (Q31), la recette
> end-to-end a été validée, le bug `conso-bench` est corrigé (cf.
> [`ETAT_AVANCEMENT.md`](./ETAT_AVANCEMENT.md) § Performance). Conservé pour
> la traçabilité historique du chantier Q41/Q42.

Suivi du chantier **scenario → consolidation** (+ taux d'ouverture).
État au 2026-06-23. Deux temps livrés et validés (`cargo test` 115 verts, `npm run build` OK, `dump_pipeline` et smoke serveur OK).

---

## ✅ Terminé

### Temps 1 — `taux_ouverture` + fin du `prev_period` (commit `96d3df3`)
- Nouvelle colonne `sat_exchange_rate.taux_ouverture` (= clôture N-1, **portée par N**).
- `convert.rs` : la branche `close_n1` (F00/F01) lit `taux_ouverture` ; JOIN `r_n1`/`r_pres_n1` supprimés.
- `ConvertParams.prev_period` + sa dérivation supprimés → **1ʳᵉ consolidation possible**, avec ou sans à-nouveau. Corrige l'erreur *« Query returned no rows »* sur REEL_2023.
- CSV rates (data / smoke / golden) + `golden_test.py` : `taux_ouverture(2024)` = clôture 2023.

### Temps 2 — Redesign identité (commits Rust `xxx` + web `e5d6404`)
- **`dim_consolidation`** (ex `dim_scenario`) : PK technique `id` auto + **clé naturelle UNIQUE** `(phase, exercice, perimeter_set, variant, presentation_currency)`. `code` disparaît. `category`→`phase`, `entry_period`→`exercice`, `a_nouveau_scenario`→`a_nouveau_consolidation_id`. **Périodes explicites** `perimeter_period` + `rate_period` (défaut = exercice).
- **Remontée** : `stg_entry.scenario`→`phase` (saisies au grain phase+entry_period, partagées entre consolidations).
- **`fact_entry`** : `scenario`→`phase` (dim propagée) **+ `consolidation_id`** (col. technique, isole chaque run).
- Pipeline : isolation par `consolidation_id` ; filtre remontée `phase`+`entry_period` ; rates à `rate_period` ; grain de clôture = `consolidation_id` ++ grain. **Règles : snapshot filtré + écritures taguées `consolidation_id`** (isolation au re-run).
- API : `/api/scenarios`→`/api/consolidations`, `/api/run` prend `consolidation_id` (entier). Export bump `conso-export-v2`. CRUD master data `consolidations` avec PK auto.
- Frontend React aligné (types, Saisie « Phase », PipelinePage par id, Filters, MasterDataPage PK auto, api.ts).
- Docs vivantes : `AGENTS.md`, `MODELE_DONNEES.md`, `QUESTIONS_OUVERTES.md` (Q41/Q42), `A_NOUVEAU.md`.

---

## 🟡 Reste à faire

### 1. Docs (finir) — ✅ FAIT (2026-06-24)
- [x] `docs/ETAT_AVANCEMENT.md` : section « Scénario (v2) » → « Consolidation (v3) » [Q41] ; périmètre/taux référencés par `dim_consolidation` avec `perimeter_period`/`rate_period` ; mention `taux_ouverture` ; à-nouveau → `a_nouveau_consolidation_id` ; agrégation filtrée phase+exercice ; en-tête Saisie « Phase » ; date MAJ.
- [x] `docs/A_NOUVEAU_IMPL.md` : bandeau « post-Q41 » (renommages historiques).
- [x] `docs/archive/specs-livrees/SPEC_SCENARIO_V2.md` + `SPEC_SCENARIO_V2_TECH.md` : bandeaux **« SUPERSEDÉ par Q41 »** (non réécrits).
- [x] `EXPRESSION_DE_BESOIN.md` : « multi-scénarios » → « multi-phases » ; « entre scénarios » → « entre consolidations ».
- [x] `docs/FLUX_CONSO.md` : aucune mention `scenario` (rien à faire).

### 2. Scripts Python de recette — ✅ FAIT (2026-06-24), golden à recalculer au runtime
Décision retenue : **migrés et gardés vivants en Python** (cohérent avec la stratégie de tests : recette = smoke tests Python). Paramètres exacts de la nouvelle API : query `consolidation` (entier) aux niveaux `fact_entry`, `phase` (texte) au niveau `raw` ; body `/api/run` = `{"consolidation_id": N}` ; `/api/scenarios`→`/api/consolidations` ; md table `scenarios`→`consolidations`.
- [x] `smoke_test.py` : id résolu via `/api/consolidations` (phase+exercice) ; run par id ; bilan/CR par `consolidation=` ; filtre `phase` au niveau raw ; **3 niveaux** (drop `reclassified`).
- [x] `rules_test.py` : **`/api/rules/run` supprimée** → le ruleset est porté par `dim_consolidation.ruleset_code` (PUT master data) et exécuté dans `/api/run` (hook par niveau) ; rapport lu dans `body.ruleset_report`.
- [x] `golden_test.py` : infra cliente migrée (id, run par id, pagination). **Réserve** : le golden master encode l'ancien modèle 4 niveaux (`reclassified`) + sortie F98 native ; bloc `S` et invariants reclassified (2b/3/4/6b/7b/8) **obsolètes** → bandeau STALE posé, invariants neutralisés. **Valeurs golden à recalculer contre le moteur 3-niveaux en marche** (tâche runtime, liée à la recette périmètre-par-règles §4).

### 3. Données & environnement — priorité haute (utilisateur)
- [ ] **La base `prototype/rust/conso.duckdb` a été reset** lors du smoke test (CONSO_FORCE_RESEED=1). Elle ne contient plus que la consolidation `REEL` issue de `data/consolidations.csv`, **sans saisies**. Re-saisir les liasses (onglet Saisie, colonne **Phase**) et recréer `REEL_2023` si besoin.
- [ ] `prototype/rust/data/entries.csv` est vide (en-tête only). Envisager un jeu de saisies minimal pour un défaut utilisable au reset.
- [ ] Pour `REEL_2023` (entry_period 2023) : remplir `taux_ouverture(2023)` pour les devises non-EUR du périmètre (cf. diagnostic initial).

### 4. Recette fonctionnelle end-to-end — priorité haute
- [ ] Reset + saisies (phase `REEL`, exercice 2024) → `POST /api/run {consolidation_id}` → vérifier bilan/compte de résultat non vides.
- [ ] Recette **avec ruleset** (`RS_INTERCO`) : écritures de règle générées **et** isolées par `consolidation_id` ; re-run sans doublons (le fix règles a été validé par tests, à confirmer sur données réelles).
- [ ] Recette **à-nouveau** : créer une consolidation 2023 (N-1), la référencer via `a_nouveau_consolidation_id` sur la 2024, vérifier le carry F99→F00.
- [ ] Parcours UI : Saisie (Phase), Master data `consolidations` (création sans `id` saisi), PipelinePage (run par id), Rapports/Écritures (filtre consolidation).

### 5. Bench (bug préexistant, hors redesign) — priorité basse
- [ ] `conso-bench` produit `consolidated = 0` et un échec d'identité de clôture. **Vérifié préexistant** (avant le refactor). Probablement un problème de périmètre/méthode sur les données générées du bench. À diagnostiquer séparément.

### 6. Détails techniques en suspens
- [ ] `count_level` (rapport de pipeline / `GET /api/levels`) compte toutes lignes d'un niveau, **pas par consolidation**. Si on veut un débit précis par run, filtrer par `consolidation_id`. (Sémantique inchangée par le refactor — préexistant.)
- [ ] Les écritures générées par règles portent désormais `consolidation_id` (correct) ; vérifier que le rapport de ruleset reste cohérent en multi-consolidations.
- [ ] `dump_pipeline.csv` est un artefact régénéré (non suivi / hors commits) — le regénérer après tout changement pour garder la référence lisible.

---

## Références
- Décisions : [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) **Q41** (redesign) + **Q42** (taux_ouverture).
- Modèle : [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) §3 `Consolidation`.
- À-nouveau : [`A_NOUVEAU.md`](./A_NOUVEAU.md) §2.2 / §3.1.
