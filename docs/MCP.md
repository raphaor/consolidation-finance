# Serveur MCP (Model Context Protocol)

> [Q54](./QUESTIONS_OUVERTES.md#q54--accessibilité-api-pour-agents-ia-mcp--opérations-en-masse)
> — accessibilité API pour agents IA. Voir aussi
> [`archive/specs-livrees/PLAN_Q54_API_MCP.md`](./archive/specs-livrees/PLAN_Q54_API_MCP.md) §5.

Le moteur de consolidation est pilotable par un agent IA (opencode, Claude,
Cursor…) via un **serveur MCP** intégré au binaire `conso-server`. L'agent
découvre des outils nommés et typés (description + JSON Schema des paramètres).
**Deux modes** coexistent :

- **stdio** (`conso-server --mcp`) : opencode spawn le process sur stdin/stdout,
  aucun serveur HTTP à lancer — idéal pour une session agent ad-hoc, mais
  process séparé → base DuckDB séparée (bac à sable) ou exclusive (verrou si
  même fichier que l'UI).
- **HTTP** (`/mcp` sur le serveur HTTP en écoute) : l'agent se connecte en MCP
  remote au serveur qui sert déjà l'UI → **même process, même base que l'UI,
  accès simultané, sans verrou ni duplication**. C'est le mode recommandé pour
  travailler sur les données réelles.

## Principe

### Mode stdio (`conso-server --mcp`)

```
Agent IA (opencode…)
    │ MCP (JSON-RPC sur stdio)
    ▼
conso-server --mcp   ← process séparé, flag --mcp
    │ appelle les fonctions Rust de conso-engine (aucun round-trip HTTP)
    ▼
conso.duckdb          ← base dédiée (.conso-mcp.duckdb) OU base réelle (mais
                        alors exclusive avec l'UI : DuckDB mono-processus)
```

### Mode HTTP (route `/mcp`) — UI + agent simultanés

```
opencode (MCP remote)        navigateur (UI React)
    │                             │
    │  http://localhost:3000/mcp   │  http://localhost:3000/api/...
    ▼                             ▼
   conso-server (UN SEUL process, port 3000)
        │  même Arc<AppState> → même connexion DuckDB
        ▼
     conso.duckdb  ← partagée : éditions UI et écritures agent visibles
                    des deux côtés, en temps réel, sans verrou
```

Les deux modes partagent le setup DB (schéma, migrations, seed JSON) et le cœur
métier : les 10 outils MCP appellent les mêmes fonctions Rust que les handlers
REST (`conso_engine::reports`, `masterdata`, `import`, `indicators`, `controls`).

## Outils exposés (10)

Focus : **saisie, run de consolidation, contrôles, rapports (bilan & P&L),
indicateurs**.

| Outil | Rôle | Params clés |
|---|---|---|
| `describe_model` | **Premier appel** : tables master data, champs de saisie, catalogue de codes (flux, natures, classes, devises, méthodes, phases), consolidations | — |
| `list_master_data` | Lecture paginée/recherchée/filtrée d'une table | `table, search?, filters?, limit?, offset?, enrich?` |
| `upsert_master_data` | Insert/update en masse (validation all-or-nothing + transaction) | `table, rows_json` |
| `import_entries` | Append d'écritures dans `stg_entry` (CSV ou JSON) | `csv? \| rows_json?` |
| `get_entries` | Lecture des écritures (raw / corporate / converted / consolidated) | `level?, consolidation_id?, entity?, phase?, …` |
| `run_consolidation` | Pipeline 3 étapes + ruleset + contrôle à-nouveau | `consolidation_id?` (défaut : 1ère « ouvert ») |
| `run_controls` | Exécute un control-set (ou liste les disponibles si `set_code` omis) | `set_code?, consolidation_id?, phase?, entry_period?` |
| `get_bilan` | Bilan par flux (classe `bilan`) | `consolidation_id?, entity?, period?, …` |
| `get_compte_resultat` | Compte de résultat par flux (classe `resultat`) | idem |
| `get_indicator` | Calcule un indicateur (code existant ou formule ad-hoc) | `code? \| expression, consolidation_id, grain?` |

## Build

```bash
# depuis prototype/rust/
cargo build --release --bin conso-server
```

Le binaire est `prototype/rust/target/release/conso-server` (`.exe` sous
Windows). La 1ʳᵉ compilation est lourde (DuckDB C++ embarqué + rmcp + schemars).

## Configuration d'opencode

Deux options selon le mode souhaité.

### Option A — MCP remote (HTTP, `/mcp`) : UI + agent simultanés sur la même base

C'est le mode **recommandé** pour travailler sur vos données réelles. Le
serveur HTTP doit tourner (celui qui sert déjà l'UI). Aucun chemin de binaire à
configurer (opencode se connecte par URL) :

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "conso": {
      "type": "remote",
      "url": "http://localhost:3000/mcp",
      "enabled": true,
      "timeout": 20000
    }
  }
}
```

Lancez `conso-server` (sans `--mcp`) comme d'habitude ; l'agent s'y branche.
L'UI et l'agent voient et modifient la même base en temps réel.

### Option B — MCP local (stdio, `--mcp`) : session agent ad-hoc, bac à sable

La config se met dans `.opencode/opencode.jsonc` (workspace). Un template
`.opencode/opencode.jsonc.example` est fourni (committed) ; la config réelle
(chemins absolus, machine-spécifique) est **gitignored**.

> **Piège de chemin (important)** : opencode résout le chemin du binaire dans
> `command` **depuis le répertoire d'où vous lancez opencode**, pas depuis la
> racine du workspace. Un chemin relatif (`prototype/rust/...`) cassera donc si
> vous lancez opencode depuis un sous-dossier (ex. `prototype/`). **Utilisez un
> chemin absolu** (ou une variable d'environnement `{env:CONSO_SERVER_EXE}`) pour
> le binaire — c'est la cause du `Connection closed` si vous l'avez rencontré.

> Activation : opencode charge les serveurs MCP **au démarrage de la session**.
> Pour prendre en compte la config, **relancez la session opencode** (ou
> redémarrez opencode). En nouvelle session, c'est automatique. Vérifiez avec
> `opencode mcp list` (statut du serveur `conso`) ; les outils apparaissent
> préfixés `conso_*`.

### Windows — `.opencode/opencode.jsonc` (chemins absolus)

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "conso": {
      "type": "local",
      "command": ["C:/.../consolidation-finance/prototype/rust/target/release/conso-server.exe", "--mcp"],
      "environment": {
        "CONSO_DB_PATH": "C:/.../consolidation-finance/.conso-mcp.duckdb",
        "CONSO_SEED_JSON": "C:/.../consolidation-finance/prototype/rust/tests/fixtures/seed.json"
      },
      "enabled": true,
      "timeout": 20000
    }
  }
}
```

(Forward slashes `/` acceptés sous Windows ; ou échappez les antislashs `\\`.)

### Portable (recommandé) — via variable d'environnement (Windows + Linux)

Définissez une fois dans votre shell (persistent) le chemin absolu du binaire,
puis référencez-le par `{env:}` — la config reste committable et fonctionne
depuis n'importe quel répertoire de lancement :

```bash
# Windows (PowerShell, persistent pour l'utilisateur)
setx CONSO_SERVER_EXE "C:/.../prototype/rust/target/release/conso-server.exe"
setx CONSO_DB_PATH "C:/.../consolidation-finance/.conso-mcp.duckdb"
setx CONSO_SEED_JSON "C:/.../consolidation-finance/prototype/rust/tests/fixtures/seed.json"

# Linux (ajouter à ~/.bashrc ou ~/.zshrc)
export CONSO_SERVER_EXE="$PWD/prototype/rust/target/release/conso-server"
export CONSO_DB_PATH="$PWD/.conso-mcp.duckdb"
export CONSO_SEED_JSON="$PWD/prototype/rust/tests/fixtures/seed.json"
```

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "conso": {
      "type": "local",
      "command": ["{env:CONSO_SERVER_EXE}", "--mcp"],
      "environment": {
        "CONSO_DB_PATH": "{env:CONSO_DB_PATH}",
        "CONSO_SEED_JSON": "{env:CONSO_SEED_JSON}"
      },
      "enabled": true,
      "timeout": 20000
    }
  }
}
```

(Sous Windows le binaire est `conso-server.exe`, sous Linux `conso-server`.)

Variables d'environnement attendues (à définir dans le shell qui lance
opencode) :
- `CONSO_DB_PATH` : fichier `.duckdb` (défaut `conso.duckdb`).
- `CONSO_SEED_JSON` : paquet JSON de seed (optionnel ; importé sur base
  vierge ou au `POST /api/reset`). Le fixture de test est
  `prototype/rust/tests/fixtures/seed.json`.

### Build Linux depuis Windows

Le binaire Rust se compile par OS cible. Pour produire le binaire Linux :

```bash
# option A : build native sur la machine Linux cible (recommandé)
cargo build --release --bin conso-server

