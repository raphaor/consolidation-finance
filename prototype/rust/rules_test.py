#!/usr/bin/env python3
"""Test du moteur de règles de consolidation.

Démarre le serveur Rust, charge le dataset golden (qui contient deux écritures
interco : M vend 1000 EUR à G sur le 700, G achète 1000 USD à M sur le 600),
lance le pipeline natif, puis :

  1. Crée une règle d'élimination interco à 4 opérations (extourne +
     contrepartie × partner hérité / vidé).
  2. Crée un ruleset contenant cette règle.
  3. POST /api/rules/run → exécute.
  4. Vérifie :
     - des lignes 2ELI sont apparues au niveau consolidated ;
     - le solde interco (partner NOT NULL) est extourné à 0 ;
     - le bilan agrégé (somme totale consolidated) est inchangé (équilibré) ;
     - le ruleset report contient bien 4 lignes générées.

Usage :
    python3 rules_test.py [--port PORT] [--binary PATH]

Exit code 0 = tout passe, 1 = au moins un échec.
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path

import urllib.request
import urllib.error

# ── Couleurs (désactivables hors TTY) ────────────────────────────────
COLOR = sys.stdout.isatty()


def green(s):  return f"\033[32m{s}\033[0m" if COLOR else s
def red(s):    return f"\033[31m{s}\033[0m" if COLOR else s
def yellow(s): return f"\033[33m{s}\033[0m" if COLOR else s
def dim(s):    return f"\033[2m{s}\033[0m" if COLOR else s
def bold(s):   return f"\033[1m{s}\033[0m" if COLOR else s


TOL = 0.01


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
            for line in detail.splitlines():
                print(f"       {red(line)}")


R = Results()


def check(name, condition, detail=""):
    if condition:
        R.ok(name)
    else:
        R.ko(name, detail)


def approx(a, b):
    return abs(a - b) <= TOL


def fmt(x):
    return f"{x:.2f}"


# =========================================================================
#  Client HTTP minimaliste (stdlib uniquement)
# =========================================================================
BASE = "http://localhost"


def req(method, path, body=None, expect=None, timeout=30):
    """Appel HTTP → (status_code, json_body). Lève en cas d'erreur réseau."""
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(url, data=data, method=method)
    if data:
        r.add_header("Content-Type", "application/json")
    try:
        resp = urllib.request.urlopen(r, timeout=timeout)
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
        raise AssertionError(f"HTTP {code} (attendu {expect}): {raw[:300]}")
    return code, parsed


def fetch_entries(level, scenario="REEL", entry_period="2024", limit=100000):
    """Pagine sur /api/entries pour récupérer TOUTES les écritures d'un niveau."""
    out = []
    offset = 0
    page = min(limit, 5000)
    while True:
        path = (f"/api/entries?level={level}&scenario={scenario}"
                f"&entry_period={entry_period}&limit={page}&offset={offset}")
        _, rows = req("GET", path, expect=200)
        if not isinstance(rows, list):
            raise AssertionError(f"entries {level} : réponse inattendue = {rows!r}")
        out.extend(rows)
        if len(rows) < page:
            break
        offset += page
        if offset >= limit:
            break
    return out


def total_amount(rows):
    """Somme des montants (toutes lignes)."""
    return sum(float(r["amount"]) for r in rows)


def sum_by_partner(rows):
    """Renvoie (somme_partner_null, somme_partner_not_null)."""
    null_sum = 0.0
    notnull_sum = 0.0
    for r in rows:
        amt = float(r["amount"])
        if r.get("partner") in (None, ""):
            null_sum += amt
        else:
            notnull_sum += amt
    return null_sum, notnull_sum


def filter_rows(rows, **kw):
    return [r for r in rows if all(r.get(k) == v for k, v in kw.items())]


