# Stack technique

Annexe de [`EXPRESSION_DE_BESOIN.md`](../EXPRESSION_DE_BESOIN.md) §7.
Proposition technique (à confirmer couche par couche). Le moteur de consolidation en Rust est imposé ; le reste est justifié ici.

---

## 1. Principes directeurs

- **Performance = critère de validation** (volumétrie large : 50+ entités, millions de lignes).
- Workload **analytique (OLAP)**, pas transactionnel : chargement en lot + gros JOIN/GROUP BY pour la conso.
- **Local, mono-utilisateur** pour le prototype (pas de pression d'écriture concurrente).
- **Facile à maintenir** : éviter la complexité spéculative.

## 2. Architecture (monolithe Rust)

```
┌──────────────────────────────────────────┐
│  Navigateur (UI web interactive)         │
└──────────────────┬───────────────────────┘
                   │ HTTP (API JSON + assets)
┌──────────────────▼───────────────────────┐
│  bin `conso-server` (serveur web Rust)   │
│  - expose le moteur + CRUD master data   │
│  - sert le frontend                      │
└──────────────────┬───────────────────────┘
                   │ appelle
┌──────────────────▼───────────────────────┐
│  crate `conso-engine` (bibliothèque)     │
│  - logique de consolidation native       │
│    (agrégation, conversion, méthodes,    │
│     variations de périmètre)             │
│  - orchestration des règles métier       │
└──────────────────┬───────────────────────┘
                   │ SQL
┌──────────────────▼───────────────────────┐
│  DuckDB embarqué (fichier local unique)  │
│  - faits : écritures                     │
│  - master data : dimensions + satellites │
└──────────────────────────────────────────┘
```

> **État réel (implémenté)** : un **seul crate** `conso-engine`, situé dans
> `prototype/rust/` — **pas de workspace Cargo**. Il fournit une bibliothèque (le
> moteur) et plusieurs binaires : `conso-engine` (défaut), `conso-server`
> (serveur Axum), `conso-bench` (benchmark). Les responsabilités restent
> séparées par module, pas par crate. Le découpage `engine`/`server` ci-dessus
> était la proposition initiale ; la réalité est consignée dans
> [`../CLAUDE.md`](../CLAUDE.md) (§Architecture).

**Stockage** : les données consolidées sont persistées à **4 niveaux** (corporate → reclassifié → converti → consolidé), chacun matérialisant l'état des données après une phase d'élaboration. Le niveau *reclassifié* (devise fonctionnelle, après reclassifications de périmètre) est persisté car utile pour l'audit et la re-conversion.

**Traitement** : le moteur enchaîne **4 étapes** en 1:1 avec les niveaux de stockage — agrégation → reclassification (périmètre, devise fonctionnelle) → conversion (multi-devises, F80/F81) → consolidation (méthodes + règles). Détail dans [`FLUX_CONSO.md`](./FLUX_CONSO.md).

## 3. Stockage — DÉCIDÉ

**DuckDB**, embarqué (in-process, pas de serveur), persisté dans un fichier local.

- **Pourquoi** : columnar + exécution vectorisée → ultra-rapide sur les agrégations/JOIN de la conso ; lit le CSV nativement et vite ; SQL accessible ; embarqué = zéro ops pour un POC local.
- **Bindings Rust** : `duckdb-rs` (ou `sqlx` avec backend DuckDB).
- **Schéma** : table des écritures (faits) + tables master data (dimensions + satellites Périmètre/Taux — voir [`MODELE_DONNEES.md`](./MODELE_DONNEES.md)).
- Hors périmètre POC : pas de réplication, pas d'accès concurrent multi-utilisateur.

## 4. Serveur web — DÉCIDÉ

- **Framework** : **Axum** (sur `tokio`). Choix moderne, écosystème Rust actuel. (Actix-web / Rocket seraient équivalents.)
- Expose une **API JSON** pour : CRUD master data, import CSV (liasses + taux), déclenchement de la consolidation, lecture des restitutions (table filtrable, bilan par flux, CR).
- Sert le frontend SPA en **statique**.

## 5. Frontend — DÉCIDÉ

**React + Vite + TanStack Table** (TypeScript, npm). Buildée via npm, servie en statique par le serveur Rust.
- TanStack Table pour les tables filtrables/triées/paginées et le pivot « bilan par flux » (comptes × flux).
- UI cible : écrans CRUD master data, imports CSV, table filtrable sur tous les champs, bilan par flux, compte de résultat, **éditeur de règles de consolidation** (bibliothèque + jeux de règles ordonnés + exécution + rapport — cf. [`REGLES_CONSO.md`](./REGLES_CONSO.md), [Q24](./QUESTIONS_OUVERTES.md) TRANCHÉE).

## 6. Structure de projet (état réel)

```
consolidation-finance/
  prototype/rust/     # crate `conso-engine` — PAS de workspace (Cargo.toml ici)
    Cargo.toml
    src/              # lib (moteur : pipeline, rules, dimensions…)
    src/bin/          # conso-server (Axum), conso-bench (benchmark)
    tests/            # pipeline.rs, rules.rs
  web/                # app React + Vite + TanStack Table (npm)
  docs/               # source de vérité fonctionnelle (+ archive/)
  prototype/python/   # prototype d'origine — référence historique
  simulations/        # scripts exploratoires Python
```

Le binaire `conso-server` est celui à lancer pour le POC : il démarre Axum, ouvre DuckDB, et sert l'app React buildée. (Le découpage `engine`/`server` en crates séparées, et le workspace racine, étaient la proposition initiale — non retenue.)

## 7. Récapitulatif stack

| Couche | Techno | Justification |
|---|---|---|
| Moteur de conso | **Rust** (`engine`) | Perf + logique métier native (imposé) |
| Stockage | **DuckDB** embarqué | Analytique columnar, perf sur gros volumes, local |
| Serveur web | **Axum** (Rust) | API JSON + static hosting, écosystème actuel |
| Frontend | **React + Vite + TanStack Table** (TS) | Tables/pivots riches, écosystème data-table |

## 8. Build & run — à définir à l'implémentation

- Rust toolchain + npm (côté web).
- Gestion de la base DuckDB (fichier local), migrations du schéma, packaging du binaire : à préciser au moment de coder.
