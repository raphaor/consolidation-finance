# Smoke tests — Cas de consolidation

Jeux de données calqués sur le fichier `Conso rules/Cas de consolidation.xlsx`.
Document de référence : [`docs/CAS_CONSO_TEST.md`](../../../docs/CAS_CONSO_TEST.md).

Chaque sous-dossier est **auto-suffisant** : il contient toutes les master data
nécessaires pour un run isolated. Les dossiers sont indépendants (pas de
fichiers partagés) — vous pouvez copier le contenu d'un dossier vers
`prototype/rust/data/` (ou pointer `CONSO_CSV_DIR` dessus) pour exécuter le cas.

## Cas disponibles

| Dossier | Cas Excel | Ce qu'il valide | Règles requises |
|---|---|---|---|
| [`conv_integ/`](./conv_integ/) | *Conv&Integ* cas 1A | Conversion USD→EUR, écarts F80/F81, clôture F99 | Aucune (pur natif) |
| [`interco/`](./interco/) | *Interco* cas 3 | Élimination interco standard au niveau *converti* avec `min_pct_integration` | R-INT (4 opérations) |

## Comment jouer un cas

### Via `CONSO_CSV_DIR` (serveur)

```powershell
$env:CONSO_CSV_DIR = "prototype/rust/data/smoke/interco"
$srv = Start-Process -FilePath ".\target\release\conso-server.exe" `
    -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput "$env:TEMP\opencode\conso-server.log"
# … Invoke-RestMethod sur /api/run puis /api/report/balance …
Stop-Process -Id $srv.Id -Force
```

### Via test Rust (à créer)

```rust
let con = open_in_memory_with_csv_dir("data/smoke/interco").unwrap();
run_pipeline(&con, /* … */).unwrap();
// Asserts → voir ./interco/expected.md
```

## État d'avancement

| Cas | Données | Mécanisme | Statut |
|---|---|---|---|
| `conv_integ` cas 1A | ✅ prêt | Natif (conversion) | **Jouable immédiatement** |
| `conv_integ` cas 1B/1C | Documenté dans `expected.md` | R-F90 (coef `variation_pct_integration` à implémenter) | ⏳ bloqué par C5 |
| `interco` | ✅ prêt | R-INT (coef `min_pct_integration` à implémenter) | ⏳ bloqué par C4 + filtre `nature NOT LIKE '2%'` dans `step_d` |
| `interco_inverse` | Non produit (optionnel) | Coef `ratio_partner_over_entity_pct` | ⏸️ post-MVP |

## Note sur les flux F90 et la nature 2ELI

- **F90** (variation de % d'intégration) est inclus dans les `flows.csv` et
  `flow_scheme_items.csv` des smoke tests — mais **pas encore dans le `data/`
  principal**. Il ne sera effectif qu'une fois la règle R-F90 + le coef C5
  implémentés.
- **`2ELI`** (nature d'élimination interco) est déjà dans le `natures.csv`
  principal. Le filtre « ne pas re-consolider les `2*` » dans `step_d` est à
  ajouter (cf. [`docs/CAS_CONSO_TEST.md` §3.3](../../../docs/CAS_CONSO_TEST.md)).
