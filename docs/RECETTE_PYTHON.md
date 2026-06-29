# Recette Python — Plan de recalibration

> Trois scripts Python de **recette boîte noire** : démarrant le serveur Rust et
> validant son comportement par HTTP. Stdlib Python seule (`urllib`, `subprocess`,
> `argparse`, `json`) — pas de dépendance à installer.
>
> **Statut** : scripts migrés vers la nouvelle API (commit `0e6bc39`) puis supprimés
> lors de la migration CSV→JSON (`3f149ae`). **À restaurer et recalibrer**.

---

## 1. État des lieux

| Élément | État | Action |
|---|---|---|
| `smoke_test.py` | Supprimé (T5) | Restaurer depuis `3f149ae~1` + adapter |
| `rules_test.py` | Supprimé (T5) | Restaurer depuis `3f149ae~1` + adapter |
| `golden_test.py` | Supprimé (T5) | Restaurer depuis `3f149ae~1` + recalibrer les valeurs |
| `data/` (smoke) | Supprimé (T5) | Remplacer par un seed JSON dédié |
| `data_golden/` | Supprimé (T5) | Remplacer par un seed JSON dédié |
| Seed mechanism | CSV → `CONSO_SEED_JSON` | Les scripts doivent passer de `--csv-dir` à `--seed-json` |

## 2. Écarts à combler

### 2.1 Mécanisme de seed

**Avant** : les scripts passaient `CONSO_CSV_DIR` au serveur pour charger les CSV.
**Maintenant** : le serveur lit `CONSO_SEED_JSON` (paquet JSON exporté via `/api/export`).

**Action** :
- Chaque script lance le serveur avec `CONSO_SEED_JSON=<fichier.json>` au lieu de `CONSO_CSV_DIR`.
- L'option `--csv-dir` disparaît, remplacée par `--seed-json PATH`.
- Les datasets sont des **fichiers JSON** (format export `/api/export`) au lieu de dossiers CSV.

### 2.2 Endpoints API

| Ancien | Nouveau | Script concerné |
|---|---|---|
| `POST /api/scenarios` | `POST /api/consolidations` | smoke, rules, golden |
| `GET /api/scenarios` | `GET /api/consolidations` | smoke, rules, golden |
| `POST /api/run` (body `scenario`) | `POST /api/run` (body `consolidation_id`, entier) | smoke, rules, golden |
| `GET /api/bilan?scenario=` | `GET /api/bilan?consolidation=` | smoke, golden |
| `GET /api/compte-resultat?scenario=` | `GET /api/compte-resultat?consolidation=` | smoke, golden |
| `GET /api/entries?scenario=` | `GET /api/entries?phase=&entry_period=` | smoke, golden |
| `GET /api/levels?scenario=` | `GET /api/levels?consolidation=` | smoke, golden |
| `POST /api/rules/run` (supprimée) | Règles via `dim_consolidation.ruleset_code` + `POST /api/run` | rules |

