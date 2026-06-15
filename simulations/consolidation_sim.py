#!/usr/bin/env python3
"""
Simulation du pipeline de consolidation par les flux.
Test des 2 questions ouvertes :
  1. Entrées/sorties de périmètre AVANT ou APRÈS conversion multi-devises ?
  2. La mise en équivalence par flag capitaux_propres est-elle la bonne approche ?

Groupe de test :
  - Mère (M) : EUR, intégration globale, périmètre continu
  - Filiale A (A) : USD, intégration globale, ENTRE en N
  - Filiale B (B) : GBP, intégration globale, SORT en N
  - Filiale C (C) : CHF, mise en équivalence, périmètre continu

Devise de présentation : EUR
"""

from dataclasses import dataclass, field
from typing import Optional
from decimal import Decimal, getcontext

getcontext().prec = 28

# ─────────────────────────────────────────────────────
#  Helpers
# ─────────────────────────────────────────────────────

def D(x):
    return Decimal(str(x))

def fmt(x):
    """Format avec 2 décimales."""
    return f"{x:.2f}"

# ─────────────────────────────────────────────────────
#  Données du scénario
# ─────────────────────────────────────────────────────

RATES = {
    # (devise_source, role_taux) -> taux vers EUR
    ('USD', 'close_n1'):  D('0.92'),   # clôture N-1 : 1 USD = 0.92 EUR
    ('USD', 'avg_n'):     D('0.95'),   # moyen N
    ('USD', 'close_n'):   D('0.90'),   # clôture N : 1 USD = 0.90 EUR
    ('GBP', 'close_n1'):  D('1.15'),
    ('GBP', 'avg_n'):     D('1.18'),
    ('GBP', 'close_n'):   D('1.12'),
    ('CHF', 'close_n1'):  D('1.05'),
    ('CHF', 'avg_n'):     D('1.06'),
    ('CHF', 'close_n'):   D('1.03'),
}

# ─────────────────────────────────────────────────────
#  Flux : chaque écriture est (entity, account, flow_code, amount_functional, currency)
# ─────────────────────────────────────────────────────

@dataclass
class Entry:
    entity: str
    account: str
    flow: str         # F00, F20, F01, F98...
    amount_func: Decimal  # en devise fonctionnelle
    currency: str
    account_class: str = 'bilan'  # 'bilan', 'resultat', 'equity'

# ─────────────────────────────────────────────────────
#  Pipeline — Approche A : Reclassification AVANT conversion
# ─────────────────────────────────────────────────────

