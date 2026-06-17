#!/usr/bin/env python3
"""Smoke test HTTP pour conso-server.

Démarre le serveur, mitraille tous les endpoints, vérifie les réponses,
tue le serveur. Usage :

    python3 smoke_test.py [--port PORT] [--binary PATH]

Exit code 0 = tout passe, 1 = au moins un échec.
"""

import json
import os
import signal
import subprocess
import sys
import time
import argparse
from pathlib import Path

import urllib.request
import urllib.error

# ── Couleurs (désactivables) ─────────────────────────────────────────
COLOR = sys.stdout.isatty()

def green(s):  return f"\033[32m{s}\033[0m" if COLOR else s
def red(s):    return f"\033[31m{s}\033[0m" if COLOR else s
def dim(s):    return f"\033[2m{s}\033[0m" if COLOR else s
def bold(s):   return f"\033[1m{s}\033[0m" if COLOR else s

# ── Compteur de résultats ────────────────────────────────────────────
class Results:
    def __init__(self):
        self.passed = 0
        self.failed = 0
        self.failures = []

    def ok(self, name):
        self.passed += 1
        print(f"  {green('✓')} {name}")

    def ko(self, name, detail=""):
        self.failed += 1
        self.failures.append((name, detail))
        print(f"  {red('✗')} {name}")
        if detail:
            print(f"       {red(detail)}")

R = Results()

# ── Client HTTP minimaliste ──────────────────────────────────────────
BASE = "http://localhost"

def req(method, path, body=None, expect=None):
    """Fait un appel HTTP, retourne (status_code, json_body)."""
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    if data:
        r.add_header("Content-Type", "application/json")
    try:
        resp = urllib.request.urlopen(r)
        code = resp.getcode()
        raw = resp.read().decode()
    except urllib.error.HTTPError as e:
        code = e.code
        raw = e.read().decode()
    try:
        parsed = json.loads(raw) if raw else None
    except json.JSONDecodeError:
        parsed = raw
    if expect is not None and code != expect:
        raise AssertionError(f"HTTP {code} (attendu {expect}): {raw[:200]}")
    return code, parsed

# ── Helpers d'assertion ──────────────────────────────────────────────
def check(name, condition, detail=""):
    if condition:
        R.ok(name)
    else:
        R.ko(name, detail)

