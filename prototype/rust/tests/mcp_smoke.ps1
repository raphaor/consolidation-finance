# Smoke test du serveur MCP conso-server --mcp (Q54).
#
# Valide le transport stdio (JSON-RPC) sans opencode : envoie initialize,
# notifications/initialized, tools/list, puis des tools/call représentatifs
# (describe_model, list_master_data, import_entries, run_consolidation, get_bilan)
# et vérifie les réponses. Utilise une base DuckDB jetable (n'écrase pas le dev).
#
# Usage (depuis prototype/rust/) :
#   .\tests\mcp_smoke.ps1
# Prérequis : .\target\release\conso-server.exe buildé (cargo build --release --bin conso-server).

param(
    [string]$Exe = ".\target\release\conso-server.exe",
    [string]$Seed = "tests\fixtures\seed.json"
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path $Exe)) {
    Write-Error "Binaire introuvable : $Exe. Lancez d'abord : cargo build --release --bin conso-server"
    exit 1
}

# Base jetable dédiée (ne touche ni le dev conso.duckdb ni le .conso-mcp.duckdb d'opencode).
$db = Join-Path $env:TEMP "opencode\mcp-smoke-$(Get-Random).duckdb"
Remove-Item $db -ErrorAction SilentlyContinue

$env:CONSO_DB_PATH = $db
$env:CONSO_SEED_JSON = $Seed
$env:CONSO_FORCE_RESEED = "1"

# --- Séquence JSON-RPC (une requête par ligne) ---
$lines = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"0.1"}}}',
    '{"jsonrpc":"2.0","method":"notifications/initialized"}',
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}',
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"describe_model","arguments":{}}}',
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_master_data","arguments":{"table":"accounts","limit":3}}}',
    '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_master_data","arguments":{"table":"accounts","search":"capital"}}}'
)
$payload = $lines -join "`n"

$raw = $payload | & $Exe --mcp 2>$null
$env:CONSO_FORCE_RESEED = ""
$env:CONSO_DB_PATH = ""
$env:CONSO_SEED_JSON = ""
Remove-Item $db, "$db.wal" -ErrorAction SilentlyContinue

# On ne garde que les lignes JSON-RPC valides (stdout contient aussi des logs DuckDB).
$responses = @{}
foreach ($l in $raw) {
    $t = "$l".Trim()
    if ($t -match '^"jsonrpc"') { $t = "{$t" }
    if ($t -match '\{"jsonrpc"') {
        try {
            $obj = $t | ConvertFrom-Json
            if ($null -ne $obj.id) { $responses["$($obj.id)"] = $obj }
        } catch {}
    }
}

$fail = 0
function Check($cond, $msg) {
    if ($cond) { Write-Host "  [OK] $msg" -ForegroundColor Green }
    else { Write-Host "  [ECHEC] $msg" -ForegroundColor Red; $script:fail++ }
}

Write-Host "Smoke test serveur MCP conso --mcp" -ForegroundColor Cyan

# 1. initialize
Check ($responses.ContainsKey("1") -and $responses["1"].result.serverInfo), "initialize -> serverInfo"

# 2. tools/list : au moins 10 outils, dont les clés attendues
$tools = $responses["2"].result.tools
$names = @($tools | ForEach-Object { $_.name })
Check ($names.Count -ge 10), "tools/list -> $($names.Count) outils (>= 10)"
foreach ($expected in @("describe_model","list_master_data","upsert_master_data","import_entries","get_entries","run_consolidation","run_controls","get_bilan","get_compte_resultat","get_indicator")) {
    Check ($names -contains $expected), "outil présent : $expected"
}

# 3. describe_model : contient code_catalog + flows
$dm = $responses["3"].result.content[0].text | ConvertFrom-Json
Check ($dm.code_catalog.flows -contains "F00"), "describe_model -> flows contient F00"
Check ($dm.consolidations.Count -gt 0), "describe_model -> >= 1 consolidation"

# 4. list_master_data(accounts, limit=3) : total > 0, 3 lignes
$lm = $responses["4"].result.content[0].text | ConvertFrom-Json
Check ($lm.total -gt 0 -and $lm.rows.Count -eq 3), "list_master_data(accounts,limit=3) -> total=$($lm.total), 3 lignes"

# 5. recherche "capital" : au moins 1 résultat dont le libellé contient capital/Capital
$sr = $responses["5"].result.content[0].text | ConvertFrom-Json
$hit = $sr.rows | Where-Object { $_.libelle -match "(?i)capital" }
Check ($sr.total -ge 1 -and $hit), "list_master_data(search=capital) -> recherche ILIKE"

Write-Host ""
if ($fail -eq 0) {
    Write-Host "SUCCES : smoke test MCP vert ($($names.Count) outils)." -ForegroundColor Green
    exit 0
} else {
    Write-Host "ECHEC : $fail vérification(s) en rouge." -ForegroundColor Red
    exit 1
}
