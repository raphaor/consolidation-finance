# Plan Q54 — Accessibilité API pour agents IA (REST + MCP)

> Plan de réalisation de la [Q54](./QUESTIONS_OUVERTES.md#q54--accessibilité-api-pour-agents-ia-mcp--opérations-en-masse).
> Document **vivant** tant que Q54 n'est pas livré ; sera déplacé vers
> `archive/specs-livrees/` une fois implémenté (cf. [`README.md`](./README.md)).

## 1. Contexte & objectifs

Rendre l'application **pilotable par des agents IA** (opencode en premier lieu).
Deux axes complémentaires :

1. **Améliorer l'API REST existante** pour les cas d'usage agent (bulk master
   data, pagination, recherche, filtres, réponses enrichies).
2. **Encapsuler l'API en un serveur MCP** (Model Context Protocol) exposant des
   outils nommés et typés, découvrables par un LLM.

L'agent consommateur naturel est **opencode lui-même**, qui pilote déjà ce
projet. La friction actuelle (lancer `conso-server` est risqué : timeout
inopérant, cleanup manuel, interdit aux subagents — cf. `AGENTS.md` §« Exécution
et tests ») est précisément ce que le MCP local stdio élimine : opencode spawn
le process à la demande, pas de port, pas de cleanup à gérer par le LLM.

## 2. Décisions prises

| # | Décision | Choix |
|---|---|---|
| D1 | Périmètre Phase 1 (REST) | Les **6 améliorations** (pagination, recherche, filtres, bulk upsert, bulk delete, enrichissement) |
| D2 | Architecture MCP | **MCP intégré au serveur** via mode `--mcp` (stdio) — pas de binaire séparé, pas de route HTTP `/mcp` dans ce sprint |
| D3 | Surface MCP | **Sous-ensemble curaté** (10 outils), focus saisie / run conso / contrôles / rapports (bilan & P&L) |
| D4 | SDK protocole MCP | **`rmcp`** (SDK officiel Rust de `modelcontextprotocol/rust-sdk`) |
| D5 | Rétrocompat pagination | `GET /api/md/{table}` sans paramètre renvoie l'**array plat** actuel (frontend préservé) ; le format enveloppé `{total, rows}` n'est activé qu'en présence d'un paramètre de pagination (`?limit=`) ou `?envelope=true`. Le frontend pourra évoluer plus tard à la marge. |
| D6 | `describe_model` | **Outil de premier appel** conçu pour être réellement utile (cf. §4.3) — pas un simple miroir SQL |

Décisions reportées dans le registre : Q54 passera à `TRANCHÉE` en fin de sprint
(cf. §7).

## 3. Phase 1 — Améliorations API REST

**Fichier principal** : `prototype/rust/src/masterdata.rs` (routes via
`router()` à la ligne 1650). Pattern de référence pour le bulk :
`entries::create_entries` (`entries.rs:300` — `validate → BEGIN → loop INSERT →
COMMIT`).

### 3.1 Pagination — `GET /api/md/{table}?limit=N&offset=N`

- Clause SQL `LIMIT ? OFFSET ?` ajoutée à `select_all` (masterdata.rs:665).
- Un `COUNT(*)` séparé sur la même requête filtreée fournit le total.
- **Réponse enveloppée** `{ "total": N, "limit": L, "offset": O, "rows": [...] }`
  activée seulement si `?limit=` est présent (ou `?envelope=true`), sinon array
  plat (D5 — rétrocompat frontend).
- Réutilise le pattern `EntriesQuery` / `default_limit()` (server.rs:150-186).

### 3.2 Recherche — `?search=texte`

- `ILIKE '%texte%'` sur la colonne `libelle` (insensible à la casse + accents).
- **Ignoré** pour les 3 tables satellites sans `libelle` : `flow_scheme_items`,
  `perimeter`, `rates` (pas d'erreur, juste no-op + warning dans la réponse).
- Multi-mots : `?search=capital+social` → `AND libelle ILIKE '%capital%' AND
  libelle ILIKE '%social%'`.

### 3.3 Filtres — `?{col}=valeur`

- `WHERE` dynamique : tout paramètre de query dont le nom correspond à une
  colonne réelle de la table (validé contre `OwnedTableDef.columns`) est ajouté
  au WHERE.
- Colonnes inconnues → `400 Bad Request` (liste les colonnes valides dans le
  message, pour guider l'agent).
- Les PK composites (`flow_scheme_items`, `perimeter`, `rates`) sont filtrables
  colonne par colonne.

### 3.4 Bulk upsert — `PUT /api/md/{table}/bulk`

- Body : array JSON d'objets (chacun porte ses colonnes, **PK incluse**).
- Logique : pour chaque ligne, déterminer insert-vs-update par existence de la
  PK (`fetch_one`), puis `INSERT` ou `UPDATE`. Tout dans une transaction
  `BEGIN`/`COMMIT` (clone du pattern `create_entries`).
- **Validation préalable** de tout le batch : `reject_unknown_fields` +
  `validate_references` (FK) par ligne, agrégation des erreurs avant toute
  écriture (all-or-nothing, comme `validate_entry_rows`).
- Gère : PK `code`, `code_iso` (devises), `id` auto (consolidations — insert
  seul, pas d'update sur PK auto), PK composites.
- Réponse : `{ "inserted": N, "updated": M, "errors": [...] }` (errors vide si
  succès ; sinon 400 + erreurs détaillées).

### 3.5 Bulk delete — `DELETE /api/md/{table}/bulk`

- Body : array d'objets PK (multi-colonnes OK, ex. `{perimeter_set, entity,
  period}`).
- `BEGIN` → boucle `DELETE FROM ... WHERE pk=?` → `COMMIT`.
- Pré-check d'existence (404 agrégé si une PK n'existe pas, ou suppression
  best-effort avec compte — à trancher à l'implé, penche pour best-effort +
  compte dans la réponse).
- Réponse : `{ "deleted": N, "missing": M }`.

### 3.6 Réponses enrichies — `?enrich=true`

- Les FK de la réponse sont accompagnées de leur libellé via JOIN :
  `"account": "101"` → `"account": {"code": "101", "libelle": "Capital"}`.
- Opt-in (`?enrich=true`) pour préserver la rétrocompat.
- Réutilise `references::dimension_master` (references.rs:241) pour savoir
  quelles colonnes sont des FK et vers quelle table.
- Moyenne difficulté : JOINs dynamiques construits comme dans `get_entries`
  (server.rs:519-533).

### 3.7 Tests Phase 1

- Tests unitaires Rust (`tests/` ou `src/masterdata.rs` `#[cfg(test)]`) :
  pagination (total correct), recherche ILIKE (insensible casse), filtres
  (colonne inconnue → erreur), bulk upsert (insert+update mixés, rollback sur
  erreur), bulk delete (PK composite).
- `cargo test --release` doit passer.
- Vérifier que le frontend React (`web/`) n'est pas cassé : `npm run build`
  (les appels `GET /api/md/{table}` sans paramètres renvoient toujours un array
  plat).

## 4. Phase 2 — Serveur MCP intégré (mode `--mcp`)

### 4.1 Architecture

- **Branche dans `server.rs`** : après construction de `AppState`
  (server.rs:1732-1736), avant le bloc Axum (server.rs:1740-1809).
  - Whitelister `--mcp` dans `validate_args` (server.rs:1847).
  - Si `args.iter().any(|a| a == "--mcp")` → appeler `mcp::run_stdio(state).await`
    puis `return` (skip TcpListener, CORS, ServeDir, `axum::serve`).
- **Tout le setup DB partagé** (ouverture `Connection::open`, migrations,
  `CONSO_SEED_JSON`, CHECKPOINT) est commun aux deux modes — le MCP bénéficie du
  même `Arc<AppState>` que le serveur HTTP.
- **Transport** : stdio (JSON-RPC 2.0 over stdin/stdout), géré par `rmcp`.
- Les outils MCP appellent les **fonctions Rust `pub` de `conso-engine`**
  directement sur la `Connection` partagée — **aucun round-trip HTTP**.

### 4.2 Nouveau module `prototype/rust/src/mcp.rs`

- Déclare les outils via macros `rmcp` (descriptions + JSON Schema des
  paramètres).
- `pub async fn run_stdio(state: Arc<AppState>)` : lance le serveur MCP sur
  stdin/stdout.
- Chaque outil : wrapper fin qui `lock_con(&state)?`, appelle la fonction
  métier, sérialise le résultat en JSON.

### 4.3 Outils MCP (curated, 10 outils)

Focus user : **saisie / run conso / contrôles / rapports**.

| # | Outil | Source Rust | Paramètres | Retour |
|---|---|---|---|---|
| 1 | `describe_model` | `masterdata::TABLES` + `references` + `dimensions` | (aucun) | doc structuré (cf. §4.3.1) |
| 2 | `list_master_data` | `masterdata` (Phase 1) | `table, search?, filters?, limit?, offset?, enrich?` | `{total, rows}` |
| 3 | `upsert_master_data` | bulk Phase 1 | `table, rows[]` | `{inserted, updated, errors}` |
| 4 | `import_entries` | **refactor** `import.rs:140` → `pub fn import_entries_csv` | `rows[]` (JSON) **ou** `csv` (string) | `{imported, ids}` |
| 5 | `get_entries` | **refactor** `get_entries` (server.rs:444) → `pub fn` | `level?, consolidation_id?, entity?, phase?, source?, limit?` | `{total, rows}` |
| 6 | `run_consolidation` | **refactor** `run_pipeline_handler` (server.rs:655) → `pub fn` | `consolidation_id?` | `{counts, consolidation, ruleset?, ruleset_report?, a_nouveau_warnings}` |
| 7 | `run_controls` | `controls::run_control_set` (controls.rs:1045, déjà `pub`) | `set_code?, consolidation_id?, phase?, entry_period?` | `ControlSetReport` |
| 8 | `get_bilan` | **refactor** `get_bilan` (server.rs:314) → `pub fn` | `consolidation_id?, entity?, entry_period?, period?, nature?` | `[{account, flow, nature, sens, amount}]` |
| 9 | `get_compte_resultat` | **refactor** `get_compte_resultat` (server.rs:376) → `pub fn` | idem bilan | idem structure |
| 10 | `get_indicator` | **refactor** : pubifier `indicators::run_indicator` (indicators.rs:512) | `code?` ou `expression, consolidation_id, grain?` | `{ok, error?, sql?, rows}` |

#### 4.3.1 `describe_model` — conçu pour être utile (D6)

L'agent appelle cet outil **en premier** pour comprendre le modèle avant de
saisir ou requêter. Ce n'est pas un simple `SELECT * FROM information_schema`.
Retourne un JSON structuré compact :

```jsonc
{
  "pipeline_levels": ["raw(stg_entry)", "corporate", "converted", "consolidated"],
  "entry_schema": {
    // Les 15 champs de stg_entry, dans l'ordre, avec type + optionnalité + FK cible
    "fields": [
      {"name": "Phase", "type": "TEXT", "required": true, "fk": "dim_scenario_category.code"},
      {"name": "Entity", "type": "TEXT", "required": true, "fk": "dim_entity.code"},
      // ... Account, Flow, Currency, Nature, Amount, etc.
    ]
  },
  "master_tables": [
    // Pour chaque table master data navigable
    {"api_name": "accounts", "label": "Comptes", "sql_name": "dim_account",
     "pk": ["code"], "search_label_col": "libelle",
     "columns": [{"name": "code", "type": "VARCHAR"}, {"name": "libelle", ...}, ...]}
    // ... 20 tables
  ],
  "code_catalog": {
    // Échantillon de codes valides pour les dimensions structurantes
    // (ce que l'agent doit connaître pour ne pas se faire rejeter à la saisie)
    "flows": ["F00", "F20", "F80", "F81", "F01", "F98", "F99", ...],
    "natures": ["reel", ...],
    "account_classes": ["bilan", "resultat"],
    "scenario_categories": ["reel", ...],
    "methods": ["globale", "proportionnelle", "equivalence"],
    "currencies": ["EUR", "USD", ...],   // code_iso
    // entity/period codes : échantillon (les 10 premiers) pour ne pas exploser
    "entities_sample": ["ORBS", "MAGL", ...],
    "periods_sample": ["2025-12", "2026-06", ...]
  },
  "consolidations": [
    // Liste des consolidations existantes (id, libelle, statut) — ce que
    // run_consolidation attend comme consolidation_id
    {"id": 1, "libelle": "Clôture 2026", "statut": "ouvert"}
  ]
}
```

**Justification** : sans cet outil, l'agent devrait appeler `list_master_data`
sur 6 tables différentes + deviner les codes de flux/natures à partir de la
doc `FLUX_CONSO.md` (qu'il n'a pas en contexte par défaut). Un seul appel
`describe_model` lui donne tout ce qu'il faut pour construire un
`import_entries` valide ou un `run_consolidation` ciblé.

### 4.4 Refactors nécessaires (facteur `pub fn` partagé HTTP/MCP)

Pour éviter la duplication MCP ↔ HTTP, extraire le cœur métier des handlers
Axum en fonctions pures prenant `&Connection` (+ params). Les handlers HTTP
deviennent des wrappers fins (extract → appeler `pub fn` → JSON).

| Fichier | Extraction | Nouvelle `pub fn` |
|---|---|---|
| `import.rs` | cœur de `import_entries` handler (multipart → CSV bytes → `read_csv_auto`) | `pub fn import_entries_csv(con, csv_bytes: &[u8]) -> Result<usize, AppError>` |
| `indicators.rs` | promouvoir `run_indicator` / `compile_indicator` | `pub fn run_indicator(...)` |
| `server.rs` | cœur de `run_pipeline_handler` | `pub fn run_consolidation(con, consolidation_id) -> Result<PipelineResult, AppError>` (à ranger dans `lib.rs` ou un module `reports.rs`) |
| `server.rs` | cœur de `get_bilan` / `get_compte_resultat` | `pub fn get_bilan(con, &BilanQuery) -> Result<Vec<BilanRow>, AppError>` (+ variant P&L) |

Ces refactors ne changent **aucun** comportement HTTP ; les tests existants
garantissent la non-régression.

### 4.5 Dépendance `rmcp`

- Ajout à `prototype/rust/Cargo.toml` : `rmcp = { version = "...", features =
  ["server", "tokio"] }` (version exacte à vérifier à l'implémentation pour
  compatibilité tokio 1 / axum 0.8).
- **Risque** : API `rmcp` peut avoir bougé ; à valider par un `cargo build`
  tôt dans la Phase 2. Fallback si blocage : implémentation manuelle JSON-RPC
  stdio (~200 lignes de glue), mais `rmcp` reste l'option préférée (D4).

## 5. Client opencode (Windows + Linux)

Opencode lance le serveur MCP local via `type: "local"` + `command`. Le binaire
Rust `conso-server` (déjà existant, augmenté du flag `--mcp`) se compile pour
les deux OS.

### 5.1 Configuration type

Déposer dans le workspace `.opencode/opencode.jsonc` (ou config globale) :

**Windows** :
```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "conso": {
      "type": "local",
      "command": ["./prototype/rust/target/release/conso-server.exe", "--mcp"],
      "environment": {
        "CONSO_DB_PATH": "{env:CONSO_DB_PATH}",
        "CONSO_SEED_JSON": "{env:CONSO_SEED_JSON}"
      },
      "enabled": true,
      "timeout": 10000
    }
  }
}
```

**Linux** : identique, binaire sans `.exe` et chemins POSIX
(`./prototype/rust/target/release/conso-server`).

### 5.2 Build cross-platform

- `cargo build --release` dans `prototype/rust/` produit le binaire pour l'OS
  courant. Pour Linux depuis Windows : `cargo build --release --target
  x86_64-unknown-linux-gnu` (nécessite le toolchain ; ou build native sur la
  machine Linux cible).
- Variables d'env attendues : `CONSO_DB_PATH` (fichier `.duckdb`),
  `CONSO_SEED_JSON` (paquet JSON de seed, optionnel).

### 5.3 Guide d'usage

Un `docs/MCP.md` (à créer en Phase 2) documentera :
- Comment builder + configurer opencode (Windows + Linux).
- La **contrainte DuckDB mono-processus** (§6) : UI XOR agent sur le même
  `.duckdb`.
- Exemples de prompts agent : « liste les entités », « saisis ces écritures
  puis lance la consolidation 2, puis donne-moi le bilan », « exécute le
  control-set CLO et résume les anomalies ».

## 6. Contraintes & risques

- **DuckDB mono-processus** : le mode `--mcp` (qui ouvre le `.duckdb` via
  `conso-engine`) et le serveur HTTP `conso-server` (sans `--mcp`) **ne peuvent
  pas** partager le même fichier simultanément. Règle d'usage : soit l'UI
  (serveur HTTP), soit l'agent (mode `--mcp`), pas les deux à la fois sur la
  même base. Pour l'usage UI+agent simultané, fallback futur = route HTTP
  `/mcp` (Streamable HTTP transport) — **hors scope de ce sprint**.
- **Workers/subagents** : ne lancent aucun serveur (règle `AGENTS.md`). Les
  tests runtime MCP (stdio) seront faits par l'utilisateur principal via
  `Start-Process` ou directement par opencode (qui gère le cycle de vie du
  child MCP).
- **Risque `rmcp`** : voir §4.5.
- **Rétrocompat frontend** : la pagination enveloppée est opt-in (D5) ; le
  frontend existant n'est pas touché. Un passage du frontend au format enveloppé
  pourra se faire plus tard à la marge.

## 7. Ordre d'exécution & suivi

1. **Phase 1 — REST** (§3) : 6 améliorations dans `masterdata.rs` + routes.
   - `cargo test --release` + `npm run build` (frontend préservé).
2. **Refactors d'extraction** (§4.4) : `pub fn` partagés HTTP/MCP.
   - `cargo test --release` (non-régression HTTP).
3. **Module `mcp.rs` + `--mcp` + `rmcp`** (§4.1, §4.2, §4.3).
   - Test stdio via harness (le serveur en `--mcp` lit un script JSON-RPC sur
     stdin et répond sur stdout — testable sans réseau).
4. **Configs opencode Windows + Linux** (§5) + `docs/MCP.md`.
5. **Clôture registre** : Q54 → `TRANCHÉE` dans `QUESTIONS_OUVERTES.md` (décision
   reportée dans l'EDB §post-MVP), `PLAN_Q54_API_MCP.md` → `archive/specs-livrees/`.

### Checklist

- [ ] Phase 1 : pagination (3.1)
- [ ] Phase 1 : recherche (3.2)
- [ ] Phase 1 : filtres (3.3)
- [ ] Phase 1 : bulk upsert (3.4)
- [ ] Phase 1 : bulk delete (3.5)
- [ ] Phase 1 : enrichissement (3.6)
- [ ] Phase 1 : tests + `npm run build`
- [ ] Refactors `pub fn` (import_entries_csv, run_consolidation, get_bilan/P&L, run_indicator)
- [ ] Dépendance `rmcp` + `cargo build` validé
- [ ] Module `mcp.rs` + 10 outils
- [ ] Flag `--mcp` dans `server.rs`
- [ ] Tests stdio MCP
- [ ] `docs/MCP.md`
- [ ] Configs opencode Windows + Linux
- [ ] Q54 TRANCHÉE + plan archivé