# ── Le scénario de test ──────────────────────────────────────────────
def run_tests():
    print(bold("\n═ conso-server — smoke test ═\n"))

    # ── 1. Health ────────────────────────────────────────────────────
    print(dim("1. Health"))
    code, body = req("GET", "/api/health", expect=200)
    check("health → 200", code == 200)
    check("health status=ok", isinstance(body, dict) and body.get("status") == "ok",
          f"body={body}")

    # ── 2. Reset → état propre ───────────────────────────────────────
    print(dim("\n2. Reset"))
    code, body = req("POST", "/api/reset", expect=200)
    check("reset → 200", code == 200)

    code, levels = req("GET", "/api/levels", expect=200)
    consolidated_before = next(
        (l["count"] for l in levels if l["level"] == "consolidated"), 0
    )
    check("reset vide consolidated", consolidated_before == 0,
          f"consolidated={consolidated_before}")

    # ── 3. Run pipeline ──────────────────────────────────────────────
    print(dim("\n3. Pipeline"))
    code, body = req("POST", "/api/run", expect=200)
    check("run → 200", code == 200)

    check("run génère corporate", isinstance(body, dict) and body.get("corporate", 0) > 0,
          f"body={body}")
    check("run génère reclassified", isinstance(body, dict) and body.get("reclassified", 0) > 0,
          f"body={body}")
    check("run génère converted", isinstance(body, dict) and body.get("converted", 0) > 0,
          f"body={body}")
    check("run génère consolidated", isinstance(body, dict) and body.get("consolidated", 0) > 0,
          f"body={body}")

    # Vérif cohérence : converted >= reclassified >= corporate
    c = body.get("corporate", 0)
    r = body.get("reclassified", 0)
    v = body.get("converted", 0)
    check("pipeline croissant (corp ≤ reclass ≤ conv)", c <= r <= v,
          f"{c} ≤ {r} ≤ {v}")

    # ── 4. Levels après run ──────────────────────────────────────────
    print(dim("\n4. Levels"))
    code, levels = req("GET", "/api/levels", expect=200)
    check("levels → 200", code == 200)
    levels_dict = {l["level"]: l["count"] for l in levels} if isinstance(levels, list) else {}
    check("levels contient 4 niveaux", len(levels_dict) == 4,
          f"levels={levels_dict}")

    # ── 5. Bilan ─────────────────────────────────────────────────────
    print(dim("\n5. Bilan"))
    code, bilan = req("GET", "/api/bilan?scenario=REEL&period=2024", expect=200)
    check("bilan → 200", code == 200)
    check("bilan non vide", isinstance(bilan, list) and len(bilan) > 0,
          f"bilan={bilan}")

    # Vérif qu'on a des flux F99 (clôture)
    flows_bilan = {row.get("flow") for row in bilan} if isinstance(bilan, list) else set()
    check("bilan contient F99 (clôture)", "F99" in flows_bilan,
          f"flows={flows_bilan}")

    # Vérif présence de F00 (ouverture)
    check("bilan contient F00 (ouverture)", "F00" in flows_bilan,
          f"flows={flows_bilan}")

    # ── 6. Compte de résultat ────────────────────────────────────────
    print(dim("\n6. Compte de résultat"))
    code, cr = req("GET", "/api/compte-resultat?scenario=REEL&period=2024", expect=200)
    check("CR → 200", code == 200)
    check("CR non vide", isinstance(cr, list) and len(cr) > 0,
          f"cr={cr}")

    flows_cr = {row.get("flow") for row in cr} if isinstance(cr, list) else set()
    check("CR contient F99 (clôture)", "F99" in flows_cr,
          f"flows={flows_cr}")

    # Le bilan et le CR exposent la nature dans leurs lignes
    if isinstance(bilan, list) and bilan:
        check("bilan expose la nature dans ses lignes",
              all("nature" in row and row["nature"] for row in bilan),
              f"sample={bilan[:1]}")
    if isinstance(cr, list) and cr:
        check("CR expose la nature dans ses lignes",
              all("nature" in row and row["nature"] for row in cr),
              f"sample={cr[:1]}")

    # Filtre par nature sur bilan et CR
    code, bilan_n = req("GET", "/api/bilan?scenario=REEL&period=2024&nature=0LIASS", expect=200)
    check("bilan nature=0LIASS → 200", code == 200)
    if isinstance(bilan_n, list) and bilan_n:
        check("bilan nature=0LIASS filtre correct",
              all(row.get("nature") == "0LIASS" for row in bilan_n),
              f"natures={ {row.get('nature') for row in bilan_n} }")

    code, cr_n = req("GET", "/api/compte-resultat?scenario=REEL&period=2024&nature=0LIASS", expect=200)
    check("CR nature=0LIASS → 200", code == 200)
    if isinstance(cr_n, list) and cr_n:
        check("CR nature=0LIASS filtre correct",
              all(row.get("nature") == "0LIASS" for row in cr_n),
              f"natures={ {row.get('nature') for row in cr_n} }")

    # ── 7. Entries avec filtres ──────────────────────────────────────
    print(dim("\n7. Entries (filtres)"))
    code, entries = req("GET", "/api/entries?level=consolidated&limit=10", expect=200)
    check("entries consolidated → 200", code == 200)
    check("entries retourne ≤ 10 lignes", isinstance(entries, list) and 0 < len(entries) <= 10,
          f"len={len(entries) if isinstance(entries, list) else 'N/A'}")

    # Vérif présence de la colonne nature
    if isinstance(entries, list) and entries:
        check("entries portent la colonne nature",
              all("nature" in e and e["nature"] for e in entries),
              f"natures={[e.get('nature') for e in entries[:3]]}")
        check("entries ont une nature non vide (0LIASS)",
              all(e.get("nature") == "0LIASS" for e in entries),
              f"natures={[e.get('nature') for e in entries[:3]]}")

    # Filtre par entité
    code, entries_e = req("GET", "/api/entries?entity=D&limit=5", expect=200)
    check("entries entity=D → 200", code == 200)
    if isinstance(entries_e, list) and entries_e:
        all_d = all(e.get("entity") == "D" for e in entries_e)
        check("entries entity=D filtre correct", all_d,
              f"entities={[e.get('entity') for e in entries_e]}")
    else:
        check("entries entity=D filtre correct", False, "réponse vide")

    # Filtre par nature
    code, entries_n = req("GET", "/api/entries?nature=0LIASS&limit=5", expect=200)
    check("entries nature=0LIASS → 200", code == 200)
    if isinstance(entries_n, list) and entries_n:
        all_n = all(e.get("nature") == "0LIASS" for e in entries_n)
        check("entries nature=0LIASS filtre correct", all_n,
              f"natures={[e.get('nature') for e in entries_n[:3]]}")
    else:
        check("entries nature=0LIASS filtre correct", False, "réponse vide")

    # Filtre par scénario
    code, entries_s = req("GET", "/api/entries?scenario=REEL&limit=5", expect=200)
    check("entries scenario=REEL → 200", code == 200)

    # ── 8. CRUD Master Data ──────────────────────────────────────────
    print(dim("\n8. CRUD Master Data"))

    # GET accounts
    code, accounts = req("GET", "/api/md/accounts", expect=200)
    check("GET accounts → 200", code == 200)
    check("accounts non vide", isinstance(accounts, list) and len(accounts) > 0,
          f"len={len(accounts) if isinstance(accounts, list) else 'N/A'}")

    # POST avec champ inconnu → 400
    code, _ = req("POST", "/api/md/accounts",
                  body={"code": "TST", "label": "mauvais champ"}, expect=400)
    check("POST champ inconnu → 400", code == 400)

    # POST valide
    code, created = req("POST", "/api/md/accounts",
                        body={"code": "TST", "libelle": "Compte test smoke",
                              "sous_classe": "8"}, expect=201)
    check("POST account TST → 201", code == 201)
    check("POST retourne la ligne",
          isinstance(created, dict) and created.get("code") == "TST",
          f"body={created}")

    # Conflict (même code)
    code, _ = req("POST", "/api/md/accounts",
                  body={"code": "TST", "libelle": "doublon"}, expect=409)
    check("POST doublon → 409", code == 409)

    # PUT update
    code, updated = req("PUT", "/api/md/accounts",
                        body={"code": "TST", "libelle": "Modifié"}, expect=200)
    check("PUT account TST → 200", code == 200)
    check("PUT modifie libelle",
          isinstance(updated, dict) and updated.get("libelle") == "Modifié",
          f"body={updated}")

    # PUT champ inconnu → 400
    code, _ = req("PUT", "/api/md/accounts",
                  body={"code": "TST", "label": "bad"}, expect=400)
    check("PUT champ inconnu → 400", code == 400)

    # DELETE sans PK → 400 avec message clair
    code, _ = req("DELETE", "/api/md/accounts", expect=400)
    check("DELETE sans PK → 400", code == 400)

    # DELETE via body JSON
    code, deleted = req("DELETE", "/api/md/accounts",
                        body={"code": "TST"}, expect=200)
    check("DELETE via body → 200", code == 200)
    check("DELETE retourne deleted=1",
          isinstance(deleted, dict) and deleted.get("deleted") == 1,
          f"body={deleted}")

    # Vérif suppression effective
    code, accounts_after = req("GET", "/api/md/accounts", expect=200)
    codes = {a["code"] for a in accounts_after} if isinstance(accounts_after, list) else set()
    check("account TST bien supprimé", "TST" not in codes)

    # ── 9. Autres tables master data ─────────────────────────────────
    print(dim("\n9. Master Data — autres tables"))
    for table in ["entities", "flows", "sous_classes", "currencies",
                  "scenarios", "periods", "natures"]:
        code, rows = req("GET", f"/api/md/{table}", expect=200)
        check(f"GET {table} → 200", code == 200,
              f"status={code}")

    # Vérif spécifique natures : 0LIASS et 1AJUST présents
    code, natures = req("GET", "/api/md/natures", expect=200)
    check("GET natures retourne une liste", isinstance(natures, list),
          f"body={natures}")
    if isinstance(natures, list):
        codes_n = {n.get("code") for n in natures}
        check("natures contient 0LIASS", "0LIASS" in codes_n,
              f"codes={codes_n}")
        check("natures contient 1AJUST", "1AJUST" in codes_n,
              f"codes={codes_n}")

    # ── 10. Cas d'erreur ─────────────────────────────────────────────
    print(dim("\n10. Cas d'erreur"))
    code, _ = req("GET", "/api/md/table_inexistante", expect=400)
    check("table inconnue → 400", code == 400)

    code, _ = req("DELETE", "/api/md/accounts?code=EXISTE_PAS", expect=404)
    check("DELETE inexistant → 404", code == 404)

