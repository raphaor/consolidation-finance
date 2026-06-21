# interco — Résultats attendus

Source : feuille *Interco* du Excel (cas 3 : A & B proportionnelles, A en USD,
année N). Périmètre : A `prop 0,85`, B `prop 0,75` → `min_pct_integration = 0,75`.

## Saisie corporate (A, USD)

| Compte | Partner | Flow | Montant USD |
|---|---|---|---|
| 467 (D) | ∅ | F00 | 1000 |
| 467 (D) | ∅ | F20 | 100 |
| 467 (D) | **B** | F20 | 100 |
| 468 (C) | ∅ | F00 | 1000 |
| 468 (C) | ∅ | F20 | 100 |

## Niveau *converti* (avant règle)

Taux : F00 → close_n1 (2,0), F20 → moyen (2,2), écart F00 = `× (2,5 − 2,0) = × 0,5`,
écart F20 = `× (2,5 − 2,2) = × 0,3`.

| Compte | Partner | Nature | F00 | F20 | F80 | F81 | F99 |
|---|---|---|---|---|---|---|---|
| 467 (D) | ∅ | 0LIASS | 2000 | 220 | 500 | 30 | **2750** |
| 467 (D) | B | 0LIASS | 0 | 220 | 0 | 30 | **250** |
| 468 (C) | ∅ | 0LIASS | 2000 | 220 | 500 | 30 | **2750** |

> Note : pas de F80 sur la ligne interco (pas de F00 interco saisi).

## Après règle **R-INT** (niveau *converti*, coef `min_pct_integration = 0,75`)