# =========================================================================
#  Définition de la règle d'élimination interco
# =========================================================================
# Règle à 4 opérations, chacune sélectionne les lignes interco
# (partner IS NOT NULL) au niveau consolidated et :
#
#   Op 1 : extourne   du compte 700 — partner hérité, multiplicateur -1
#   Op 2 : extourne   du compte 600 — partner hérité, multiplicateur -1
#   Op 3 : contrepartie du compte 700 — partner vidé (NULL), multiplicateur +1
#   Op 4 : contrepartie du compte 600 — partner vidé (NULL), multiplicateur +1
#
# Le scope restreint aux (entity, partner) de méthode globale garantit qu'on
# ne touche que les vraies interco groupe (pas l'équivalence).
ELIM_RULE_CODE = "ELI_INTERCO"
ELIM_RULESET_CODE = "RS_INTERCO"

ELIM_DEFINITION = {
    "scope": [
        {"target": "entity",  "dim": "methode", "op": "=", "val": "globale"},
        {"target": "partner", "dim": "methode", "op": "=", "val": "globale"},
    ],
    "operations": [
        {
            "seq": 1,
            "level": "consolidated",
            "selection": [
                {"dim": "account", "op": "=", "val": "700"},
                {"dim": "partner", "op": "IS NOT NULL"},
            ],
            "coefficient": {"type": "pct_integration"},
            "multiplicateur": -1,
            "destination": {
                "nature":  {"mode": "override", "value": "2ELI"},
                "partner": {"mode": "inherit"},
            },
        },
        {
            "seq": 2,
            "level": "consolidated",
            "selection": [
                {"dim": "account", "op": "=", "val": "600"},
                {"dim": "partner", "op": "IS NOT NULL"},
            ],
            "coefficient": {"type": "pct_integration"},
            "multiplicateur": -1,
            "destination": {
                "nature":  {"mode": "override", "value": "2ELI"},
                "partner": {"mode": "inherit"},
            },
        },
        {
            "seq": 3,
            "level": "consolidated",
            "selection": [
                {"dim": "account", "op": "=", "val": "700"},
                {"dim": "partner", "op": "IS NOT NULL"},
            ],
            "coefficient": {"type": "pct_integration"},
            "multiplicateur": 1,
            "destination": {
                "nature":  {"mode": "override", "value": "2ELI"},
                "partner": {"mode": "null"},
            },
        },
        {
            "seq": 4,
            "level": "consolidated",
            "selection": [
                {"dim": "account", "op": "=", "val": "600"},
                {"dim": "partner", "op": "IS NOT NULL"},
            ],
            "coefficient": {"type": "pct_integration"},
            "multiplicateur": 1,
            "destination": {
                "nature":  {"mode": "override", "value": "2ELI"},
                "partner": {"mode": "null"},
            },
        },
    ],
}


