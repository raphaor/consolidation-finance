# À-nouveau — Suivi d'implémentation (tracker transitoire)

> Fichier de **suivi de chantier**, à supprimer quand l'à-nouveau est livré.
> Spec de référence : [`A_NOUVEAU.md`](./A_NOUVEAU.md). Plan complet en bas.
>
> **Branche** : `feat/a-nouveau` (depuis `docs/a-nouveau`).
> **Reprise** : `git log --oneline` pour voir où on en est, puis reprendre à la
> 1ʳᵉ phase non cochée ci-dessous. Build/test depuis `prototype/rust/` :
> `cargo build --release` puis `cargo test --release`.
> **Le serveur n'est jamais lancé par l'agent** (cf. CLAUDE.md anti-blocage).

## État des phases

- [x] **Phase 0 — Schéma & seed** (additif, ne casse rien) ✅ commit
  - [x] `schema.rs` : `dim_flow.flux_a_nouveau`, `dim_scenario.a_nouveau_scenario`
  - [x] `loader.rs` : colonnes flows + scenarios
  - [x] `masterdata.rs` : colonnes flows + scenarios
  - [x] `seed.rs` + `bench.rs` + `tests/pipeline.rs` : INSERT positionnels → listes de colonnes explicites (sinon cassés par la nouvelle colonne)
  - [x] CSV `data/` flows (F99→F00) + scenarios (a_nouveau vide) ; `data_golden/flows.csv` idem
  - [x] build + 26 tests Rust verts
  - ⚠️ `data_golden/scenarios.csv` est en schéma **v1 (4 colonnes)**, déjà désaligné du loader v2 **avant** ce chantier → golden serveur déjà cassé sur scenarios (recette, Phase 7). Non touché.
- [x] **Phase 1 — Suppression `reclassified`** (refactor 3 niveaux) ✅ commit
  - [x] `pipeline/mod.rs` (retrait ReclassifyStep, arrays →3, closures après CHAQUE étape), `convert.rs` (lit `corporate`)
  - [x] suppr. `pipeline/reclassify.rs` + `pub mod reclassify;`
  - [x] `rules.rs` ALLOWED_LEVELS, `validate.rs` (validate_functional → corporate), `report.rs`, `main.rs`, `dump_pipeline.rs`, `bench.rs`, `server.rs` stats + doc (lib.rs, schema.rs, staging.rs, materialize_closures.rs)
  - [x] `schema.rs` CHECK level (3 valeurs)
  - [x] **closures au corporate OUI** (révisé vs guide) : run_steps matérialise après chaque étape → `validate_functional` repointé sur corporate, reste vivant
  - [x] MAJ tests Rust : 14 passed + 2 `#[ignore]` (montants & sortie périmètre = Phase 7) ; rules 10/10. Build OK.
  - ⚠️ **Phase 6** : `web/src` référence `reclassified` (PipelinePage.tsx, RulesPage.tsx, types.ts) + le champ `reclassified` retiré de la réponse `/api/run` → à traiter avec l'UI.
- [x] **Phase 2 — Isolation scénario + filtre scope corporate** ✅ commit
  - [x] `server.rs` DELETE fact_entry WHERE scenario (préserve snapshots figés)
  - [x] `aggregate::step_a(con, scenario)` : filtre `s.scenario = ?` + INNER JOIN `sat_perimeter` (scope, toutes méthodes, entrantes/sortantes incluses) ; colonnes préfixées `s.`
  - [x] build + tests verts (no-op sur seed mono-scénario : toutes entités dans périmètre REEL)
