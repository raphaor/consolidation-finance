#!/usr/bin/env python3
"""Golden master — test de non-régression du moteur de consolidation.

Démarre le serveur Rust, charge le dataset dédié (`data_golden/`), lance le
pipeline, et compare les résultats niveau par niveau aux valeurs attendues
calculées à la main.

Le dataset couvre 5 entités (M/G/P/E/S), 3 devises (EUR/USD/GBP), les 3 méthodes
de consolidation (globale/proportionnelle/équivalence), une sortie de périmètre
(S → F98 miroir + F99=0), une nature d'ajustement séparée (1AJUST), et les trois
niveaux d'injection staging (2MAN/3MAN/4MAN).

Usage :
    python3 golden_test.py [--port PORT] [--binary PATH]

Exit code 0 = tout passe, 1 = au moins un échec.
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import time
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


# ── Tolérance de comparaison (montants à 2 décimales) ────────────────
TOL = 0.01

# ── Grains d'intérêt : (entity, account, flow, nature) ───────────────
# Le grain complet d'agrégation consolidé. On somme `amount` sur ce grain.

# =========================================================================
#  ⚠️ VALEURS GOLDEN À RECALCULER (post-Q41 + suppression du niveau reclassified)
# =========================================================================
# Ces valeurs ont été calculées à la main pour l'ancien modèle **4 niveaux**
# (A→B→C→D avec `reclassified`) où la **sortie de périmètre** produisait F98 de
# façon NATIVE à l'étape B. Depuis :
#   - le niveau `reclassified` a été supprimé (pipeline 3 niveaux : corporate →
#     converted → consolidated) ;
#   - les variations de périmètre (entrée F01, sortie F98) sont **repensées en
#     règles** (tests natifs `#[ignore]`) → la sortie de S n'est plus produite
#     nativement : tout le bloc `S` ci-dessous et les invariants `reclassified`
#     (2b, 3, 4, 6b, 7b, 8) sont **obsolètes** ;
#   - le staging a changé de cible : préfixe `2` → converted (et non reclassified),
#     `3`/`4` → consolidated (cf. src/pipeline/staging.rs).
# → À recalculer contre le moteur 3-niveaux en marche (tâche runtime, recette E2E
#   §4 du REFACTOR_CONSO_RESTE_A_FAIRE). Tant que ce n'est pas fait, ce golden
#   master échouera volontairement sur S et les invariants reclassified.
# =========================================================================
# Chaque entrée : (entity, account, flow, nature) -> montant attendu.
# Calculées à la main à partir des taux et du pipeline (A→B→C→D, modèle pré-Q41).
#
# Taux :
#   USD : close_n1 = taux_ouverture(2024) = 0.92 | close_n=0.90 / avg=0.95 (2024)
#   GBP : close_n1 = taux_ouverture(2024) = 1.15 | close_n=1.12 / avg=1.18 (2024)
#   (taux_ouverture de N = clôture N-1, porté par N — résout close_n1 sans période antérieure)
#
# Méthodes : M/G/S = globale ×1.0 ; P = proportionnelle ×0.6 ; E = équivalence (exclue)
# =========================================================================
EXPECTED_CONSOLIDATED = {
    # ── M (EUR, globale ×1.0) — référence sans FX ──────────────────────
    ("M", "100", "F00", "0LIASS"): 10000,      # EUR, copie directe
    ("M", "100", "F20", "0LIASS"): 1000,
    ("M", "100", "F99", "0LIASS"): 11000,      # F00+F20 (EUR → pas d'écart)
    ("M", "200", "F00", "0LIASS"): 5000,
    ("M", "200", "F20", "0LIASS"): 500,
    ("M", "200", "F99", "0LIASS"): 5500,
    ("M", "700", "F20", "0LIASS"): 4000,       # 3000 + 1000 interco vers G
    ("M", "700", "F99", "0LIASS"): 4000,       # F20 reconstitué (inclut l'interco)

    # ── G (USD, globale ×1.0) — FX complète ────────────────────────────
    # F00→close_n1, écart F80=(close_n-close_n1), F20→avg, écart F81=(close_n-avg)
    ("G", "100", "F00", "0LIASS"): 7360,       # 8000×0.92
    ("G", "100", "F80", "0LIASS"): -160,       # 8000×(0.90-0.92)
    ("G", "100", "F20", "0LIASS"): 760,        # 800×0.95
    ("G", "100", "F81", "0LIASS"): -40,        # 800×(0.90-0.95)
    ("G", "100", "F99", "0LIASS"): 7920,       # 7360-160+760-40
    ("G", "200", "F00", "0LIASS"): 5520,       # 6000×0.92
    ("G", "200", "F80", "0LIASS"): -120,       # 6000×(0.90-0.92)
    ("G", "200", "F20", "0LIASS"): 380,        # 400×0.95
    ("G", "200", "F81", "0LIASS"): -20,        # 400×(0.90-0.95)
    ("G", "200", "F99", "0LIASS"): 5760,       # 5520-120+380-20
    ("G", "700", "F20", "0LIASS"): 1900,       # 2000×0.95
    ("G", "700", "F81", "0LIASS"): -100,       # 2000×(0.90-0.95)
    ("G", "700", "F99", "0LIASS"): 1800,       # 1900-100

    # ── G / 600 (USD, globale ×1.0) — achat interco depuis M ──────────
    # 1000 USD → 0.95 = 950 EUR ; écart F81 = 1000×(0.90-0.95) = -50
    ("G", "600", "F20", "0LIASS"): 950,        # 1000×0.95 (achat interco depuis M)
    ("G", "600", "F81", "0LIASS"): -50,        # 1000×(0.90-0.95)
    ("G", "600", "F99", "0LIASS"): 900,        # 950-50

    # ── P (USD, proportionnelle ×0.6) ──────────────────────────────────
    # Converti puis ×0.6 à la consolidation (sur TOUS les flux, écarts compris)
    ("P", "100", "F00", "0LIASS"): 2760,       # 5000×0.92×0.6
    ("P", "100", "F80", "0LIASS"): -60,        # 5000×(0.90-0.92)×0.6
    ("P", "100", "F20", "0LIASS"): 285,        # 500×0.95×0.6
    ("P", "100", "F81", "0LIASS"): -15,        # 500×(0.90-0.95)×0.6
    ("P", "100", "F99", "0LIASS"): 2970,       # 2760-60+285-15
    ("P", "700", "F20", "0LIASS"): 570,        # 1000×0.95×0.6
    ("P", "700", "F81", "0LIASS"): -30,        # 1000×(0.90-0.95)×0.6
    ("P", "700", "F99", "0LIASS"): 540,        # 570-30

    # ── S (GBP, globale ×1.0, sortie) — F98 miroir + F99 = 0 ───────────
    # Sortante : F98 = -(F00+F20) en GBP, puis converti à close_n (terminal)
    ("S", "100", "F00", "0LIASS"): 6900,       # 6000×1.15
    ("S", "100", "F80", "0LIASS"): -180,       # 6000×(1.12-1.15)
    ("S", "100", "F20", "0LIASS"): 708,        # 600×1.18
    ("S", "100", "F81", "0LIASS"): -36,        # 600×(1.12-1.18)
    ("S", "100", "F98", "0LIASS"): -7392,      # -(6000+600)×1.12 = -6600×1.12
    ("S", "100", "F99", "0LIASS"): 0,          # 6900-180+708-36-7392
    ("S", "700", "F20", "0LIASS"): 1770,       # 1500×1.18
    ("S", "700", "F81", "0LIASS"): -90,        # 1500×(1.12-1.18)
    ("S", "700", "F98", "0LIASS"): -1680,      # -(1500)×1.12
    ("S", "700", "F99", "0LIASS"): 0,          # 1770-90-1680

    # ── M / 1AJUST (grain séparé) ──────────────────────────────────────
    ("M", "100", "F00", "1AJUST"): 500,        # EUR, copie
    ("M", "100", "F99", "1AJUST"): 500,        # F00=500, pas de F20

    # ── Staging : préfixes 2/3/4 (injection au niveau cible, cf. staging.rs) ───
    # 2MAN → converted (avant écarts) puis D ; EUR donc pas de FX
    ("M", "100", "F20", "2MAN"): 200,
    ("M", "100", "F99", "2MAN"): 200,          # F20 reconstitué
    # 3MAN → consolidated, avant le × pct
    ("M", "100", "F20", "3MAN"): 300,
    ("M", "100", "F99", "3MAN"): 300,
    # 4MAN → consolidated, après le × pct (injecté tel quel), clôture reconstituée sur place
    ("M", "100", "F20", "4MAN"): 400,
    ("M", "100", "F99", "4MAN"): 400,
}


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


def consolidation_id(phase="REEL", exercice="2024"):
    """Résout l'id technique d'une consolidation par sa phase + son exercice
    (clé naturelle, post-Q41 : plus de `code` textuel). Renvoie None si absente."""
    _, rows = req("GET", "/api/consolidations", expect=200)
    if not isinstance(rows, list):
        return None
    for c in rows:
        if c.get("phase") == phase and str(c.get("exercice")) == str(exercice):
            return c.get("id")
    return None


def fetch_grains(level, consolidation, entry_period="2024", limit=100000):
    """Récupère les écritures d'un niveau et agrège par grain
    (entity, account, flow, nature) → somme des montants."""
    rows = _fetch_entries(level, consolidation, entry_period, limit)
    grains = {}
    for e in rows:
        key = (e["entity"], e["account"], e["flow"], e["nature"])
        grains[key] = grains.get(key, 0.0) + float(e["amount"])
    return grains


def _fetch_entries(level, consolidation, entry_period, limit):
    """Pagine sur /api/entries pour récupérer TOUTES les écritures d'un niveau,
    isolées par `consolidation` (id technique, filtre fact_entry post-Q41)."""
    out = []
    offset = 0
    page = min(limit, 5000)
    while True:
        path = (f"/api/entries?level={level}&consolidation={consolidation}"
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


# =========================================================================
#  Compteur de résultats
# =========================================================================
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
#  Comparaison golden master
# =========================================================================
def compare_consolidated(cid):
    print(bold("\n─ Montants consolidés (golden master) ─"))
    actual = fetch_grains("consolidated", cid)

    expected = EXPECTED_CONSOLIDATED
    ek = set(expected)
    ak = set(actual)

    # Grains attendus manquants dans l'actual
    missing = sorted(ek - ak)
    # Grains présents dans l'actual mais non attendus
    extra = sorted(ak - ek)

    if missing:
        R.ko("grains attendus absents du consolidé",
             "\n".join(f"{g} = attendu {fmt(expected[g])}, absent"
                       for g in missing))
    else:
        R.ok(f"tous les grains attendus sont présents ({len(ek)})")

    if extra:
        R.ko("grans inattendus au consolidé",
             "\n".join(f"{g} = {fmt(actual[g])} (non prévu)"
                       for g in extra))
    else:
        R.ok("aucun grain inattendu au consolidé")

    # Comparaison valeur par valeur
    diffs = []
    matched = 0
    for g in sorted(ek & ak):
        exp = expected[g]
        act = actual[g]
        if approx(act, exp):
            matched += 1
        else:
            diffs.append(f"{g} : attendu {fmt(exp)}, obtenu {fmt(act)} "
                         f"(Δ {fmt(act - exp)})")

    if diffs:
        R.ko(f"valeurs correctes ({matched}/{len(ek)})",
             "\n".join(diffs))
    elif matched == len(ek):
        R.ok(f"les {len(ek)} montants correspondent (±{TOL})")


# =========================================================================
#  Invariants structurels
# =========================================================================
def check_invariants(cid):
    print(bold("\n─ Invariants structurels ─"))

    corporate = _fetch_entries("corporate", cid, "2024", 100000)
    converted = _fetch_entries("converted", cid, "2024", 100000)
    consolidated = _fetch_entries("consolidated", cid, "2024", 100000)

    # ⚠️ Niveau `reclassified` supprimé (pipeline 3 niveaux). Les invariants qui
    # en dépendaient (2b ; 3 à reclassified ; 4 sortie F98 native ; 6b ; 7b) sont
    # neutralisés : ils relèvent désormais des règles de périmètre (tests Rust
    # `#[ignore]`). À réécrire lors de la recette périmètre-par-règles.
    print(yellow("  ⏭  invariants reclassified (2b, 3@reclass, 4, 6b, 7b) — "
                 "OBSOLÈTES (niveau supprimé, sortie périmètre → règles)"))

    def has(rows, **kw):
        return [r for r in rows if all(r.get(k) == v for k, v in kw.items())]

    def natures_with_prefix(rows, prefix):
        return [r for r in rows if str(r.get("nature", "")).startswith(prefix)]

    # 1 — E a 0 ligne à consolidated (équivalence exclue par l'étape D)
    e_cons = has(consolidated, entity="E")
    check("1. E a 0 ligne à consolidated (équivalence exclue)",
          len(e_cons) == 0,
          f"{len(e_cons)} ligne(s) E trouvée(s)")

    # 2 — E est présente à corporate / converted (reclassified supprimé)
    check("2a. E présente à corporate", len(has(corporate, entity="E")) > 0)
    check("2c. E présente à converted", len(has(converted, entity="E")) > 0)

    # 3 — S a F99=0 à consolidated (la sortante ne fuit pas). NB : tant que la
    #     sortie de périmètre n'est pas portée par une règle, cet invariant peut
    #     échouer (S n'est plus extournée nativement).
    s99 = [r for r in consolidated if r.get("entity") == "S"
           and r.get("flow") == "F99" and r.get("nature") == "0LIASS"]
    ok = len(s99) > 0 and all(approx(float(r["amount"]), 0.0) for r in s99)
    check("3. S/0LIASS F99 = 0 à consolidated", ok,
          f"lignes = {[(r['account'], r['amount']) for r in s99]}")

    # 5 — Préfixe 2 absent de corporate (staging skip A)
    check("5. préfixe 2 absent de corporate",
          len(natures_with_prefix(corporate, "2")) == 0,
          f"{len(natures_with_prefix(corporate, '2'))} ligne(s)")

    # 6 — Préfixe 3 absent de corporate (skip A+C ; cible consolidated)
    check("6a. préfixe 3 absent de corporate",
          len(natures_with_prefix(corporate, "3")) == 0)

    # 7 — Préfixe 4 absent de corporate / converted (cible consolidated, après pct)
    check("7a. préfixe 4 absent de corporate",
          len(natures_with_prefix(corporate, "4")) == 0)
    check("7c. préfixe 4 absent de converted",
          len(natures_with_prefix(converted, "4")) == 0)

    # 8 — F99 saisi est écrasé : S/100/0LIASS F99 reconstruit = 0 (≠ saisi 6600)
    s99c = [r for r in consolidated if r.get("entity") == "S"
            and r.get("account") == "100" and r.get("flow") == "F99"
            and r.get("nature") == "0LIASS"]
    ok8 = len(s99c) == 1 and approx(float(s99c[0]["amount"]), 0.0)
    check("8. F99 saisi (6600) écrasé → 0 sur S/100",
          ok8, f"obtenu = {s99c}")

    # 9 — 1AJUST ≠ 0LIASS au même grain M/100/F99
    f99_liass = [r for r in consolidated if r.get("entity") == "M"
                 and r.get("account") == "100" and r.get("flow") == "F99"
                 and r.get("nature") == "0LIASS"]
    f99_ajust = [r for r in consolidated if r.get("entity") == "M"
                 and r.get("account") == "100" and r.get("flow") == "F99"
                 and r.get("nature") == "1AJUST"]
    ok9 = (len(f99_liass) == 1 and len(f99_ajust) == 1
           and not approx(float(f99_liass[0]["amount"]),
                          float(f99_ajust[0]["amount"])))
    check("9. F99@1AJUST (500) ≠ F99@0LIASS (11000) sur M/100",
          ok9,
          f"0LIASS={f99_liass}, 1AJUST={f99_ajust}")

    # 10 — Identité de reconstruction : Σ(constituants reportant à F99) = F99
    #       pour chaque grain au niveau consolidated.
    check_closure_identity(consolidated, "consolidated")


def check_closure_identity(rows, level):
    """Pour chaque grain (entity, account, nature), vérifie que
    Σ(flux ≠ F99, flux_de_report = F99) == F99."""
    # On assume un seul flux de clôture : F99 (auto-référentiel).
    # Constituants = tous les flux sauf F99 (ils reportent tous à F99 ici).
    from collections import defaultdict
    sums = defaultdict(float)   # grain -> somme des constituants
    f99 = {}                    # grain -> montant F99
    for r in rows:
        grain = (r["entity"], r["account"], r["nature"])
        amt = float(r["amount"])
        if r["flow"] == "F99":
            f99[grain] = f99.get(grain, 0.0) + amt
        else:
            sums[grain] += amt

    bad = []
    n_ok = 0
    for grain, total in sorted(f99.items()):
        if approx(total, sums.get(grain, 0.0)):
            n_ok += 1
        else:
            bad.append(f"{grain} : Σconstituants={fmt(sums.get(grain, 0.0))}, "
                       f"F99={fmt(total)}, Δ={fmt(total - sums.get(grain, 0.0))}")
    if bad:
        R.ko(f"10. identité F99 = Σconstituants ({n_ok} OK / {len(f99)})",
             "\n".join(bad))
    else:
        R.ok(f"10. identité de reconstruction F99 = Σconstituants "
             f"({n_ok} grains)")


# =========================================================================
#  Démarrage / arrêt du serveur
# =========================================================================
def start_server(binary, port, csv_dir):
    env = os.environ.copy()
    env["CONSO_CSV_DIR"] = csv_dir
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


# =========================================================================
#  Orchestration
# =========================================================================
def run_tests():
    print(bold("\n═ Moteur de consolidation — golden master ═\n"))

    # 1. Health
    print(dim("1. Health"))
    code, body = req("GET", "/api/health", expect=200)
    check("health → 200 + status=ok",
          isinstance(body, dict) and body.get("status") == "ok",
          f"body={body}")

    # 2. Reset → état propre (recharge les CSV golden)
    print(dim("\n2. Reset (recharge data_golden)"))
    code, body = req("POST", "/api/reset", expect=200)
    check("reset → 200", code == 200, f"body={body}")
    check("reset a chargé des écritures",
          isinstance(body, dict) and body.get("entries", 0) > 0,
          f"body={body}")

    # 3. Run pipeline (3 niveaux : corporate → converted → consolidated)
    print(dim("\n3. Pipeline A→C→D"))
    cid = consolidation_id(phase="REEL", exercice="2024")
    check("consolidation REEL/2024 résolue", cid is not None,
          "aucune consolidation (phase=REEL, exercice=2024) dans /api/consolidations")
    code, body = req("POST", "/api/run", body={"consolidation_id": cid}, expect=200)
    check("run → 200", code == 200, f"body={body}")
    if isinstance(body, dict):
        for lvl in ("corporate", "converted", "consolidated"):
            n = body.get(lvl, 0)
            check(f"run produit {lvl} (>0)", n > 0, f"{lvl}={n}")

    # 4. Golden master — montants consolidés
    compare_consolidated(cid)

    # 5. Invariants structurels
    check_invariants(cid)


def main():
    parser = argparse.ArgumentParser(description="Golden master conso-server")
    parser.add_argument("--port", type=int, default=3000)
    parser.add_argument("--binary", default=None,
                        help="Chemin du binaire conso-server")
    parser.add_argument("--csv-dir", default="data_golden",
                        help="Répertoire des CSV golden (défaut: data_golden)")
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
            print(dim("  Logs : CONSO_CSV_DIR=data_golden ./target/release/conso-server"))
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
        print(green(f"  ✓ {R.passed}/{total} vérifications passent — GOLDEN MASTER OK"))
    else:
        print(red(f"  ✗ {R.failed}/{total} échecs"))
        for name, detail in R.failures:
            print(f"    {red('•')} {name}")
    print()

    sys.exit(1 if R.failed > 0 else 0)


if __name__ == "__main__":
    main()