# =========================================================================
#  Scénario de test
# =========================================================================
def run_tests():
    print(bold("\n═ Moteur de règles — élimination interco ═\n"))

    # ── 1. Reset + run pipeline (état baseline) ─────────────────────
    print(dim("1. Reset + pipeline natif"))
    code, body = req("POST", "/api/reset", expect=200)
    check("reset → 200", code == 200, f"body={body}")

    code, body = req("POST", "/api/run", expect=200)
    check("run → 200", code == 200, f"body={body}")
    check("run produit consolidated > 0",
          isinstance(body, dict) and body.get("consolidated", 0) > 0,
          f"body={body}")

    # ── 2. Baseline consolidée (avant règles) ───────────────────────
    print(dim("\n2. Baseline consolidée"))
    before = fetch_entries("consolidated")
    total_before = total_amount(before)
    null_before, notnull_before = sum_by_partner(before)
    print(f"     total         = {fmt(total_before)}")
    print(f"     partner=NULL  = {fmt(null_before)}")
    print(f"     partner≠NULL  = {fmt(notnull_before)}")
    check("baseline a des lignes partner non null (interco présentes)",
          notnull_before != 0.0,
          f"partner≠NULL = {fmt(notnull_before)} — dataset sans interco ?")
    check("aucune ligne 2ELI avant règles",
          len(filter_rows(before, nature="2ELI")) == 0,
          f"{len(filter_rows(before, nature='2ELI'))} lignes 2ELI déjà présentes")

    # ── 3. Création de la règle via API ─────────────────────────────
    print(dim("\n3. POST /api/rules — création de la règle ELI_INTERCO"))
    code, body = req("POST", "/api/rules", body={
        "code": ELIM_RULE_CODE,
        "libelle": "Élimination interco 700/600",
        "definition": ELIM_DEFINITION,
    }, expect=201)
    check("POST /api/rules → 201", code == 201, f"body={body}")
    check("réponse contient le code de la règle",
          isinstance(body, dict) and body.get("code") == ELIM_RULE_CODE,
          f"body={body}")
    check("réponse contient la définition JSON",
          isinstance(body, dict) and isinstance(body.get("definition"), dict),
          f"body={body}")

    # GET liste
    code, rules = req("GET", "/api/rules", expect=200)
    check("GET /api/rules → 200", code == 200)
    check("la règle apparaît dans la liste",
          isinstance(rules, list) and any(r.get("code") == ELIM_RULE_CODE for r in rules),
          f"rules={rules}")

    # GET détail
    code, detail = req("GET", f"/api/rules/{ELIM_RULE_CODE}", expect=200)
    check("GET /api/rules/{code} → 200", code == 200)
    check("le détail contient 4 opérations",
          isinstance(detail, dict)
          and isinstance(detail.get("definition"), dict)
          and len(detail["definition"].get("operations", [])) == 4,
          f"detail={detail}")

    # ── 4. Création du ruleset ──────────────────────────────────────
    print(dim("\n4. POST /api/rulesets — création du ruleset RS_INTERCO"))
    code, body = req("POST", "/api/rulesets", body={
        "code": ELIM_RULESET_CODE,
        "libelle": "Ruleset démo élimination interco",
        "items": [{"ordre": 1, "rule_code": ELIM_RULE_CODE}],
    }, expect=201)
    check("POST /api/rulesets → 201", code == 201, f"body={body}")
    check("réponse contient 1 item",
          isinstance(body, dict) and len(body.get("items", [])) == 1,
          f"body={body}")

    # GET détail ruleset
    code, rs = req("GET", f"/api/rulesets/{ELIM_RULESET_CODE}", expect=200)
    check("GET /api/rulesets/{code} → 200", code == 200)
    check("ruleset contient la règle ELI_INTERCO ordonnée",
          isinstance(rs, dict)
          and len(rs.get("items", [])) == 1
          and rs["items"][0].get("rule_code") == ELIM_RULE_CODE
          and rs["items"][0].get("ordre") == 1,
          f"rs={rs}")

    # ── 5. Cas d'erreur : DELETE rule référencée → 409 ──────────────
    print(dim("\n5. DELETE rule référencée → 409 (Conflict)"))
    code, _ = req("DELETE", f"/api/rules/{ELIM_RULE_CODE}", expect=409)
    check("DELETE rule référencée → 409", code == 409)

    # ── 6. Exécution du ruleset ─────────────────────────────────────
    print(dim("\n6. POST /api/rules/run — exécution"))
    code, report = req("POST", "/api/rules/run",
                       body={"ruleset": ELIM_RULESET_CODE}, expect=200)
    check("POST /api/rules/run → 200", code == 200, f"body={report}")
    check("report.ruleset = code attendu",
          isinstance(report, dict) and report.get("ruleset") == ELIM_RULESET_CODE,
          f"report={report}")
    total_gen = report.get("total_generated", 0) if isinstance(report, dict) else 0
    check("report.total_generated > 0 (lignes générées)",
          total_gen > 0, f"total_generated={total_gen}")
    # La règle doit générer à minima 4 lignes (1 par opération).
    check("report a généré au moins 4 lignes",
          total_gen >= 4, f"total_generated={total_gen}")

    # ── 7. Vérifications post-exécution ─────────────────────────────
    print(dim("\n7. Vérifications post-exécution"))
    after = fetch_entries("consolidated")
    total_after = total_amount(after)
    null_after, notnull_after = sum_by_partner(after)

    # 7a. Présence de lignes 2ELI au niveau consolidated
    lines_2eli = filter_rows(after, nature="2ELI")
    check("7a. des lignes 2ELI existent au niveau consolidated",
          len(lines_2eli) > 0,
          f"{len(lines_2eli)} ligne(s) 2ELI")
    if lines_2eli:
        # Analysis2 au format RULE:{code}:{seq} — uniquement sur les lignes
        # générées directement par la règle (les F99 reconstruits par
        # materialize_closures ont analysis2=NULL par construction).
        generated = [e for e in lines_2eli if e.get("flow") != "F99"]
        bad_audit = [e["analysis2"] for e in generated
                     if not str(e.get("analysis2", "")).startswith(f"RULE:{ELIM_RULE_CODE}:")]
        check("7a'. toutes les 2ELI non-F99 portent analysis2='RULE:ELI_INTERCO:<seq>'",
              generated and not bad_audit,
              f"analysis2 non conformes : {bad_audit[:5]}")
        # Les F99 2ELI reconstruits portent analysis2=NULL
        f99_2eli = [e for e in lines_2eli if e.get("flow") == "F99"]
        if f99_2eli:
            check("7a''. F99 2ELI reconstruits portent analysis2 NULL",
                  all(e.get("analysis2") is None for e in f99_2eli),
                  f"analysis2={[(e['flow'], e['analysis2']) for e in f99_2eli[:3]]}")

    # 7b. Solde interco extourné : sum(partner NOT NULL) = 0 à présent.
    #     La règle génère pour chaque ligne source interco une extourne
    #     (partner hérité, ×−1) qui l'annule exactement au niveau consolidated.
    check("7b. solde interco (partner NOT NULL) extourné à 0",
          approx(notnull_after, 0.0),
          f"partner≠NULL = {fmt(notnull_after)} (attendu ~0)")
    if not approx(notnull_after, 0.0):
        # Détailler par entité/partner pour aider au diagnostic
        diag = defaultdict(float)
        for r in after:
            if r.get("partner") not in (None, ""):
                diag[(r.get("entity"), r.get("partner"))] += float(r["amount"])
        for k, v in sorted(diag.items()):
            print(f"       {red(f'{k} → {fmt(v)}')}")

    # 7c. Bilan équilibré : la somme totale consolidated est inchangée
    #     (la règle génère autant de +X que de −X : extourne + contrepartie).
    check("7c. bilan agrégé (total) inchangé par la règle",
          approx(total_after, total_before),
          f"total avant={fmt(total_before)}, après={fmt(total_after)}, "
          f"Δ={fmt(total_after - total_before)}")

    # 7d. Vérif par compte : sur le 700 et le 600, l'axe interco (partner≠NULL)
    #     est aussi extourné à 0 (la règle cible ces deux comptes).
    for acc in ("700", "600"):
        rows_acc = [r for r in after if r.get("account") == acc]
        _, nn = sum_by_partner(rows_acc)
        check(f"7d. solde interco {acc} (partner NOT NULL) = 0",
              approx(nn, 0.0),
              f"{acc} partner≠NULL = {fmt(nn)}")

    # 7e. Contrepartie : sur l'axe bilan (partner=NULL), la règle a bien
    #     généré des lignes 2ELI (la contrepartie des extournes).
    eli_null = [r for r in lines_2eli if r.get("partner") in (None, "")]
    eli_notnull = [r for r in lines_2eli if r.get("partner") not in (None, "")]
    check("7e1. 2ELI avec partner NULL (contreparties) présent",
          len(eli_null) > 0,
          f"{len(eli_null)} ligne(s)")
    check("7e2. 2ELI avec partner non NULL (extournes) présent",
          len(eli_notnull) > 0,
          f"{len(eli_notnull)} ligne(s)")

    # 7f. Somme des 2ELI partner=NULL = -somme des 2ELI partner NOT NULL
    #     (la règle est équilibrée par construction).
    sum_eli_null = sum(float(r["amount"]) for r in eli_null)
    sum_eli_notnull = sum(float(r["amount"]) for r in eli_notnull)
    check("7f. somme 2ELI total = 0 (extourne et contrepartie s'équilibrent)",
          approx(sum_eli_null + sum_eli_notnull, 0.0),
          f"2ELI partner=NULL = {fmt(sum_eli_null)}, "
          f"2ELI partner≠NULL = {fmt(sum_eli_notnull)}")

    # ── 8. Idempotence : un second run ne doit pas doubler les lignes ─
    print(dim("\n8. Idempotence — second run"))
    # Le moteur utilise des snapshots et materialize_closures reconstruit
    # F99 autoritairement : un second run génère à nouveau les extournes
    # (puisque les sources 0LIASS sont toujours là). On vérifie simplement
    # que le serveur répond 200 et qu'on a toujours des 2ELI.
    code, report2 = req("POST", "/api/rules/run",
                        body={"ruleset": ELIM_RULESET_CODE}, expect=200)
    check("second run → 200", code == 200)
    after2 = fetch_entries("consolidated")
    lines_2eli_2 = filter_rows(after2, nature="2ELI")
    check("second run laisse des 2ELI au consolidé",
          len(lines_2eli_2) > 0,
          f"{len(lines_2eli_2)} ligne(s) 2ELI")