# ── Démarrage / arrêt serveur ────────────────────────────────────────
def start_server(binary, port, csv_dir):
    env = os.environ.copy()
    env["CONSO_CSV_DIR"] = csv_dir
    proc = subprocess.Popen(
        [binary],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    # Health check loop
    global BASE
    BASE = f"http://localhost:{port}"
    for _ in range(40):
        time.sleep(0.25)
        if proc.poll() is not None:
            return None  # Process died
        try:
            urllib.request.urlopen(f"{BASE}/api/health", timeout=1)
            return proc
        except Exception:
            continue
    return None

def main():
    parser = argparse.ArgumentParser(description="Smoke test conso-server")
    parser.add_argument("--port", type=int, default=3000)
    parser.add_argument("--binary", default=None,
                        help="Chemin du binaire conso-server")
    parser.add_argument("--csv-dir", default="data",
                        help="Répertoire des CSV (defaut: data)")
    parser.add_argument("--no-server", action="store_true",
                        help="Ne pas démarrer de serveur (utilise --port)")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent
    binary = args.binary or str(repo_root / "target" / "release" / "conso-server")
    csv_dir = str(repo_root / args.csv_dir) if not Path(args.csv_dir).is_absolute() else args.csv_dir

    proc = None
    if not args.no_server:
        print(dim(f"Démarrage serveur : {binary}"))
        proc = start_server(binary, args.port, csv_dir)
        if proc is None:
            print(red("\n✗ Impossible de démarrer le serveur. Build manquant ?"))
            print(dim(f"  Lance : cargo build --release --bin conso-server"))
            sys.exit(1)
        print(dim(f"Serveur up sur :{args.port}\n"))

    try:
        run_tests()
    finally:
        if proc:
            proc.send_signal(signal.SIGTERM)
            proc.wait(timeout=5)
            print(dim("\nServeur arrêté."))

    # ── Bilan ────────────────────────────────────────────────────────
    total = R.passed + R.failed
    print(f"\n{'═' * 48}")
    if R.failed == 0:
        print(green(f"  ✓ {R.passed}/{total} tests passent — tout vert"))
    else:
        print(red(f"  ✗ {R.failed}/{total} échecs"))
        for name, detail in R.failures:
            print(f"    {red('•')} {name}: {detail}")
    print()

    sys.exit(1 if R.failed > 0 else 0)

if __name__ == "__main__":
    main()