4 opérations × 1 ligne source interco = **4 lignes générées** sur la seule
écriture `467 / partner=B` (la sélection `partner IS NOT NULL` ne matche qu'elle).

| Op | Compte | Partner | Nature | F20 | F81 | F99 | Sens |
|---|---|---|---|---|---|---|---|
| R-INT-1 | 467 | **B** (inherit) | 2ELI | **−165** | **−22,5** | **−187,5** | Extourne, audit |
| R-INT-2 | 467 | **∅** (null) | 2ELI | **−165** | **−22,5** | **−187,5** | Extourne, bilan |
| R-INT-3 | **471L** | **B** (inherit) | 2ELI | **+165** | **+22,5** | **+187,5** | Contrepartie, audit |
| R-INT-4 | **471L** | **∅** (null) | 2ELI | **+165** | **+22,5** | **+187,5** | Contrepartie, bilan |

Calcul : `220 × 0,75 = 165`, `30 × 0,75 = 22,5`, `165 + 22,5 = 187,5`.

> Vérification (cf. Excel ligne 40, *Check elim*) :
> Sur la ligne interco consolidée 467/B 0LIAS, on doit retrouver un solde
> net = `(250 × 0,85) + (−187,5) = 212,5 − 187,5 = 25`. Le *Check elim* du
> Excel affiche bien `F20=22, F81=3, F99=25` pour cette ligne (résidu de
> l'écart A=0,85 − B=0,75 = 0,10 × montant d'origine, soit `220 × 0,10 = 22`).

## Niveau *consolidé*

`× pct_integration` appliqué aux **0LIASS uniquement** (la nature `2ELI` est
**exclue** du `× pct` — filtre à ajouter dans `step_d` : `nature NOT LIKE '2%'`).

| Compte | Partner | Nature | F00 | F20 | F80 | F81 | F99 | Coef appliqué |
|---|---|---|---|---|---|---|---|---|
| 467 (D) | ∅ | 0LIASS | 1700 | 187 | 425 | 25,5 | **2337,5** | × 0,85 |
| 467 (D) | B | 0LIASS | 0 | 187 | 0 | 25,5 | **212,5** | × 0,85 |
| 468 (C) | ∅ | 0LIASS | 1700 | 187 | 425 | 25,5 | **2337,5** | × 0,85 |
| 467 (D) | B | 2ELI | 0 | −165 | 0 | −22,5 | **−187,5** | × 1 (non re-multiplié) |
| 467 (D) | ∅ | 2ELI | 0 | −165 | 0 | −22,5 | **−187,5** | × 1 |
| 471L (D) | B | 2ELI | 0 | +165 | 0 | +22,5 | **+187,5** | × 1 |
| 471L (D) | ∅ | 2ELI | 0 | +165 | 0 | +22,5 | **+187,5** | × 1 |

### Bilan consolidé (somme par compte, partner=∅ uniquement)

| Compte | F00 | F20 | F80 | F81 | F99 |
|---|---|---|---|---|---|
| 467 (D, main) | 1700 | 187 | 425 | 25,5 | 2337,5 |
| 467 (D, 2ELI) | 0 | −165 | 0 | −22,5 | −187,5 |
| 468 (C, main) | 1700 | 187 | 425 | 25,5 | 2337,5 |
| 471L (D, 2ELI) | 0 | +165 | 0 | +22,5 | +187,5 |
| **Total** | **3400** | **374** | **850** | **51** | **4675** |

> Le Bilan est équilibré : `3400 + 374 + 850 + 51 = 4675` en débit comme en
> crédit (467 + 471L côté débit = 468 côté crédit). L'élimination interco a
> neutralisé la créance reportée sur B sans casser l'équilibre.

## Pré-requis moteur pour que ce smoke passe

1. **Coef C4 `min_pct_integration`** ajouté à [`rules.rs`](../../../src/rules.rs)
   (`LEAST(p_ent.pct_integration, p_part.pct_integration)`).
2. **Filtre `nature NOT LIKE '2%'`** dans `pipeline::consolidate` (sinon les
   écritures 2ELI seraient multipliées par 0,85 → déséquilibre).
3. La table `dim_rule` / `dim_ruleset` est chargée depuis `rules.csv` +
   `rulesets.csv` + `ruleset_items.csv` (le scénario `SMOKE_IC` référence
   `RS_INTERCO` dans `scenarios.csv`).

## Requêtes SQL de contrôle

```sql
-- 1. Converti après règle R-INT (toutes les lignes 2ELI)
SELECT account, partner, flow, SUM(amount) AS total
FROM fact_entry
WHERE scenario='SMOKE_IC' AND level='converted' AND nature='2ELI'
GROUP BY account, partner, flow
ORDER BY account, partner, flow;

-- Attendu :
--   467 / B    / F20 → -165
--   467 / B    / F81 → -22.5
--   467 / B    / F99 → -187.5
--   467 / NULL / F20 → -165
--   467 / NULL / F81 → -22.5
--   467 / NULL / F99 → -187.5
--   471L / B   / F20 → 165
--   471L / B   / F81 → 22.5
--   471L / B   / F99 → 187.5
--   471L / NULL/ F20 → 165
--   471L / NULL/ F81 → 22.5
--   471L / NULL/ F99 → 187.5

-- 2. Consolidé (vérif filtre 2ELI)
SELECT account, nature, flow, SUM(amount) AS total
FROM fact_entry
WHERE scenario='SMOKE_IC' AND level='consolidated' AND account='467'
GROUP BY account, nature, flow
ORDER BY nature, flow;

-- Attendu (nature 0LIASS × 0,85 ; nature 2ELI × 1) :
--   467 / 0LIASS / F00 → 1700   (= 2000 × 0,85)
--   467 / 0LIASS / F20 → 187    (= 220 × 0,85)
--   467 / 0LIASS / F80 → 425
--   467 / 0LIASS / F81 → 25,5
--   467 / 0LIASS / F99 → 2337,5
--   467 / 2ELI   / F20 → -330   (= 2 × -165 : partner=B + partner=NULL)
--   467 / 2ELI   / F81 → -45
--   467 / 2ELI   / F99 → -375

-- 3. Check elim (solde net ligne interco 467/B consolidée)
SELECT flow, SUM(amount) FROM fact_entry
WHERE scenario='SMOKE_IC' AND level='consolidated'
  AND account='467' AND partner='B'
GROUP BY flow ORDER BY flow;
-- Attendu : F20=22 (187 + -165), F81=3 (25,5 + -22,5), F99=25 (212,5 + -187,5)
-- → correspond exactement au "Check elim" de la feuille Interco du Excel.
```