def run_approach_A():
    """
    Pipeline : Corporate → Reclassification périmètre → Conversion → Consolidé
    """
    print("\n" + "="*70)
    print("  APPROCHE A : Reclassification de périmètre AVANT conversion")
    print("="*70)

    entries = []

    # ── Mère (M) : EUR, continue ──
    entries.append(Entry('M', '100_Capital',       'F00', D('10000'), 'EUR', 'equity'))
    entries.append(Entry('M', '400_Resultat',      'F00', D('5000'),  'EUR', 'resultat'))
    entries.append(Entry('M', '400_Resultat',      'F20', D('800'),   'EUR', 'resultat'))
    entries.append(Entry('M', '200_Immobilisations','F00', D('12000'), 'EUR', 'bilan'))
    entries.append(Entry('M', '200_Immobilisations','F20', D('500'),   'EUR', 'bilan'))
    entries.append(Entry('M', '300_Stocks',         'F00', D('3000'),  'EUR', 'bilan'))

    # ── Filiale A (USD) : ENTRE en N ──
    entries.append(Entry('A', '100_Capital',        'F00', D('5000'),  'USD', 'equity'))
    entries.append(Entry('A', '400_Resultat',       'F00', D('2000'),  'USD', 'resultat'))
    entries.append(Entry('A', '400_Resultat',       'F20', D('300'),   'USD', 'resultat'))
    entries.append(Entry('A', '200_Immobilisations', 'F00', D('8000'), 'USD', 'bilan'))
    entries.append(Entry('A', '200_Immobilisations', 'F20', D('400'),  'USD', 'bilan'))

    # ── Filiale B (GBP) : SORT en N (était en N-1) ──
    entries.append(Entry('B', '100_Capital',         'F00', D('4000'), 'GBP', 'equity'))
    entries.append(Entry('B', '400_Resultat',        'F00', D('1500'), 'GBP', 'resultat'))
    entries.append(Entry('B', '400_Resultat',        'F20', D('200'),  'GBP', 'resultat'))
    entries.append(Entry('B', '200_Immobilisations',  'F00', D('6000'),'GBP', 'bilan'))
    entries.append(Entry('B', '200_Immobilisations',  'F20', D('300'), 'GBP', 'bilan'))

    # ─────────────────────────────────────────────
    #  ÉTAPE 1 : Corporate (agrégation, déjà faite ci-dessus)
    # ─────────────────────────────────────────────

    # ─────────────────────────────────────────────
    #  ÉTAPE 2a : Reclassification de périmètre (AVANT conversion)
    # ─────────────────────────────────────────────
    print("\n── Étape 2a : Reclassification (en devise fonctionnelle) ──")

    reclassified = []
    for e in entries:
        if e.entity == 'A':
            # ENTREE : F00 → F01
            if e.flow == 'F00':
                new_e = Entry(e.entity, e.account, 'F01', e.amount_func, e.currency, e.account_class)
                reclassified.append(new_e)
                print(f"  {e.entity} {e.account} F00→F01 : {fmt(e.amount_func)} {e.currency}")
            else:
                reclassified.append(e)
        elif e.entity == 'B':
            # SORTIE : F00 + F20 → F98 (collapse en solde de clôture)
            if e.flow in ('F00', 'F20'):
                # On accumulate mais on va collapse
                reclassified.append(e)  # traité ensuite
            else:
                reclassified.append(e)
        else:
            reclassified.append(e)

    # Collapse de B vers F98
    b_by_account = {}
    remaining = []
    for e in reclassified:
        if e.entity == 'B' and e.flow in ('F00', 'F20'):
            key = (e.entity, e.account)
            b_by_account[key] = b_by_account.get(key, D('0')) + e.amount_func
        else:
            remaining.append(e)

    for (entity, account), total in b_by_account.items():
        # Déterminer la classe du compte
        cls = 'equity' if 'Capital' in account else ('resultat' if 'Resultat' in account else 'bilan')
        remaining.append(Entry(entity, account, 'F98', total, 'GBP', cls))
        print(f"  {entity} {account} F00+F20→F98 : {fmt(total)} GBP")

    entries = remaining

    # ─────────────────────────────────────────────
    #  ÉTAPE 2b : Conversion multi-devises
    # ─────────────────────────────────────────────
    print("\n── Étape 2b : Conversion vers EUR (devise de présentation) ──")

    converted = []
    for e in entries:
        if e.currency == 'EUR':
            converted.append((e, e.amount_func, D('0')))  # pas de conversion
            continue

        # Déterminer le taux selon le flux
        if e.flow == 'F00':
            rate_key = 'close_n1'
        elif e.flow == 'F01':
            rate_key = 'close_n1'  # F01 = entrée, hérite de la logique ouverture
        elif e.flow == 'F20':
            rate_key = 'avg_n'
        elif e.flow in ('F98', 'F99'):
            rate_key = 'close_n'  # terminal, taux clôture
        else:
            rate_key = 'close_n'

        rate = RATES[(e.currency, rate_key)]
        close_n = RATES[(e.currency, 'close_n')]
        amount_conv = e.amount_func * rate

        # Écart de conversion
        if e.flow in ('F98', 'F99'):
            gap = D('0')  # flux terminal, pas d'écart
        else:
            gap = e.amount_func * (close_n - rate)

        converted.append((e, amount_conv, gap))

        gap_flow = 'F80' if e.flow in ('F00', 'F01') else ('F81' if e.flow == 'F20' else '-')
        print(f"  {e.entity} {e.account} {e.flow} : "
              f"{fmt(e.amount_func)} {e.currency} × {rate} = {fmt(amount_conv)} EUR"
              f"  (écart → {gap_flow} = {fmt(gap)})")

    # ─────────────────────────────────────────────
    #  ÉTAPE 3 : Consolidé (agrégation par compte × flux)
    # ─────────────────────────────────────────────
    print("\n── Étape 3 : Tableau consolidé (compte × flux) ──")

    # Construire le tableau (account, flow) -> amount_EUR
    grid = {}
    for e, amt_conv, gap in converted:
        key = (e.account, e.flow)
        grid[key] = grid.get(key, D('0')) + amt_conv

        # Ajouter l'écart
        if gap != 0:
            gap_flow = 'F80' if e.flow in ('F00', 'F01') else 'F81'
            gap_key = (e.account, gap_flow)
            grid[gap_key] = grid.get(gap_key, D('0')) + gap

    # Calculer F99 par identité
    accounts = sorted(set(k[0] for k in grid))
    flows = ['F00', 'F01', 'F20', 'F80', 'F81', 'F98', 'F99']

    print(f"\n  {'Compte':<25} " + " ".join(f"{f:>10}" for f in flows))
    print("  " + "-"*95)

    for acc in accounts:
        row = []
        total = D('0')
        for f in flows:
            if f == 'F99':
                row.append(fmt(total))
            else:
                val = grid.get((acc, f), D('0'))
                row.append(fmt(val) if val != 0 else '-')
                total += val
        # Stocker F99
        grid[(acc, 'F99')] = total
        print(f"  {acc:<25} " + " ".join(f"{v:>10}" for v in row))

    # ─────────────────────────────────────────────
    #  Vérification de l'identité
    # ─────────────────────────────────────────────
    print("\n── Vérification identité F99 = F00 + F01 + F20 + F80 + F81 + F98 ──")
    all_ok = True
    for acc in accounts:
        f99 = grid.get((acc, 'F99'), D('0'))
        somme = sum(grid.get((acc, f), D('0')) for f in ['F00', 'F01', 'F20', 'F80', 'F81', 'F98'])
        ok = abs(f99 - somme) < D('0.01')
        status = "✓" if ok else "✗ ECHEC"
        if not ok:
            all_ok = False
        print(f"  {acc:<25} F99={fmt(f99):>10}  Σ={fmt(somme):>10}  {status}")

    return grid, all_ok


