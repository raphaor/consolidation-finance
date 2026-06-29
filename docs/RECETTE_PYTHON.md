# Recette Python (tests boîte noire)

> **OBSOLÈTE** (2026-06-28) — les scripts Python `golden_test.py`,
> `rules_test.py`, `smoke_test.py` et les datasets CSV associés
> (`data_golden/`, `data/smoke/`) ont été supprimés par le chantier
> migration CSV→JSON (cf. [`PLAN_MIGRATION_CSV_JSON.md`](./PLAN_MIGRATION_CSV_JSON.md)).
> La couverture équivalente (non-régression, élimination interco, conversion
> multi-devises) est assurée par les tests Rust : `tests/golden.rs`,
> `tests/rules.rs`, `tests/pipeline.rs`, `tests/a_nouveau.rs`, plus
> `tests/loader.rs` (qui valide désormais `export::import_bundle`). Ce document
> est conservé pour référence historique.

> Trois scripts Python dans [`../prototype/rust/`](../prototype/rust/) démarrant le
> serveur Rust et validaient son comportement par HTTP. Stdlib Python seule
> (`urllib`, `subprocess`, `argparse`, `json`) — pas de dépendance à installer.

## Pré-requis

1. **Binaire** : depuis `prototype/rust/`, compiler le serveur en release :
   ```sh
   cargo build --release --bin conso-server
   ```
   Produit `target/release/conso-server.exe`. Les scripts le cherchent à cet
   emplacement par défaut.
2. **Python 3.10+** (testé sur CPython 3.11+).

## Lancement

Depuis `prototype/rust/` :

```sh
python smoke_test.py     # couverture HTTP large (tous les endpoints)
python rules_test.py     # moteur de règles (élimination interco)
python golden_test.py    # non-régression (56 montants + invariants)
```

**Code de sortie** : `0` = tout passe, `1` = au moins un échec (détail sur stdout).

## Options communes

| Option | Défaut | Rôle |
|---|---|---|
| `--port PORT` | `3000` | Port d'écoute du serveur |
| `--binary PATH` | `target/release/conso-server` | Binaire à lancer (`.exe` ajouté sous Windows) |
| `--csv-dir DIR` | `data` (`smoke`) / `data_golden` (`rules`, `golden`) | Dataset chargé via `CONSO_CSV_DIR`. Relatif au script ou absolu. |
| `--no-server` | off | Ne démarre pas de serveur — pointe vers un serveur déjà lancé sur `--port` (utile pour debugger avec les logs visibles) |

Les scripts démarrent le serveur eux-mêmes (sauf `--no-server`), attendent
`GET /api/health` (60 × 250 ms), exécutent les vérifications, puis envoient
`SIGTERM` (`kill` en fallback) au serveur — cf. `start_server()` dans chaque script.

## Rôle de chaque script

| Script | Dataset | Couvre |
|---|---|---|
| `smoke_test.py` | `data/` | Santé HTTP de tous les endpoints : `health`, `reset`, `run`, `levels`, `bilan`, `compte_resultat`, `entries`, CRUD master data, cas d'erreur. Vérifie aussi la cohérence des comptages par niveau et la présence de F00/F99 dans le bilan. |
| `rules_test.py` | `data_golden/` | Crée une règle d'élimination interco à 4 opérations via l'API, exécute le ruleset, puis vérifie : (1) des lignes `2ELI` sont apparues au niveau `consolidated` ; (2) le solde interco (`partner NOT NULL`) est extourné à 0 ; (3) le bilan agrégé consolidé est inchangé (écritures équilibrées) ; (4) le rapport du ruleset contient 4 lignes générées. |
| `golden_test.py` | `data_golden/` | Non-régression du moteur : 5 entités (M/G/P/E/S), 3 devises (EUR/USD/GBP), 3 méthodes (globale/proportionnelle/équivalence), une sortie de périmètre (S → F98 miroir + F99=0), une nature d'ajustement (`1AJUST`), 3 natures de staging (`2MAN`/`3MAN`/`4MAN`). Compare 56 montants consolidés à un dictionnaire calculé à la main + 10 invariants structurels (exclu du consolidé, F99=Σconstituants, F98=−Σconstituants pour les sortantes, etc.). |

## Anti-blocage

Ces scripts démarrent un serveur en avant-plan. Si tu utilises `--no-server`
pour debug, lance le serveur toi-même en arrière-plan (cf.
[`../AGENTS.md`](../AGENTS.md) §« Exécution et tests » — snippet PowerShell
`Start-Process -PassThru -RedirectStandardOutput`), puis :

```sh
python golden_test.py --no-server --port 3000 --csv-dir data_golden
```

N'oublie pas d'arrêter le serveur toi-même (`Stop-Process -Id $pid -Force`).

## État courant (juin 2026)

- `smoke_test.py` : **vert** sur `data/` (mais contient des assertions
  `reclassified` et `len(levels_dict) == 4` qui casseront quand la suppression
  du niveau `reclassified` sera livrée — cf. [`A_NOUVEAU.md`](./A_NOUVEAU.md) §4).
- `rules_test.py` : opérationnel sous réserve que le moteur de règles reste
  stable.
- `golden_test.py` : **non vert** — la refonte à-nouveau (suppression de
  `reclassified`, redéfinition du staging préfixe 2/3/4) invalide plusieurs
  invariants structurels et valeurs attendues. Le recalibrer est planifié en
  **Phase 7** du tracker [`A_NOUVEAU_IMPL.md`](./A_NOUVEAU_IMPL.md).

Aucune procédure d'intégration continue n'utilise ces scripts aujourd'hui — ils
se lancent à la main depuis `prototype/rust/`.
