# Cas de consolidation — Cahier de recette pour l'éditeur de règles

Source : `Conso rules/Cas de consolidation.xlsx` (3 feuilles de scénarios).

Annexe de [`REGLES_CONSO.md`](./REGLES_CONSO.md). Sert de **cahier de recette**
pour valider que le moteur de règles + le moteur natif produisent les bons
chiffres sur des cas représentatifs.

> **Note** (2026-06-28) : les datasets CSV `prototype/rust/data/smoke/` et le
> script `smoke_test.py` ont été supprimés par le chantier migration CSV→JSON
> (cf. [`PLAN_MIGRATION_CSV_JSON.md`](./PLAN_MIGRATION_CSV_JSON.md)). Ce document
> reste valable comme spécification des cas de test ; l'implémentation de
> référence vit désormais dans `tests/rules.rs`, `tests/golden.rs`,
> `tests/pipeline.rs`.

---

## 1. Synthèse : coefficients nécessaires

Le moteur de règles applique à chaque grain sélectionné un **facteur** =
`coefficient × multiplicateur` ([`REGLES_CONSO.md` §4.2](./REGLES_CONSO.md#42-facteur)).
L'analyse des 3 feuilles du Excel fait ressortir **6 coefficients distincts**,
dont 3 sont à ajouter au moteur.

| # | Coefficient | Formule | Statut | Usage |
|---|---|---|---|---|
| C1 | `constant` | valeur littérale | ✅ **existe** ([`rules.rs:548`](../prototype/rust/src/rules.rs)) | Copie, extourne, facteurs fixes |
| C2 | `pct_integration` | `p_ent.pct_integration` | ✅ **existe** | Intégration globale/proportionnelle simple (étape D) |
| C3 | `pct_interet` | `p_ent.pct_interet` | ✅ **existe** | Quote-part d'intérêt (intérêts minoritaires post-MVP) |
| **C4** | **`min_pct_integration`** | `LEAST(p_ent.pct_integration, p_part.pct_integration)` | 🆕 **à ajouter** | Élimination interco **standard** (au niveau *converti*) |
| **C5** | **`variation_pct_integration`** | `p_ent.pct_integration − p_ent.pct_integration_N-1` | 🆕 **à ajouter** | Génération du **F90** (variation de % d'intégration sur l'à-nouveau) |
| **C6** | **`ratio_partner_over_entity_pct`** | `p_part.pct_integration / p_ent.pct_integration` | 🆕 **à ajouter** | Élimination interco **inverse** (au niveau *consolidé*, après application du %) |

### 1.1 Justifications (issues du Excel)

**C4 — `min_pct_integration`** *(le plus bloquant)*

La feuille *Interco* pose un `Coef Elim = 0,75` qui n'est autre que
`min(0,80 ; 0,70)` en année N, puis `min(0,85 ; 0,75) = 0,75` et
`min(0,85 ; 0,80) = 0,80` en N+1. L'élimination interco standard se fait
**avant** l'application du `% d'intégration`, au **Min des taux** des deux
entités liées — pour neutraliser toute la chaîne bilancielle côté vendeur **et**
acheteur. La note Excel est explicite :

> « Les taux d'intégrations ne doivent pas être appliqués sur les 2ELI si
> l'elim a été faite avec le Min des taux d'intégration. »

Conséquence mécanique : la règle d'interco doit s'exécuter au niveau
**converti** (avant consolidation), avec ce coef, et les écritures `2ELI`
générées **ne sont pas re-multipliées** par `pct_integration` à l'étape D — ce
qui suppose un marqueur (la nature `2ELI`) que `step_d` doit ignorer.

**C5 — `variation_pct_integration`**

La feuille *Conv&Integ* exhibe un flux **F90** qui n'existe pas encore dans le
moteur. Il capture le rattrapage de l'à-nouveau quand le `% d'intégration`
change d'une année sur l'autre :

```
F90 = (pct_intégration_N − pct_intégration_N-1) × F00_converti
```

Vérifié sur le Excel : `F90 = (0,85 − 0,80) × 2000 = 100` en N, puis
`F90 = (0,90 − 0,85) × 2750 = 137,5` en N+1. La note Excel confirme :

> « F90 est calculé par règle d'après le flux sur F00 provenant d'à nouveau. »

Implémentation : le coef nécessite un **double JOIN** sur `sat_perimeter`
(année courante + année précédente via `entry_period − 1`). La résolution de la
« période précédente » est à spec (durée variable d'un exercice).

**C6 — `ratio_partner_over_entity_pct`**

La feuille *Interco (inverse)* illustre un **autre mode opératoire** : on
consolide d'abord (× `pct_integration` de l'entité), puis on élimine au niveau
*consolidé*. Pour que le **net** soit égal au `min` des taux, le coef à appliquer
est `p_part.pct_integration / p_ent.pct_integration`. Sur le Excel :
`0,75 / 0,85 = 0,882353` — correspond exactement au `Coef Elim = 0,882353` affiché.

> Ce mode est moins usuel (cf. note Excel *« pas usuel »*) ; son support n'est
> pas bloquant pour le smoke test standard mais permet la parité avec un
> existant qui pratique ainsi.

### 1.2 Ce qui n'est **pas** un coefficient de règle

- **Taux de change** → refusé en [R2](./REGLES_CONSO.md#7-questions-ouvertes)
  (TRANCHÉ) : la conversion FX est native (étape C), jamais dans une règle.
- **Marqueur "ne pas re-consolider les 2ELI"** → c'est un comportement de
  `step_d` (filtre sur `nature = '2ELI'` lorsque le coef était `min_pct`),
  pas un coefficient.

---

## 2. Cas de test — smoke tests

Trois cas indépendants calqués sur les feuilles du Excel. Tous les jeux de
données sont dans [`prototype/rust/data/smoke/`](../prototype/rust/data/smoke/).
Le format CSV est celui de `data/` (cf. [`MODELE_DONNEES.md`](./MODELE_DONNEES.md)).

> Convention du Excel : `D` = Débit (actif/charges), `C` = Crédit
> (passif/produits). Les comptes 467 (D, autres débiteurs) et 468 (C, autres
> créditeurs) sont utilisés comme couple équilibré Bilan.

### 2.1 `conv_integ` — Conversion + variation de périmètre

Couvre les feuilles *Conv&Integ* cas 1, 2, 3 du Excel.

| Cas | Description | Ce qu'il valide |
|---|---|---|
| **1A** | A globale en USD, exercice N (entrée) | Conversion + écarts F80/F81 + clôture F99 |
| **1B** | A proportionnelle (80 % → 85 %) sur 2 exercices | À-nouveau figé au % N-1, F90 de variation |
| **1C** | A proportionnelle (85 % → 90 %), 3ʳᵉ exercice | Report F90 en cascade, écarts F80 qui évoluent |

**Périmètre** (`perimeter.csv`) :

```
perimeter_set,entity,period,methode,pct_interet,pct_integration,entree,sortie
SMOKE_CI,A,2024,globale,1.00,1.00,true,false     # Cas 1A — entrante
SMOKE_CI,A,2024,proportionnelle,1.00,0.80,true,false   # Cas 1B N
SMOKE_CI,A,2025,proportionnelle,1.00,0.85,false,false  # Cas 1B N+1
SMOKE_CI,A,2026,proportionnelle,1.00,0.90,false,false  # Cas 1C N+2
```

**Taux** (`rates.csv`, USD → EUR) :

```
rate_set,currency_source,period,taux_close,taux_moyen
SMOKE_RATES,USD,2023,2.0,         # close N-1 (entrée 2024)
SMOKE_RATES,USD,2024,2.5,2.2      # close N, moyen N (cas 1A/1B)
SMOKE_RATES,USD,2025,2.9,2.6      # close N+1, moyen N+1 (cas 1C)
```

**Résultats attendus** (après pipeline complet, **sans** règle F90 — cas 1A) :

| Niveau | Ligne | F00 | F20 | F80 | F81 | F99 |
|---|---|---|---|---|---|---|
| Converti | 467 D (A, USD→EUR) | 2000 | 220 | 500 | 30 | 2750 |
| Consolidé (×1) | 467 D | 2000 | 220 | 500 | 30 | 2750 |

Avec **règle R-F90** (cas 1B N+1) :

| Ligne | F00 | F20 | F80 | F81 | F90 | F99 |
|---|---|---|---|---|---|---|
| 467 D consolidé | 1600 | 187 | 425 | 25,5 | **100** | 2337,5 |

> `F00 = 1600` = à-nouveau figé au % N (2000 × 0,80), **non ré-appliqué** du
> % N+1. `F90 = 100` = rattrapage = (0,85 − 0,80) × 2000. Sans F90, le F99
> serait erroné (2225 au lieu de 2337,5).

Détails complets (entrées + résultats) dans
[`prototype/rust/data/smoke/conv_integ/expected.md`](../prototype/rust/data/smoke/conv_integ/expected.md).

### 2.2 `interco` — Élimination interco standard

Couvre la feuille *Interco* (cas 3 : A et B proportionnelles, A en USD, année N).

**Périmètre** :

```
perimeter_set,entity,period,methode,pct_interet,pct_integration,entree,sortie
SMOKE_IC,A,2025,proportionnelle,1.00,0.85,true,false
SMOKE_IC,B,2025,proportionnelle,1.00,0.75,true,false
```

→ `min_pct_integration` = `min(0,85 ; 0,75)` = **0,75**.

**Écritures corporate** (A, USD) — une ligne hors-co, une interco :

```
scenario,entity,...,account,flow,currency,nature,partner,...,amount
REEL,A,...,467,F20,USD,0LIASS,,100    # créance standard (D)
REEL,A,...,467,F20,USD,0LIASS,B,100   # créance interco sur B (D)
REEL,A,...,468,F20,USD,0LIASS,,100    # dette standard (C)
```

**Résultat attendu** (à l'équilibre Bilan près) :

| Niveau | Ligne | F20 | F81 | F99 |
|---|---|---|---|---|
| Converti | 467 D / partner=∅ | 220 | 30 | 250 |
| Converti | 467 D / partner=B | 220 | 30 | 250 |
| Converti | 468 C / partner=∅ | 220 | 30 | 250 |
| Converti (règle R2) | 467 / partner=B / **2ELI** | **−165** | **−22,5** | **−187,5** |
| Converti (règle R2) | **471L** / partner=B / 2ELI | **+165** | **+22,5** | **+187,5** |
| Consolidé × pct | 467 D / 0LIAS (×0,85) | 187 | 25,5 | 212,5 |
| Consolidé × pct | 468 C / 0LIAS (×0,85) | 187 | 25,5 | 212,5 |
| Consolidé (2ELI non re-×) | 467 / B / 2ELI | −165 | −22,5 | −187,5 |
| Consolidé (2ELI non re-×) | 471L / B / 2ELI | +165 | +22,5 | +187,5 |

> La nature `2ELI` est **exclue** du `× pct_integration` à l'étape D (sinon
> l'élimination serait trop faible : `0,75 × 0,85 = 0,6375` au lieu de `0,75`).

Détails dans
[`prototype/rust/data/smoke/interco/expected.md`](../prototype/rust/data/smoke/interco/expected.md).

### 2.3 `interco_inverse` — Élimination après consolidation

Couvre la feuille *Interco (inverse)* : intégration **avant** élim, coef
`p_part.pct / p_ent.pct = 0,75 / 0,85 = 0,882353`.

Optionnel — uniquement utile si vous voulez valider la parité avec un système
qui pratique ce mode. Voir
[`prototype/rust/data/smoke/interco_inverse/expected.md`](../prototype/rust/data/smoke/interco_inverse/expected.md).

---

## 3. Règles à implémenter

### 3.1 Traitement natif (déjà présent, à vérifier)

| Mécanisme | Où | Statut |
|---|---|---|
| Agrégation (étape A) | `pipeline::aggregate` | ✅ |
| Conversion F80/F81 via schéma de flux (étape C) | `pipeline::convert` + `v_flow_behavior` | ✅ ([Q32](./QUESTIONS_OUVERTES.md)) |
| Application des méthodes × `pct_integration` (étape D) | `pipeline::consolidate` | ✅ |
| À-nouveau F99→F00 (report au % N-1) | `pipeline::a_nouveau` | ✅ ([Q31](./QUESTIONS_OUVERTES.md)) |
| Reconstruction F99 par `flux_de_report` | `pipeline::materialize_closures` | ✅ |

### 3.2 Règles utilisateur (à écrire dans l'éditeur)

| ID | Règle | Niveau | Scope | Coef | Mult | Sélection | Destination | Coef requis |
|---|---|---|---|---|---|---|---|---|
| **R-F90** | Variation de % sur à-nouveau | consolidé | aucune (toutes entités) | **C5** `variation_pct_integration` | +1 | `flow = F00` | flow → **F90** | 🆕 C5 |
| **R-INT-1** | Extourne interco — partenaire conservé | converti | `entity.methode IN ('globale','proportionnelle')` ET `partner.methode IN ('globale','proportionnelle')` | **C4** `min_pct_integration` | −1 | `partner IS NOT NULL` | nature → `2ELI`, partner → **inherit** | 🆕 C4 |
| **R-INT-2** | Extourne interco — partenaire vidé | converti | idem | **C4** `min_pct_integration` | −1 | `partner IS NOT NULL` | nature → `2ELI`, partner → **null** | 🆕 C4 |
| **R-INT-3** | Contrepartie sur compte liaison — partenaire conservé | converti | idem | **C4** `min_pct_integration` | +1 | `partner IS NOT NULL` | account → **471L** (map `comportement.compte_liaison`), nature → `2ELI`, partner → **inherit** | 🆕 C4 |
| **R-INT-4** | Contrepartie sur compte liaison — partenaire vidé | converti | idem | **C4** `min_pct_integration` | +1 | `partner IS NOT NULL` | account → **471L**, nature → `2ELI`, partner → **null** | 🆕 C4 |

> L'élimination est calquée sur [`REGLES_CONSO.md` §6](./REGLES_CONSO.md#6-exemple--élimination-interco)
> mais avec deux ajustements tirés du Excel :
>
> 1. **Niveau = `converti`** (et non `consolidé`) — l'élimination se fait
>    **avant** l'application du `%`, ce qui exige le coef `min` (C4) ;
> 2. **Coef = `min_pct_integration`** au lieu de `pct_integration`, sinon
>    l'élim est trop faible en cas de déséquilibre de `%` entre les deux entités.

### 3.3 Comportement concomitant à ajouter au moteur (étape D)

**Filtre « ne pas re-consolider les 2ELI »** dans `pipeline::consolidate` :
lorsque `step_d` applique `× pct_integration`, il doit **exclure** les natures
d'élimination (`2ELI`) générées par une règle au niveau *converti* — ces écritures
portent déjà le `min` des `%` et ne doivent pas être re-multipliées.

Marqueur retenu : **préfixe `2` de la nature** (cf. [Q29](./QUESTIONS_OUVERTES.md)
staging — `2ELI` est injecté « après reclass »). Le filtre devient :
`WHERE nature NOT LIKE '2%'` dans le `step_d`. À documenter dans
[`FLUX_CONSO.md`](./FLUX_CONSO.md).

### 3.4 Ordre d'implémentation pour que les 3 cas passent

1. **Ajouter C4 (`min_pct_integration`)** à `rules.rs` — débloque l'interco.
2. **Ajouter le filtre `nature NOT LIKE '2%'`** dans `step_d`.
3. **Créer la règle R-INT (1 à 4)** dans l'éditeur → valide le smoke `interco`.
4. **Ajouter F90** dans `dim_flow` + le schéma `BILAN` (taux `close_n`,
   `flux_ecart` vide, `flux_de_report = F99`).
5. **Ajouter C5 (`variation_pct_integration`)** à `rules.rs`.
6. **Créer la règle R-F90** → valide le smoke `conv_integ` cas 1B/1C.
7. *(Optionnel)* C6 + règle interco inverse → valide le smoke `interco_inverse`.

---

## 4. Comment jouer les smoke tests

Les données sont auto-suffisantes (chaque sous-dossier contient toutes les
master data nécessaires). Deux modes :

### 4.1 Via `cargo test` (test Rust dédié, à écrire)

```rust
// Dans tests/smoke.rs (à créer)
#[test]
fn smoke_interco_standard() {
    let con = open_in_memory_with_csv_dir("data/smoke/interco").unwrap();
    run_pipeline_with_hook(&con, "REEL", ruleset = "RS_INTERCO").unwrap();
    // Asserts sur les montants attendus (cf. expected.md)
}
```

### 4.2 Via le serveur (validation manuelle)

```powershell
$env:CONSO_CSV_DIR = "prototype/rust/data/smoke/interco"
$srv = Start-Process -FilePath ".\target\release\conso-server.exe" -PassThru -WindowStyle Hidden -RedirectStandardOutput "$env:TEMP\opencode\conso-server.log"
# ... wait + Invoke-RestMethod sur /api/run + /api/report/balance ...
Stop-Process -Id $srv.Id -Force
```

Voir [`AGENTS.md` §Exécution](../AGENTS.md) pour le pattern complet.

---

## 5. À trancher (à reporter dans QUESTIONS_OUVERTES.md)

| ID | Sujet | Priorité |
|---|---|---|
| Q-SMOKE-1 | Faut-il **exclure `nature LIKE '2%'` du `× pct`** dans `step_d` ? (implicite dans le smoke interco) | TÔT |
| Q-SMOKE-2 | Résolution de la **« période précédente »** pour C5 (exercice décalé, période custom) — stratégie ? | TÔT |
| Q-SMOKE-3 | Le mode interco **inverse** (C6) est-il dans le périmètre MVP ou reporté ? | POST |
| Q-SMOKE-4 | Faut-il exposer `min_pct_integration` et `variation_pct_integration` dans l'UI RulesPage (liste déroulante des coef) ? | TÔT |