- [x] **Phase 3 — Cœur à-nouveau** ✅ commit
  - [x] `ConvertParams::load_params` charge `a_nouveau_scenario` (+ champ struct)
  - [x] nouveau module `pipeline/a_nouveau.rs` : `carry(con, params, level)` générique via `dim_flow.flux_a_nouveau` (pairs source→target), câblé dans `run_steps` (corporate + consolidated, avant clôtures)
  - [x] détection consolidée-en-N1 = EXISTS F99 consolidé snapshot **∩ scope courant** (`sat_perimeter`) → continues/sortantes seulement
  - [x] carry corporate (écrase liasse F00) + carry consolidé (écrase le F00 produit par step_d → reste au % N-1). **Pas d'exemption dans step_d** : l'écrasement post-step_d est plus simple et équivalent.
  - [x] **Correctif d'isolation (lacune Phase 2)** : `step_c`/`step_d` lisaient leur niveau sans filtrer le scénario → duplication des lignes du snapshot lors d'un 2ᵉ run. Ajout `AND f.scenario = ?` aux deux.
  - [x] test dédié `tests/a_nouveau.rs` (2 tests) : report corporate+consolidé, liasse écrasée, clôture qui se referme ; + contrôle sans-à-nouveau. Build + suite verts (36 + 14/2ignored + 10 + 2).
- [x] **Phase 4 — Staging cible** ✅ commit
  - [x] préfixe `2` → consommé dans `step_c` (UNION corporate + stg préfixe 2 fonctionnel) → conversion + écarts, absent du corporate
  - [x] préfixe `3` → consommé dans `step_d` (UNION converti + stg préfixe 3) → subit le × pct_integration (JOIN sat_perimeter), absent du converti
  - [x] préfixe `4` → injection post-step au consolidé (tel quel) via `inject_by_prefix` (seul préfixe routé par `staging.rs`)
  - [x] `ConvertStep` ne déclare plus de staging post-étape ; doc `staging.rs` MAJ ; test `staging_route_les_prefixes_vers_le_bon_niveau` réécrit (2/3/4) — vert
  - ⚠️ priorité ouverture (F00 staffé en préfixe ignoré) : non spécifiquement codée ; le carry à-nouveau écrase le F00 de toute façon. À couvrir en Phase 5 si besoin.
- [x] **Phase 5 — Validation** ✅ commit (runtime confirmé)
  - [x] `validate::check_a_nouveau_coherence(con, scenario, a_nouveau, period)` : détecte (1) divergence `entree` saisi vs présence au snapshot, (2) entités consolidées en N-1 hors périmètre courant (orphelines). Non bloquant.
  - [x] `server.rs` : appel après le run → **avertissement** (eprintln), conforme A5 (statut `ouvert` toléré)
  - [x] 2 tests d'intégration (`tests/a_nouveau.rs`) : divergences+orphelins, et cas aligné→vide
  - [x] **suite complète verte** (serveur arrêté) : lib 36 + a_nouveau 4 + pipeline 14/2 ignorés + rules 10.
- [x] **Phase 6 — API / UI** ✅ commit
  - [x] `server.rs` : `ScenarioSummary` (+ SELECT `list_scenarios`) renvoie `a_nouveau_scenario` ; `/api/run` déjà à 3 niveaux (Phase 1)
  - [x] `types.ts` : `PipelineCounts`/`LEVELS` à 3 niveaux ; `Scenario`/`ScenarioSummary` + `a_nouveau_scenario` ; colonne CRUD `a_nouveau_scenario` (scenarios) + `flux_a_nouveau` (flows)
  - [x] `PipelinePage.tsx` : STEP `reclassified` retiré, ligne « Conso d'à-nouveau » affichée ; `RulesPage.tsx` : `LEVELS_LIST` à 3 niveaux
  - [x] `npm run build` (tsc + vite) vert ; suite Rust verte après recompil serveur
- [ ] **Phase 7 — Tests & règles** : règles corporate (UTILISATEUR), tests Rust, golden, recette Python (écarts préfixe 2)

## Décisions clés (rappel, détail dans A_NOUVEAU.md)

