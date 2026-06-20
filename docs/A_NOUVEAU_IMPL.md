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
- [ ] **Phase 1 — Suppression `reclassified`** (refactor 3 niveaux) ⚠️ casse golden + retire périmètre natif
  - [ ] `pipeline/mod.rs` (retrait ReclassifyStep, arrays →3), `convert.rs` (lit `corporate`)
  - [ ] suppr. `pipeline/reclassify.rs`
  - [ ] `rules.rs` ALLOWED_LEVELS, `validate.rs`, `report.rs`, `main.rs`, `dump_pipeline.rs`, `bench.rs`, `server.rs` stats
  - [ ] `schema.rs` CHECK level (3 valeurs)
  - [ ] corporate gagne `materialize_closures`
  - [ ] régénérer golden, MAJ tests pipeline/rules
- [ ] **Phase 2 — Isolation scénario + filtre scope corporate**
  - [ ] `server.rs` DELETE fact_entry WHERE scenario
  - [ ] `aggregate::step_a` filtre scénario + jointure sat_perimeter
- [ ] **Phase 3 — Cœur à-nouveau**
  - [ ] `ConvertParams::load_params` charge `a_nouveau_scenario`
  - [ ] détection consolidée-en-N1 (EXISTS snapshot)
  - [ ] carry corporate (écrase liasse) + carry consolidé
  - [ ] exemption F00 à `step_d`
- [ ] **Phase 4 — Staging cible + orchestration**
  - [ ] cycle de vie par niveau (pré/transform/post/règles/clôtures)
  - [ ] préfixe 2→converti (fonctionnel), 3→consolidé avant %, 4→après %
  - [ ] priorité ouverture (F00 staffé préfixe 3 ignoré)
- [ ] **Phase 5 — Validation** : contrôle de cohérence entrant/snapshot + clôtures 3 niveaux
- [ ] **Phase 6 — API / UI** : champs `a_nouveau_scenario`, `flux_a_nouveau` ; stats 3 niveaux
- [ ] **Phase 7 — Tests & règles** : règles corporate (UTILISATEUR), tests Rust, golden, recette Python (écarts préfixe 2)

## Décisions clés (rappel, détail dans A_NOUVEAU.md)

- À-nouveau = snapshot N-1 figé ; carry **corporate** (écrase liasse, base écart F80 + report F99) + **consolidé** (fige % N-1) ; converti déduit par conversion native ; F00 exempté du `× pct` seulement.
- « Consolidée en N-1 » = F99 consolidé présent dans le snapshot. Entrant **dérivé** de l'absence + contrôle de cohérence.
- Périmètre (F00→F01, F98) et variation de % = **règles utilisateur**, pas le moteur.
- Staging intérimaire (préfixe de nature fragile) : `0/1`→corporate, `2`→converti, `3`/`4`→consolidé avant/après %.

## Journal

- **Phase 0 faite** : schéma (2 colonnes nullables) + loader + masterdata + seed/bench/test (listes de colonnes explicites) + CSV. `cargo build` + `cargo test` (16+10) verts. Aucun impact pipeline (colonnes inertes tant que la Phase 3 ne les lit pas).
- **NEXT → Phase 1** : suppression de `reclassified`. ⚠️ Casse golden serveur (déjà partiellement cassé) et retire le périmètre natif (F00→F01, F98) → résultats divergents tant que les règles utilisateur n'existent pas. Prévoir MAJ des tests Rust `tests/pipeline.rs` (assertions sur `reclassified`, F01/F98) en même temps.