# ─────────────────────────────────────────────────────
#  Pipeline — Approche B : Conversion AVANT reclassification
# ─────────────────────────────────────────────────────

def run_approach_B():
    """
    Pipeline : Corporate → Conversion → Reclassification périmètre → Consolidé
    """
    print("\n" + "="*70)
    print("  APPROCHE B : Conversion multi-devises AVANT reclassification")
    print("="*70)

    entries = []

    # ── Mère (M) : EUR, continue ──
    entries.append(Entry('M', '100_Capital',       'F00', D('10000'), 'EUR', 'equity'))
    entries.append(Entry('M', '400_Resultat',      'F00', D('5000'),  'EUR', 'resultat'))
    entries.append(Entry('M', '400_Resultat',      'F20', D('800'),   'EUR', 'resultat'))
    entries.append(Entry('M', '200_Immobilisations','F00', D('12000'), 'EUR', 'bilan'))
    entries.append(Entry('M', '200_Immobilisations','F20', D('500'),   'EUR', 'bilan'))
    entries.append(Entry('M', '300_Stocks',         'F00', D('3000'),  'EUR', 'bilan'))

    # ── Filiale A (USD) : ENTRE en N ──
    entries.append(Entry('A', '100_Capital',        'F00', D('5000'),  'USD', 'equity'))
    entries.append(Entry('A', '400_Resultat',       'F00', D('2000'),  'USD', 'resultat'))
    entries.append(Entry('A', '400_Resultat',       'F20', D('300'),   'USD', 'resultat'))
    entries.append(Entry('A', '200_Immobilisations', 'F00', D('8000'), 'USD', 'bilan'))
    entries.append(Entry('A', '200_Immobilisations', 'F20', D('400'),  'USD', 'bilan'))

    # ── Filiale B (GBP) : SORT en N ──
    entries.append(Entry('B', '100_Capital',         'F00', D('4000'), 'GBP', 'equity'))
    entries.append(Entry('B', '400_Resultat',        'F00', D('1500'), 'GBP', 'resultat'))
    entries.append(Entry('B', '400_Resultat',        'F20', D('200'),  'GBP', 'resultat'))
    entries.append(Entry('B', '200_Immobilisations',  'F00', D('6000'),'GBP', 'bilan'))
    entries.append(Entry('B', '200_Immobilisations',  'F20', D('300'), 'GBP', 'bilan'))

    # ─────────────────────────────────────────────
    #  ÉTAPE 2a : Conversion multi-devises (AVANT reclassification)
    # ─────────────────────────────────────────────
    print("\n── Étape 2a : Conversion vers EUR (sur les flux sociaux F00/F20) ──")

    converted = []  # (entity, account, flow, amount_EUR, gap)
    for e in entries:
        if e.currency == 'EUR':
            converted.append((e.entity, e.account, e.flow, e.amount_func, D('0')))
            continue

        if e.flow == 'F00':
            rate_key = 'close_n1'
        elif e.flow == 'F20':
            rate_key = 'avg_n'
        else:
            rate_key = 'close_n'

        rate = RATES[(e.currency, rate_key)]
        close_n = RATES[(e.currency, 'close_n')]
        amount_eur = e.amount_func * rate
        gap = e.amount_func * (close_n - rate)
        converted.append((e.entity, e.account, e.flow, amount_eur, gap))

        gap_flow = 'F80' if e.flow == 'F00' else ('F81' if e.flow == 'F20' else '-')
        print(f"  {e.entity} {e.account} {e.flow} : "
              f"{fmt(e.amount_func)} {e.currency} × {rate} = {fmt(amount_eur)} EUR"
              f"  (écart → {gap_flow} = {fmt(gap)})")

    # ─────────────────────────────────────────────
    #  ÉTAPE 2b : Reclassification de périmètre (APRÈS conversion, en EUR)
    # ─────────────────────────────────────────────
    print("\n── Étape 2b : Reclassification (en EUR, devise de présentation) ──")

    reclassified = []
    b_accumulator = {}  # (account) -> (sum_converted, sum_gaps)

    for entity, account, flow, amount_eur, gap in converted:
        if entity == 'A':
            # ENTREE : F00 → F01, mais l'écart F80 a déjà été généré !
            if flow == 'F00':
                new_flow = 'F01'
                print(f"  {entity} {account} F00→F01 : {fmt(amount_eur)} EUR")
                # PROBLÈME : l'écart F80 a été calculé contre F00.
                # Si F00 devient F01, l'écart F80 est-il toujours valide ?
                # L'écart reste F80 (partagé entre F00 et F01).
                reclassified.append((entity, account, new_flow, amount_eur, gap))
                print(f"    ⚠ Écart F80 = {fmt(gap)} EUR — orphelin (F00 disparu, "
                      f"reclassifié en F01)")
            else:
                reclassified.append((entity, account, flow, amount_eur, gap))

        elif entity == 'B':
            # SORTIE : F00 + F20 + F80 + F81 → F98 (collapse total)
            if flow in ('F00', 'F20'):
                key = account
                if key not in b_accumulator:
                    b_accumulator[key] = {'converted': D('0'), 'gaps': D('0'), 'detail': []}
                b_accumulator[key]['converted'] += amount_eur
                b_accumulator[key]['gaps'] += gap
                b_accumulator[key]['detail'].append(f"{flow}={fmt(amount_eur)} (écart={fmt(gap)})")
            else:
                reclassified.append((entity, account, flow, amount_eur, gap))
        else:
            reclassified.append((entity, account, flow, amount_eur, gap))

    # Collapse B vers F98
    print("\n  Collapse B → F98 :")
    for account, data in b_accumulator.items():
        # F98 = converted + gaps (valeur totale en EUR)
        f98_amount = data['converted'] + data['gaps']
        reclassified.append(('B', account, 'F98', f98_amount, D('0')))
        print(f"    {account} : {' + '.join(data['detail'])} → F98 = {fmt(f98_amount)} EUR")
        print(f"      (les écarts F80/F81 sont absorbés dans F98 — détail perdu)")

    # ─────────────────────────────────────────────
    #  ÉTAPE 3 : Consolidé
    # ─────────────────────────────────────────────
    print("\n── Étape 3 : Tableau consolidé (compte × flux) ──")

    grid = {}
    for entity, account, flow, amount_eur, gap in reclassified:
        key = (account, flow)
        grid[key] = grid.get(key, D('0')) + amount_eur

        if gap != 0:
            gap_flow = 'F80' if flow in ('F00', 'F01') else 'F81'
            gap_key = (account, gap_flow)
            grid[gap_key] = grid.get(gap_key, D('0')) + gap

    accounts = sorted(set(k[0] for k in grid))
    flows = ['F00', 'F01', 'F20', 'F80', 'F81', 'F98', 'F99']

    print(f"\n  {'Compte':<25} " + " ".join(f"{f:>10}" for f in flows))
    print("  " + "-"*95)

    for acc in accounts:
        row = []
        total = D('0')
        for f in flows:
            if f == 'F99':
                row.append(fmt(total))
            else:
                val = grid.get((acc, f), D('0'))
                row.append(fmt(val) if val != 0 else '-')
                total += val
        grid[(acc, 'F99')] = total
        print(f"  {acc:<25} " + " ".join(f"{v:>10}" for v in row))

    # Vérification
    print("\n── Vérification identité F99 = F00 + F01 + F20 + F80 + F81 + F98 ──")
    all_ok = True
    for acc in accounts:
        f99 = grid.get((acc, 'F99'), D('0'))
        somme = sum(grid.get((acc, f), D('0')) for f in ['F00', 'F01', 'F20', 'F80', 'F81', 'F98'])
        ok = abs(f99 - somme) < D('0.01')
        status = "✓" if ok else "✗ ECHEC"
        if not ok:
            all_ok = False
        print(f"  {acc:<25} F99={fmt(f99):>10}  Σ={fmt(somme):>10}  {status}")

    return grid, all_ok