- À-nouveau = snapshot N-1 figé ; carry **corporate** (écrase liasse, base écart F80 + report F99) + **consolidé** (fige % N-1) ; converti déduit par conversion native ; F00 exempté du `× pct` seulement.
- « Consolidée en N-1 » = F99 consolidé présent dans le snapshot. Entrant **dérivé** de l'absence + contrôle de cohérence.
- Périmètre (F00→F01, F98) et variation de % = **règles utilisateur**, pas le moteur.
- Staging intérimaire (préfixe de nature fragile) : `0/1`→corporate, `2`→converti, `3`/`4`→consolidé avant/après %.

## Journal

- **Phase 0 faite** : schéma (2 colonnes nullables) + loader + masterdata + seed/bench/test (listes de colonnes explicites) + CSV. `cargo build` + `cargo test` (16+10) verts. Aucun impact pipeline (colonnes inertes tant que la Phase 3 ne les lit pas).
- **NEXT → Phase 1** : suppression de `reclassified`. ⚠️ Casse golden serveur (déjà partiellement cassé) et retire le périmètre natif (F00→F01, F98) → résultats divergents tant que les règles utilisateur n'existent pas. Prévoir MAJ des tests Rust `tests/pipeline.rs` (assertions sur `reclassified`, F01/F98) en même temps.
- **Arrêt session 2026-06-20** sur checkpoint **Phase 0 vert** (commit `db8307f`). Phase 1 non démarrée : pas de sous-ensemble neutre, risque de branche cassée si interrompue. Guide d'exécution prêt ci-dessous.
- **Phase 1 faite (2026-06-21)** : `reclassified` supprimé du programme entier ; pipeline A→C→D (convert lit corporate) ; clôtures reconstruites **après chaque étape, corporate inclus** (corporate devient point de traitement, `validate_functional` repointé dessus). Périmètre natif (F00→F01, F98) retiré → 2 tests `#[ignore]` (rétablis par règles en Phase 7). Build + tests verts (14+2ignored / 10).
- **Phase 2 faite (2026-06-21)** : `step_a` filtre par scénario du run + INNER JOIN `sat_perimeter` (scope) ; `server.rs` purge `fact_entry` par scénario (snapshots préservés). No-op sur seed mono-scénario, tests verts.
- **Phase 3 faite (2026-06-21)** : module `pipeline/a_nouveau.rs` (carry générique corporate+consolidé, intersection snapshot∩scope) ; `ConvertParams.a_nouveau_scenario` ; **isolation complétée** (convert+consolidate filtrent le scénario — sinon duplication du snapshot). Approche retenue : écrasement du F00 **après** step_d (plus simple que l'exemption dans step_d, résultat identique). Test `tests/a_nouveau.rs` vert.
- **NEXT → Phase 4** (staging cible : préfixe 2→converti fonctionnel, 3/4→consolidé avant/après %) ou **Phase 6** (UI/API : champs `a_nouveau_scenario`/`flux_a_nouveau`, retrait `reclassified` du frontend). Le cœur moteur de l'à-nouveau (Phases 0–3) est fonctionnel et testé. Restent surtout : staging fin (Phase 4), validation cohérence (Phase 5), UI (Phase 6), règles utilisateur + recette (Phase 7).
- **Note** : `materialize_closures` reconstruit encore les clôtures de TOUS les scénarios à chaque run (idempotent, valeurs identiques pour le snapshot figé → sans danger ; optimisation possible : scoper par scénario).
- **Phase 4 faite (2026-06-21)** : staging cible sur 3 niveaux. Préfixes 2 et 3 consommés DANS step_c/step_d (UNION fact_entry + stg) → ils subissent conversion / × pct ; préfixe 4 injecté tel quel après step_d. Test staging réécrit, vert (9 binaires OK, 0 FAILED).
- **Phase 5 faite (2026-06-21)** : `check_a_nouveau_coherence` (divergence entree/snapshot + orphelins), avertissement non bloquant côté serveur, 2 tests d'intégration. **Suite complète verte** (serveur arrêté) : lib 36 + a_nouveau 4 + pipeline 14/2 ignorés + rules 10.
- **Moteur de l'à-nouveau terminé (Phases 0→5).**
- **Phase 6 faite (2026-06-21)** : API (`ScenarioSummary` expose `a_nouveau_scenario`) + frontend (CRUD scenarios/flows avec les nouveaux champs, pipeline/règles à 3 niveaux, affichage conso d'à-nouveau). `npm run build` vert, suite Rust verte. `reclassified` totalement retiré du frontend.
- **NEXT → Phase 7 (toi/recette)** : règles corporate (F00→F01 entrée, miroir F98 sortie, variation de % d'intégration) à créer dans l'éditeur de règles + recette golden serveur (`golden_test.py`) à recaler sur le pipeline 3 niveaux et le staging 2/3/4. Les 2 tests Rust `#[ignore]` (sortie périmètre, montants consolidés) redeviendront pertinents une fois les règles livrées.

---

## Guide Phase 1 (prêt à exécuter)

Touchpoints `reclassified` repérés (grep) — à traiter tous :

1. **`pipeline/mod.rs`** : retirer `pub mod reclassify;` ; retirer `ReclassifyStep` (struct + impl) ; `steps = [Aggregate, Convert, Consolidate]` dans `run_pipeline_with_hook` ; `LevelCounts = [usize; 3]` ; `PipelineReport.steps: [StepTiming; 3]` ; `counts()` →3 entrées ; `run_steps` `try_into` "3 étapes" ; MAJ doc-comments A→C→D.
2. **`pipeline/convert.rs`** : `WHERE f.level = 'reclassified'` → `'corporate'` (+ doc-comment).
3. **`pipeline/reclassify.rs`** : **supprimer le fichier**.
4. **`schema.rs`** : `CHECK (level IN ('corporate','converted','consolidated'))` + MAJ tableau doc en tête.
5. **`rules.rs`** : `ALLOWED_LEVELS` sans `"reclassified"`.
6. **`validate.rs`** : retirer le check clôtures `reclassified` (fn dédiée + appel). Garder converted/consolidated. (Le test `validate_f99_functional` cible reclassified → à retirer ou repointer.)
7. **`report.rs`** : retirer `"reclassified"` des tableaux de niveaux (l.163, 166, 260, 299).
8. **`main.rs`** : `labels` (l.111) → 3 niveaux.
9. **`dump_pipeline.rs`** : refs reclassified (l.50, 62, 69, 76, 84).
10. **`bench.rs`** : boucle l.328 `["corporate","reclassified"]` → enlever reclassified.
11. **`server.rs`** : struct stats champ `reclassified: usize` (l.88) ; CASE ordre niveaux (l.213) ; mapping `counts[1]`/`counts[2]`/`counts[3]` → décaler (l.539) ; println l.1212.

**Closures au corporate** : NE PAS les ajouter en Phase 1 (recommandé). Garder le comportement actuel (corporate sans F99 reconstruit, 1ʳᵉ reconstruction au converti). Les closures corporate viendront en Phase 3/4 avec le carry + règles de périmètre. → évite de changer les comptages corporate.

**Tests Rust à MAJ (`tests/pipeline.rs`)** — comportement périmètre natif disparu :
- `pipeline_produit_les_bons_comptes_par_niveau` : comptages → 3 niveaux, retirer reclassified.
- `sortie_perimetre_donne_f99_zero_et_f98_negatif` : le miroir F98 natif n'existe plus → **`#[ignore]`** avec note « rétabli en Phase 7 via règle », ou supprimer.
- `validate_f99_functional` (clôtures reclassified) : retirer ou repointer corporate.
- Tout test/asserts citant `reclassified`, `F01`, `F98`.

Build incrémental ~OK (DuckDB déjà compilé). `cargo build --release` puis `cargo test --release`. Commit seulement si vert.