**Note** : les scripts avaient **déjà été migrés** en `0e6bc39` (résolution d'id via `/api/consolidations`, run par `consolidation_id`). Les changements ci-dessus sont déjà pris en compte dans les versions à restaurer.

### 2.3 Niveaux de pipeline

**Avant** : 4 niveaux (`corporate`, `reclassified`, `converted`, `consolidated`)
**Maintenant** : 3 niveaux (`corporate`, `converted`, `consolidated`)

**Impact par script** :

| Script | Impact |
|---|---|
| `smoke_test.py` | Assertions `len(levels_dict) == 4` → `== 3`. Toute assertion sur `reclassified` à supprimer. |
| `rules_test.py` | Niveau cible des règles inchangé (`consolidated`). Pas d'impact direct. |
| `golden_test.py` | **Impact majeur** : les valeurs attendues pour le bloc S (sortie de périmètre) et les invariants `reclassified` sont obsolètes. |

### 2.4 Staging préfixes

| Préfixe | Avant | Maintenant |
|---|---|---|
| `0`, `1` | corporate | corporate (inchangé) |
| `2` | reclassified | **converted** |
| `3` | converted | **consolidated** (avant × pct) |
| `4` | consolidated | **consolidated** (après × pct) |

**Impact** : les valeurs golden pour les natures `2MAN`, `3MAN`, `4MAN` doivent être recalculées selon la nouvelle cible.

### 2.5 Format du périmètre

**Avant** : `perimeter.csv` avec colonne `scenario` (texte).
**Maintenant** : `sat_perimeter` avec colonne `perimeter_set` (clé vers `dim_perimeter_set`), référencé par `dim_consolidation.perimeter_set`.

**Impact** : le seed JSON doit contenir `dim_perimeter_set` + `sat_perimeter` (clé `perimeter_set, entity, period`).

### 2.6 Format des consolidations

**Avant** : `consolidations.csv` avec `libelle,phase,exercice,perimeter_set,...`
**Maintenant** : `dim_consolidation` avec `id` auto, clé naturelle `(phase, exercice, perimeter_set, variant, presentation_currency)`.

**Impact** : le seed JSON contient la table `dim_consolidation` directement (l'id est attribué à l'import).

## 3. Plan de restauration

### T1 — Restaurer les scripts depuis git

```sh
git show 3f149ae~1:prototype/rust/smoke_test.py > prototype/rust/smoke_test.py
git show 3f149ae~1:prototype/rust/rules_test.py > prototype/rust/rules_test.py
git show 3f149ae~1:prototype/rust/golden_test.py > prototype/rust/golden_test.py
```

Les versions restaurées ont déjà la résolution d'id (`/api/consolidations`, `consolidation_id`). Il reste à adapter le mécanisme de seed et les assertions de niveaux.

### T2 — Générer les seed JSON de test

Deux seed JSON à produire (format `/api/export`) :

**`tests/fixtures/seed_smoke.json`** — dataset smoke (couverture large de tous les endpoints) :
- Reprend le contenu de l'ancien `data/` (référentiels + quelques écritures).
- Contenu minimal : `dim_consolidation`, `dim_entity`, `dim_account`, `dim_flow`, `dim_currency`, `dim_nature`, `dim_period`, `dim_scenario_category`, `dim_rate_set`, `sat_exchange_rate`, `dim_perimeter_set`, `sat_perimeter`, `dim_method`, `dim_sous_classe`, `dim_flow_scheme`, `sat_flow_scheme_item`, `dim_variant`, `stg_entry`.

**`tests/fixtures/seed_golden.json`** — dataset golden (non-régression) :
- Reprend le contenu de l'ancien `data_golden/` (5 entités, 3 devises, 3 méthodes, interco, staging).
- **Même structure que `seed_smoke.json`** mais avec les données golden.

**Comment les générer** :
1. Démarrer le serveur avec un seed minimal (`CONSO_SEED_JSON=tests/fixtures/seed.json`).
2. Créer les données via l'API (CRUD master data + import entries).
3. Exporter via `GET /api/export` → sauvegarder comme seed de test.
4. Alternative : construire manuellement le JSON en se basant sur le format de `tests/fixtures/seed.json`.

### T3 — Adapter `smoke_test.py`

Changements à apporter (en plus de la migration déjà faite en `0e6bc39`) :

1. **Seed** : `--csv-dir` → `--seed-json`. Lancer le serveur avec `CONSO_SEED_JSON=<path>`.
2. **Niveaux** : `len(levels_dict) == 4` → `== 3`. Supprimer toute assertion sur `reclassified`.
3. **Endpoints** : vérifier que les endpoints testés existent toujours (ajouter les nouveaux si besoin : `/api/consolidations`, `/api/indicators`, `/api/coefficients`, etc.).

### T4 — Adapter `rules_test.py`

Changements à apporter :

1. **Seed** : idem T3.
2. **Exécution des règles** : la route `POST /api/rules/run` n'existe plus. Le ruleset est porté par `dim_consolidation.ruleset_code` (PUT master data) et exécuté dans `POST /api/run`. **Déjà migré en `0e6bc39`** — vérifier que ça fonctionne.
3. **Rapport** : le rapport de règles est dans `body.ruleset_report` de la réponse `/api/run`. **Déjà migré**.

### T5 — Recalibrer `golden_test.py`

C'est le chantier principal. Les valeurs attendues (`EXPECTED_CONSOLIDATED`) doivent être recalculées pour le pipeline 3 niveaux.

**Changements structurels** :

1. **Supprimer le bloc S** (sortie de périmètre native) — la sortie de S n'est plus produite nativement (elle passe par des règles). Deux options :
   - (a) Neutraliser S dans le golden (supprimer les assertions S).
   - (b) Créer une règle de sortie de périmètre dans le seed et vérifier qu'elle produit F98/F99=0. Option plus complète mais plus complexe.

2. **Recalculer les valeurs pour le pipeline 3 niveaux** :
   - Les valeurs M, G, P, E (hors S) restent identiques (agrégation → conversion → consolidation, pas de changement sur ces entités).
   - Les valeurs de staging `2MAN` changent de cible : `2` → converted (au lieu de reclassified). Le montant sera maintenant soumis à la conversion (FX) avant la consolidation.
   - Les valeurs de staging `3MAN` et `4MAN` restent au consolidated (inchangé de sens, mais le chemin de traitement est différent).

3. **Invariants à recalibrer** :
   - Supprimer les invariants sur `reclassified` (anciens 2b, 3, 4, 6b, 7b, 8).
   - Conserver les invariants sur `converted` et `consolidated`.
   - Vérifier F99 = Σ(constituants) à chaque niveau.
   - Vérifier que E (équivalence) est exclue du consolidé.

4. **Valeurs golden recalculées** (à vérifier runtime) :
   - Les montants M, G, P, E au consolidated ne changent pas (le pipeline 3 niveaux produit les mêmes résultats que l'ancien pipeline 4 niveaux pour ces entités, car la reclassification n'affectait que S).
   - Le bloc S est **neutralisé** en attente des règles de périmètre.
   - Les natures staging `2MAN` : le montant 200 EUR passe maintenant par la conversion (EUR → EUR, pas de FX) → reste 200. **Mais** si le dataset est en devise étrangère, le taux de conversion s'applique.

**Approche recommandée** :
1. Lancer le serveur avec le seed golden.
2. Exécuter `POST /api/run`.
3. Lire les résultats via `GET /api/entries?consolidation=<id>&level=consolidated`.
4. Comparer aux valeurs attendues (adapter le dict Python).
5. Itérer jusqu'à ce que le golden passe.

### T6 — Mettre à jour la doc

Mettre à jour ce document (`docs/RECETTE_PYTHON.md`) avec :
- Les nouvelles options (`--seed-json` au lieu de `--csv-dir`).
- L'état de chaque script (vert / non vert).
- Les commandes de lancement exactes.

## 4. Récapitulatif des tâches

| Tâche | Dépend de | Difficulté | Livrable |
|---|---|---|---|
| T1 — Restaurer scripts | — | Faible | 3 fichiers `.py` |
| T2 — Générer seed JSON | T1 | Moyenne | 2 fichiers `seed_*.json` |
| T3 — Adapter smoke | T1, T2 | Faible | `smoke_test.py` vert |
| T4 — Adapter rules | T1, T2 | Faible | `rules_test.py` vert |
| T5 — Recalibrer golden | T1, T2 | **Moyenne** | `golden_test.py` vert |
| T6 — Doc | T3–T5 | Faible | `RECETTE_PYTHON.md` à jour |

## 5. Pré-requis

- Binaire `conso-server` compilé en release.
- `tests/fixtures/seed.json` existant (référence pour le format JSON).
- Comprendre le format `/api/export` (structure du paquet JSON).

---

## Pré-requis d'exécution (inchangés)

1. **Binaire** : depuis `prototype/rust/`, compiler le serveur en release :
   ```sh
   cargo build --release --bin conso-server
   ```
2. **Python 3.10+** (testé sur Cpython 3.11+).

## Lancement (après recalibration)

Depuis `prototype/rust/` :

```sh
python smoke_test.py --seed-json tests/fixtures/seed_smoke.json
python rules_test.py --seed-json tests/fixtures/seed_golden.json
python golden_test.py --seed-json tests/fixtures/seed_golden.json
```

**Code de sortie** : `0` = tout passe, `1` = au moins un échec (détail sur stdout).

## Anti-blocage (inchangé)

Ces scripts démarrent un serveur en avant-plan. Si tu utilises `--no-server`
pour debug, lance le serveur toi-même en arrière-plan (cf.
[`../AGENTS.md`](../AGENTS.md) §« Exécution et tests » — snippet PowerShell
`Start-Process -PassThru -RedirectStandardOutput`), puis :

```sh
python golden_test.py --no-server --port 3000 --seed-json tests/fixtures/seed_golden.json
```

N'oublie pas d'arrêter le serveur toi-même (`Stop-Process -Id $pid -Force`).