# ─────────────────────────────────────────────────────
#  Comparaison des deux approches
# ─────────────────────────────────────────────────────

def compare(grid_A, grid_B):
    print("\n" + "="*70)
    print("  COMPARAISON A vs B")
    print("="*70)

    all_keys = sorted(set(list(grid_A.keys()) + list(grid_B.keys())))

    print(f"\n  {'(Compte, Flux)':<30} {'Approche A':>12} {'Approche B':>12} {'Δ':>12} {'Match':>6}")
    print("  " + "-"*75)

    all_match = True
    for key in all_keys:
        val_A = grid_A.get(key, D('0'))
        val_B = grid_B.get(key, D('0'))
        diff = val_A - val_B
        match = "✓" if abs(diff) < D('0.01') else "✗"
        if abs(diff) >= D('0.01'):
            all_match = False
        if val_A != 0 or val_B != 0:
            label = f"({key[0][:18]}, {key[1]})"
            print(f"  {label:<30} {fmt(val_A):>12} {fmt(val_B):>12} {fmt(diff):>12} {match:>6}")

    print(f"\n  Résultat : {'IDENTIQUE ✓' if all_match else 'DIFFÉRENCES ✗'}")
    return all_match


# ─────────────────────────────────────────────────────
#  Simulation Mise en Équivalence
# ─────────────────────────────────────────────────────

