# conv_integ — Résultats attendus

Source : feuille *Conv&Integ* du Excel `Cas de consolidation.xlsx`.

## Cas 1A — A globale en USD, exercice d'entrée 2024

**Périmètre** : A `globale` `pct_integration=1.00` `entree=true` en 2024.
**Taux** : USD 2023 (close_n1) = 2,0 ; USD 2024 close = 2,5, moyen = 2,2.
**Saisie corporate** (A, USD) :

| Compte | Flow | Montant USD |
|---|---|---|
| 467 (D) | F00 | 1000 |
| 467 (D) | F20 | 100 |
| 468 (C) | F00 | 1000 |
| 468 (C) | F20 | 100 |

### Niveau *converti* (EUR)

Taux appliqués : F00 → close_n1 (2,0), F20 → moyen (2,2), F80 = écart F00 =
`1000 × (2,5 − 2,0) = 500`, F81 = écart F20 = `100 × (2,5 − 2,2) = 30`.

| Compte | F00 | F20 | F80 | F81 | F99 |
|---|---|---|---|---|---|
| 467 (D) | 2000 | 220 | 500 | 30 | **2750** |
| 468 (C) | 2000 | 220 | 500 | 30 | **2750** |

Vérifications :
- `F99 = F00 + F20 + F80 + F81` (identité de reconstruction)
- Total Bilan = 0 (467 D − 468 C équilibré sur chaque flux)

### Niveau *consolidé*

Méthode `globale` → `× pct_integration (1,00)` → **identique au converti**.

### Requête SQL de contrôle

```sql
SELECT account, flow, SUM(amount) AS total
FROM fact_entry
WHERE scenario='SMOKE_CI' AND level='converted'
GROUP BY account, flow
ORDER BY account, flow;
-- Attendu :
--   467 / F00 → 2000
--   467 / F20 → 220
--   467 / F80 → 500
--   467 / F81 → 30
--   467 / F99 → 2750
--   468 / idem
```

---

## Cas 1B — A proportionnelle, variation de % sur 2 exercices

> **Non couvert par les CSV courants** (à étendre avec un périmètre multi-période).
> Documenté ici pour référence — sert de spec à la règle **R-F90**.

**Périmètre** :

```
perimeter_set,entity,period,methode,pct_interet,pct_integration,entree,sortie
SMOKE_PERIM_CI,A,2024,proportionnelle,1.00,0.80,true,false    # année N
SMOKE_PERIM_CI,A,2025,proportionnelle,1.00,0.85,false,false   # année N+1
```

**Taux** :
- USD 2023 close = 2,0 ; USD 2024 close = 2,5 / moyen = 2,2 (année N)
- USD 2024 close = 2,5 ; USD 2025 close = 2,9 / moyen = 2,6 (année N+1)

### Attendu consolidé N+1 (sans R-F90)

| Compte | F00 | F20 | F80 | F81 | F99 |
|---|---|---|---|---|---|
| 467 (D) | **1600** | 187 | 425 | 25,5 | 2337,5 |

Détail :
- `F00 = 1600` → à-nouveau figé au % N (2000 × 0,80) — **non ré-appliqué** du % N+1
- `F20 = 220 × 0,85 = 187`
- `F80 = 500 × 0,85 = 425`
- `F81 = 30 × 0,85 = 25,5`
- `F99 = 1600 + 187 + 425 + 25,5 = 2337,5`

> ⚠ **Sans règle R-F90**, le Bilan consolidé est **incohérent** : la société
> vaut en réalité `2000 × 0,85 + 220 × 0,85 + … = 2337,5 + 100` mais le F99
> affiché est trop bas de 100 (= le rattrapage de l'à-nouveau).

### Attendu consolidé N+1 (avec règle R-F90)

| Compte | F00 | F20 | F80 | F81 | **F90** | F99 |
|---|---|---|---|---|---|---|
| 467 (D) | 1600 | 187 | 425 | 25,5 | **100** | 2437,5 |

Formule du F90 :
```
F90 = (pct_N+1 − pct_N) × F00_converti
    = (0,85 − 0,80) × 2000
    = 100
```

Le F99 reconstruit devient `1600 + 187 + 425 + 25,5 + 100 = 2337,5 + 100 = 2437,5`.
✅ Cohérent avec une intégration à 85 % sur la totalité du bilan reporté.

> Note : on retrouve exactement les chiffres de la feuille *Conv&Integ* du
> Excel, lignes 23-27 (cas « Année 1 » de la 3ᵉ colonne).

---

## Cas 1C — Année N+2 (variation 0,85 → 0,90)

Report en cascade : F00 N+2 = F99 N+1 consolidé (= 2437,5 ci-dessus, déjà
incluant le F90 N+1). Le rattrapage N+2 porte sur la totalité du bilan à
nouveau consolidé (F00_reporté).

```
F90 N+2 = (0,90 − 0,85) × F00_converti_N+2
        = 0,05 × 2750     # F00 corp 2026 × close_n1
        = 137,5
```

(Correspond aux lignes 24-26 du Excel, 4ᵉ colonne.)

---

## Désactivation du cas 1A pour tester 1B/1C

Pour activer 1B/1C, remplacer dans ce dossier :
- `perimeter.csv` → version multi-période ci-dessus
- `periods.csv` → ajouter 2025 (et 2026 pour 1C)
- `rates.csv` → ajouter USD 2025 (close 2,9, moyen 2,6)
- `scenarios.csv` → `entry_period=2025` et idéalement un `a_nouveau_scenario`
  pointant vers un snapshot de SMOKE_CI 2024 clôturé
- `entries.csv` → ajouter les écritures 2025 (F20 uniquement, pas de F00)

Et implémenter la règle R-F90 (cf.
[`docs/CAS_CONSO_TEST.md` §3.2](../../../docs/CAS_CONSO_TEST.md)).