# option B : cross-compile depuis Windows (nécessite le toolchain ciblé)
rustup target add x86_64-unknown-linux-gnu
cargo build --release --bin conso-server --target x86_64-unknown-linux-gnu
```

## Contrainte DuckDB mono-processus ⚠️

DuckDB embarqué n'autorise qu'**un seul processus writer** sur un fichier
`.duckdb`. La contrainte s'applique selon le mode MCP choisi :

- **Mode HTTP `/mcp` (recommandé)** — **aucune contrainte** : l'agent et l'UI
  tournent dans le **même process** (`conso-server`), partagent la même
  connexion DuckDB. UI et agent sont simultanés sur la même base réelle.
- **Mode stdio `--mcp`** — process **séparé** qui ouvre la base directement :
  il ne peut pas coexister avec une instance HTTP `conso-server` sur le **même
  fichier**. Deux options :
  - bac à sable : `CONSO_DB_PATH` pointe sur une base **distincte**
    (`.conso-mcp.duckdb`) → pas de conflit, mais données séparées de l'UI ;
  - base réelle : `CONSO_DB_PATH` = la base de l'UI → alors **exclusive**
    (arrêtez l'UI avant de lancer une session MCP stdio, et inversement).

## Exemples de prompts agent

Une fois le MCP configuré (mode HTTP ou stdio), dans opencode :

```
use the conso tool to describe_model, then list the entities
```
```
use conso : importe ces écritures (coller le CSV) puis lance la consolidation 1,
puis donne-moi le bilan consolidé
```
```
use conso run_controls sans set_code pour lister les jeux, puis exécute CLO sur
la consolidation 1 et résume les anomalies
```

## Tester manuellement (sans opencode)

### Mode HTTP (`/mcp`) — sans client MCP, via `curl`/Invoke-WebRequest

Lancez `conso-server` (sans `--mcp`), puis :

```bash
# initialize
curl -sS -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}'

