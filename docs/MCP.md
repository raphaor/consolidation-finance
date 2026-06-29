# Serveur MCP (Model Context Protocol)

> [Q54](./QUESTIONS_OUVERTES.md#q54--accessibilité-api-pour-agents-ia-mcp--opérations-en-masse)
> — accessibilité API pour agents IA. Voir aussi
> [`archive/specs-livrees/PLAN_Q54_API_MCP.md`](./archive/specs-livrees/PLAN_Q54_API_MCP.md) §5.

Le moteur de consolidation est pilotable par un agent IA (opencode, Claude,
Cursor…) via un **serveur MCP** intégré au binaire `conso-server`. L'agent
découvre des outils nommés et typés (description + JSON Schema des paramètres)
et les invoque sur stdin/stdout — aucun serveur HTTP à lancer, aucun port à
gérer.

## Principe

```
Agent IA (opencode…)
    │ MCP (JSON-RPC sur stdio)
    ▼
conso-server --mcp   ← même binaire que le serveur HTTP, flag --mcp
    │ appelle les fonctions Rust de conso-engine (aucun round-trip HTTP)
    ▼
conso.duckdb
```

Le mode `--mcp` partage tout le setup DB (schéma, migrations, seed JSON) avec
le mode HTTP. Le cœur métier est commun : les outils MCP appellent les mêmes
fonctions que les handlers REST (`conso_engine::reports`, `masterdata`,
`import`, `indicators`, `controls`).

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

### Windows — `.opencode/opencode.jsonc` (workspace) ou `%APPDATA%\opencode\opencode.jsonc`

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
      "timeout": 15000
    }
  }
}
```

### Linux — `~/.config/opencode/opencode.jsonc` (ou `.opencode/opencode.jsonc` du workspace)

```jsonc
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "conso": {
      "type": "local",
      "command": ["./prototype/rust/target/release/conso-server", "--mcp"],
      "environment": {
        "CONSO_DB_PATH": "{env:CONSO_DB_PATH}",
        "CONSO_SEED_JSON": "{env:CONSO_SEED_JSON}"
      },
      "enabled": true,
      "timeout": 15000
    }
  }
}
```

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
`.duckdb`. Le mode `--mcp` ouvre la base directement (via `conso-engine`) :
**il ne peut pas coexister avec une instance HTTP `conso-server`** (sans
`--mcp`) sur le même fichier.

Règle d'usage :
- **Soit** l'UI (serveur HTTP, `conso-server` sans `--mcp`),
- **Soit** l'agent (mode `--mcp`), pas les deux à la fois sur la même base.

Pour un usage UI + agent simultané, évolution future = route HTTP `/mcp`
(Streamable HTTP transport), non couverte par ce sprint
([Q54](./QUESTIONS_OUVERTES.md) décision D2).

## Exemples de prompts agent

Une fois le MCP configuré, dans opencode :

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

Le serveur MCP parle JSON-RPC sur stdio. Pour le déboguer :

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