def run_equity_method():
    """
    Test de la mise en équivalence.

    Approche actuelle (doc) :
      - Comptes capitaux propres → agrégés au % d'intégration
      - Autres comptes → NON agrégés
      - Contrepartie sur compte actif (261E)
      - P&L condensé sur compte unique (880E)

    Problèmes identifiés :
      1. L'investissement initial (titres 261) n'est pas éliminé
      2. Pas de calcul d'écart d'acquisition (goodwill)
      3. En flux, comment gérer F00 (équité d'ouverture) vs variation ?
    """
    print("\n\n" + "="*70)
    print("  MISE EN ÉQUIVALENCE — Simulation")
    print("="*70)

    # Filiale C (CHF), mise en équivalence, % intégration = 40%
    # Détenue depuis plusieurs années (périmètre continu)
    pct_integration = D('0.40')

    # Données sociales de C en CHF
    c_capital = D('10000')      # capitaux propres (hors résultat)
    c_resultat_ouverture = D('2000')  # résultat N-1 (dans capitaux propres à l'ouverture)
    c_resultat_variation = D('500')   # résultat de l'exercice N

    # Total capitaux propres ouverture = capital + résultat cumulé
    c_equity_ouverture = c_capital + c_resultat_ouverture  # 12000 CHF
    c_equity_clôture = c_capital + c_resultat_ouverture + c_resultat_variation  # 12500 CHF

    # Investissement de la mère dans C (compte 261)
    c_titres = D('5500')  # CHF — valeur brute d'acquisition

    # Taux CHF→EUR
    close_n1 = RATES[('CHF', 'close_n1')]  # 1.05
    avg_n = RATES[('CHF', 'avg_n')]         # 1.06
    close_n = RATES[('CHF', 'close_n')]     # 1.03

    print(f"\n  Filiale C (CHF) — Mise en équivalence à {pct_integration*100}%")
    print(f"  Capitaux propres (hors rés.) : {fmt(c_capital)} CHF")
    print(f"  Résultat N-1 (dans cap. prop.) : {fmt(c_resultat_ouverture)} CHF")
    print(f"  Résultat N : {fmt(c_resultat_variation)} CHF")
    print(f"  Équité totale ouverture : {fmt(c_equity_ouverture)} CHF")
    print(f"  Équité totale clôture : {fmt(c_equity_clôture)} CHF")
    print(f"  Titres (261) détenus : {fmt(c_titres)} CHF")
    print(f"  Taux : clôture N-1={close_n1}, moyen={avg_n}, clôture N={close_n}")

    # ─────────────────────────────────────────────
    #  Approche actuelle (doc) : flag capitaux_propres
    # ─────────────────────────────────────────────
    print("\n" + "─"*50)
    print("  MÉTHODE A (doc actuel) : flag capitaux_propres")
    print("─"*50)

    # F00 (ouverture) en CHF
    f00_capital_chf = c_capital + c_resultat_ouverture  # 12000 (capitaux propres)
    f20_resultat_chf = c_resultat_variation              # 500 (P&L de l'année)

    # Conversion
    f00_eur = f00_capital_chf * close_n1  # capitaux propres → taux clôture N-1
    f00_gap = f00_capital_chf * (close_n - close_n1)
    f20_eur = f20_resultat_chf * avg_n    # résultat → taux moyen
    f20_gap = f20_resultat_chf * (close_n - avg_n)

    print(f"\n  F00 (cap. propres) : {fmt(f00_capital_chf)} CHF × {close_n1} = {fmt(f00_eur)} EUR")
    print(f"    Écart F80 = {fmt(f00_gap)} EUR")
    print(f"  F20 (résultat) : {fmt(f20_resultat_chf)} CHF × {avg_n} = {fmt(f20_eur)} EUR")
    print(f"    Écart F81 = {fmt(f20_gap)} EUR")

    # Application % d'intégration
    f00_conso = f00_eur * pct_integration
    f80_conso = f00_gap * pct_integration
    f20_conso = f20_eur * pct_integration
    f81_conso = f20_gap * pct_integration

    print(f"\n  × {pct_integration*100}% :")
    print(f"    F00 conso = {fmt(f00_conso)} EUR")
    print(f"    F80 conso = {fmt(f80_conso)} EUR")
    print(f"    F20 conso = {fmt(f20_conso)} EUR")
    print(f"    F81 conso = {fmt(f81_conso)} EUR")

    # P&L condensé sur 880E
    print(f"\n  → P&L : F20+F81 = {fmt(f20_conso + f81_conso)} EUR → compte 880E")
    # Contrepartie sur 261E (actif)
    total_equity_conso = f00_conso + f80_conso + f20_conso + f81_conso
    print(f"  → Contrepartie 261E (actif) = {fmt(total_equity_conso)} EUR")

    # MAIS : que devient l'investissement social (261) de la mère ?
    titres_eur = c_titres * close_n1  # à clôture N-1 (F00)
    titres_gap = c_titres * (close_n - close_n1)
    print(f"\n  ⚠ L'investissement (261) = {fmt(c_titres)} CHF n'est PAS éliminé !")
    print(f"    Il reste dans le bilan consolidé à {fmt(titres_eur)} EUR (F00)")
    print(f"    + écart F80 = {fmt(titres_gap)} EUR")
    print(f"    → DOUBLE COMPTABILISATION : titres {fmt(titres_eur + titres_gap)} EUR")
    print(f"      + contrepartie équivalence {fmt(total_equity_conso)} EUR")

    # ─────────────────────────────────────────────
    #  Approche alternative : élimination + remplacement
    # ─────────────────────────────────────────────
    print("\n" + "─"*50)
    print("  MÉTHODE B (alternative) : élimination titres + équivalence")
    print("─"*50)

    # 1. Éliminer les titres (261) de la mère
    print(f"\n  1. Élimination titres 261 : -{fmt(c_titres)} CHF")
    # En flux : F00 = -5500 CHF (extourne à l'ouverture)
    elim_f00_chf = -c_titres
    elim_f00_eur = elim_f00_chf * close_n1
    elim_f00_gap = elim_f00_chf * (close_n - close_n1)
    print(f"     F00 = {fmt(elim_f00_chf)} CHF × {close_n1} = {fmt(elim_f00_eur)} EUR")
    print(f"     F80 = {fmt(elim_f00_gap)} EUR")

    # 2. Reconnaître la quote-part d'équité
    print(f"\n  2. Quote-part capitaux propres ({pct_integration*100}%) :")
    print(f"     F00 = {fmt(f00_capital_chf)} × {close_n1} × {pct_integration} = {fmt(f00_conso)} EUR → 261E")
    print(f"     F80 = {fmt(f80_conso)} EUR → écart de conversion")

    # 3. Quote-part résultat
    print(f"\n  3. Quote-part résultat ({pct_integration*100}%) :")
    print(f"     F20 = {fmt(f20_resultat_chf)} × {avg_n} × {pct_integration} = {fmt(f20_conso)} EUR → 880E")
    print(f"     F81 = {fmt(f81_conso)} EUR → écart")

    # 4. Écart d'acquisition
    qpac_ouverture = f00_eur * pct_integration  # quote-part cap.propres à l'ouverture
    ecart_acq = titres_eur - qpac_ouverture
    print(f"\n  4. Écart d'acquisition :")
    print(f"     Titres = {fmt(titres_eur)} EUR")
    print(f"     QP cap. propres = {fmt(qpac_ouverture)} EUR")
    print(f"     Goodwill = {fmt(ecart_acq)} EUR")
    if ecart_acq > 0:
        print(f"     → Survaleur (goodwill positif)")
    else:
        print(f"     → Survaleur négative (boni)")

    # Bilan équivalence (méthode B)
    # Actif : 261E = QP cap.propres + goodwill = titres (en principe)
    # Résultat : 880E = QP résultat
    total_b = f00_conso + f80_conso + f20_conso + f81_conso
    print(f"\n  Bilan équivalence (Méthode B) :")
    print(f"    261E (actif) = {fmt(f00_conso + f80_conso)} EUR (QP cap. propres + écart conv.)")
    print(f"    261E inclut goodwill = {fmt(ecart_acq)} EUR (si non amorti)")
    print(f"    880E (résultat) = {fmt(f20_conso + f81_conso)} EUR (QP résultat)")

    # ─────────────────────────────────────────────
    #  Le vrai problème : comment ça se passe en FLUX ?
    # ─────────────────────────────────────────────
    print("\n" + "─"*50)
    print("  ANALYSE : la mise en équivalence en modèle par les flux")
    print("─"*50)

    print("""
  Le modèle par les flux repose sur : F99 = F00 + Σ(variations) + Σ(écarts)

  Pour la mise en équivalence, on a DEUX dimensions qui changent simultanément :
    1. Le COMPTE (redirection : titres 261 → équivalence 261E, P&L détaillé → 880E)
    2. Le FLUX (F00/F20/F80/F81 standards)

  Problème : l'élimination des titres (extourne de 261) et la reconnaissance
  de l'équivalence (261E) sont des écritures de NATURE DIFFÉRENTE :
    - L'extourne des titres est une écriture INTERCO / de participation
    - La reconnaissance d'équivalence est une RECLASSIFICATION de méthode

  Questions soulevées :
    Q-E1 : L'élimination des titres doit-elle être NATIVE (moteur) ou
           passer par l'éditeur de règles (post-MVP) ?
           → Actuellement c'est ni l'un ni l'autre : ignorée.

    Q-E2 : Le goodwill : calculé nativement (titres - QP cap.propres à l'entrée)
           ou saisi manuellement ?
           → Nécessite de connaître la date d'acquisition et les cap.propres
             à cette date, pas seulement à l'ouverture N.

    Q-E3 : En flux, l'équivalence d'OUVERTURE (F00) et l'élimination des
           titres (F00) se compensent-elles au même compte ou à des comptes
           différents ?
           → 261 (titres) vs 261E (équivalence) = comptes différents.
           → Il faut que le moteur génère l'écriture d'élimination ET
             l'écriture de reconnaissance, sur deux comptes distincts.

    Q-E4 : Le flag `capitaux_propres` suffit-il ?
           → NON. Il identifie quels comptes consolider, mais ne gère pas :
             - l'élimination des titres
             - le calcul du goodwill
             - la redirection du P&L vers un compte unique
           → Ce sont des mécanismes supplémentaires au-delà du simple flag.
    """)

    return