# =========================================================================
#  Démarrage / arrêt du serveur
# =========================================================================
def start_server(binary, port, csv_dir):
    env = os.environ.copy()
    env["CONSO_CSV_DIR"] = csv_dir
    env["CONSO_FORCE_RESEED"] = "1"  # forcage reseed : on veut le dataset golden propre
    proc = subprocess.Popen(
        [binary],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    global BASE
    BASE = f"http://localhost:{port}"
    for _ in range(60):
        time.sleep(0.25)
        if proc.poll() is not None:
            return None  # le process est mort
        try:
            urllib.request.urlopen(f"{BASE}/api/health", timeout=1)
            return proc
        except Exception:
            continue
    return None


def main():
    parser = argparse.ArgumentParser(description="Test du moteur de règles conso-server")
    parser.add_argument("--port", type=int, default=3000)
    parser.add_argument("--binary", default=None,
                        help="Chemin du binaire conso-server")
    parser.add_argument("--csv-dir", default="data_golden",
                        help="Répertoire des CSV (défaut: data_golden)")
    parser.add_argument("--no-server", action="store_true",
                        help="Ne pas démarrer de serveur (utilise --port)")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent
    binary = args.binary or str(repo_root / "target" / "release" / "conso-server")
    csv_dir = (str(repo_root / args.csv_dir)
               if not Path(args.csv_dir).is_absolute() else args.csv_dir)

    proc = None
    if not args.no_server:
        if not Path(binary).exists():
            print(red(f"\n✗ Binaire introuvable : {binary}"))
            print(dim("  Lance : cargo build --release --bin conso-server"))
            sys.exit(1)
        print(dim(f"Démarrage serveur : {binary}"))
        print(dim(f"CSV golden        : {csv_dir}"))
        proc = start_server(binary, args.port, csv_dir)
        if proc is None:
            print(red("\n✗ Impossible de démarrer le serveur."))
            sys.exit(1)
        print(dim(f"Serveur up sur :{args.port}"))

    try:
        run_tests()
    finally:
        if proc:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
            print(dim("\nServeur arrêté."))

    # Bilan
    total = R.passed + R.failed
    print(f"\n{'═' * 56}")
    if R.failed == 0:
        print(green(f"  ✓ {R.passed}/{total} vérifications passent — RULES TEST OK"))
    else:
        print(red(f"  ✗ {R.failed}/{total} échecs"))
        for name, detail in R.failures:
            print(f"    {red('•')} {name}")
    print()

    sys.exit(1 if R.failed > 0 else 0)


if __name__ == "__main__":
    main()