# tools/list
curl -sS -X POST http://localhost:3000/mcp \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

Réponses en JSON direct (mode stateless + `json_response`). L'UI REST
(`/api/...`) reste disponible en parallèle sur la même base.

### Mode stdio (`--mcp`)

Le serveur MCP parle JSON-RPC sur stdio :

```bash
# depuis prototype/rust/
CONSO_DB_PATH=/tmp/conso.duckdb CONSO_SEED_JSON=tests/fixtures/seed.json \
  ./target/release/conso-server --mcp
# puis saisir les requêtes JSON-RPC sur stdin, une par ligne :
# {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}
# {"jsonrpc":"2.0","method":"notifications/initialized"}
# {"jsonrpc":"2.0","id":2,"method":"tools/list"}
# {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"describe_model","arguments":{}}}
```

Les réponses (une par ligne sur stdout) contiennent `result.content[0].text`
(JSON sérialisé de l'outil).

## Recette automatisée (smoke test)

Un script PowerShell valide le serveur MCP via stdio **sans opencode** (CI-able) :

```bash
# depuis prototype/rust/
cargo build --release --bin conso-server   # prérequis
.\tests\mcp_smoke.ps1
```

Il envoie `initialize` + `tools/list` + 4 `tools/call` représentatifs
(`describe_model`, `list_master_data` paginé, recherche `?search`, etc.) sur une
base jetable, et asserte : ≥ 10 outils exposés, présences des outils clés,
`describe_model` renvoie le catalogue de flux, la pagination respecte `limit`,
la recherche `ILIKE` filtre. Sortie `[OK]`/`[ECHEC]` par vérification, exit 0/1.

### Recette via opencode (après activation)

Une fois le MCP chargé, demander à l'agent (prompts types) :

1. **Lecture** : « use conso : décris le modèle puis liste 5 comptes de classe
   bilan » → valide `describe_model` + `list_master_data` (filtre `classe`).
2. **Saisie + exécution** : « use conso : importe ces écritures (coller un CSV
   minimal) puis lance la consolidation 1 » → valide `import_entries` +
   `run_consolidation`.
3. **Rapports** : « use conso : donne le bilan consolidé et le compte de
   résultat de la consolidation 1 » → valide `get_bilan` + `get_compte_resultat`.
4. **Contrôles** : « use conso : liste les control-set puis exécute le premier
   sur la consolidation 1 » → valide `run_controls` (découverte + exécution).

Pour réinitialiser la base de test (repartir d'un seed propre) : supprimez
`.conso-mcp.duckdb` à la racine du workspace — il sera recréé au prochain
démarrage du MCP.