# ─────────────────────────────────────────────────────
#  MAIN
# ─────────────────────────────────────────────────────

if __name__ == '__main__':
    print("="*70)
    print("  SIMULATION CONSOLIDATION PAR LES FLUX")
    print("  Groupe : Mère (EUR) + A (USD, entrée) + B (GBP, sortie)")
    print("  Devise de présentation : EUR")
    print("="*70)

    print("\n\nTaux de change (vers EUR) :")
    for (curr, role), rate in sorted(RATES.items()):
        print(f"  {curr} {role:>10} : {rate}")

    grid_A, ok_A = run_approach_A()
    grid_B, ok_B = run_approach_B()
    identical = compare(grid_A, grid_B)

    run_equity_method()

    # ─────────────────────────────────────────────
    #  Synthèse
    # ─────────────────────────────────────────────
    print("\n\n" + "="*70)
    print("  SYNTHÈSE")
    print("="*70)

    print(f"""
  1. ENTRÉES/SORTIES : AVANT vs APRÈS CONVERSION
  ────────────────────────────────────────────────
  Résultat numérique : {'IDENTIQUE' if identical else 'DIFFÉRENT'}

  Mais l'APPROCHE A (avant) est nettement supérieure pour :

  a) TRACEABILITÉ
     - A : F01 hérite proprement de F00. L'écart F80 est généré contre F01.
           Chaîne d'audit claire : F00 social → F01 consolidé → F80 (écart de F01).
     - B : F80 est généré contre F00, puis F00 est reclassifié en F01.
           F80 devient orphelin : son flux parent n'existe plus.

  b) SORTIE DE PÉRIMÈTRE (le cas le plus problématique)
     - A : Collapse en devise fonctionnelle (F00+F20 → F98), puis conversion
           au taux clôture (terminal). Une seule écriture, propre.
     - B : La conversion génère F00_conv + F80 + F20_conv + F81.
           Pour collapsier vers F98, il faut absorber les écarts → détail perdu.
           Ou bien garder les écarts → ils sont orphelins (F00/F20 disparus).

  c) IMPLÉMENTATION
     - A : La reclassification est un simple relabeling de flux en fonctionnel.
           Puis le moteur de conversion fonctionne uniformément.
     - B : Le moteur de conversion doit gérer des flux déjà convertis,
           reclassifier en devise de présentation, et gérer les orphelins.

  d) IDENTITÉ DE RECONSTRUCTION
     - A : F99 = F00 + F01 + F20 + F80 + F81 + F98 (les écarts sont propres)
     - B : Pour les sorties, F98 absorbe les écarts → l'identité tient mais
           la granularité est perdue (F98 est un "fourre-tout").

  RECOMMANDATION : Reclassification AVANT conversion (Approche A).
  Le pipeline devient : Corporate → Reclassification → Conversion → Consolidé.
  La ligne de FLUX_CONSO.md §9 doit être mise à jour.

  2. MISE EN ÉQUIVALENCE
  ────────────────────────
  L'approche actuelle (flag capitaux_propres + contrepartie) est INCOMPLÈTE :

  a) L'ÉLIMINATION DES TITRES est manquante. Sans elle, double comptabilisation
     (titres + équivalence).

  b) LE GOODWILL n'est pas calculé. Or il faut connaître les capitaux propres
     à la date d'acquisition, pas seulement à l'ouverture N.

  c) Le flag `capitaux_propres` seul ne suffit pas : il faut aussi la redirection
     du P&L vers un compte unique, ce qui est un changement de dimension (compte).

  d) La mise en équivalence est hybride : partie native (agréger cap. propres
     au %, condenser le P&L) + partie qui ressemble à des règles (élimination
     titres, goodwill). À clarifier dans l'architecture.

  RECOMMANDATION : Détailler la mise en équivalence comme un mécanisme natif
  COMPLEXE (pas juste un flag), avec :
    - Élimination automatique des titres (nécessite données d'acquisition)
    - Calcul du goodwill
    - Condensation du P&L
    - Maintien en flux (F00/F20) avec redirection de compte

  Alternative : repousser la mise en équivalence au POST-MVP et se concentrer
  sur globale + proportionnelle pour le POC. C'est moins sexy mais plus réaliste.
""")
