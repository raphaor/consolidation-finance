# Index de la documentation

Ce dossier regroupe la **source de vérité fonctionnelle** du projet. Pour les
conventions de travail et l'architecture du moteur, voir [`../AGENTS.md`](../AGENTS.md)
et [`../CLAUDE.md`](../CLAUDE.md).

## Documents vivants (source de vérité)

Tenus à jour avec le code. À lire avant d'implémenter.

| Document | Rôle |
|---|---|
| [`ETAT_AVANCEMENT.md`](./ETAT_AVANCEMENT.md) | **Point d'entrée** : ce qui est fait / partiel / reste à faire, et le comportement de chaque brique. |
| [`../EXPRESSION_DE_BESOIN.md`](../EXPRESSION_DE_BESOIN.md) | Doc principal (volontairement court). Vision, périmètre, MVP. |
| [`QUESTIONS_OUVERTES.md`](./QUESTIONS_OUVERTES.md) | Registre des décisions (priorisé `BLOC`/`TÔT`/`POST`/`HORS`, ID `Qn`). Historique des arbitrages. |
| [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) | Sémantique des champs CSV, dimensions, satellites, attributs dynamiques. |
| [`FLUX_CONSO.md`](./FLUX_CONSO.md) | Catalogue des flux F00–F99, schémas de flux, pipeline (agrégation → conversion → consolidation, 3 niveaux). |
| [`A_NOUVEAU.md`](./A_NOUVEAU.md) | Spec du report d'ouverture (à-nouveau) : snapshot N-1 figé → F00 N. |
| [`REGLES_CONSO.md`](./REGLES_CONSO.md) | Spécification de l'éditeur de règles de consolidation. |
| [`FORMULES.md`](./FORMULES.md) | Spec du moteur de formules (coefficients utilisateur + indicateurs/KPI). |
| [`CONTROLES_DONNEES.md`](./CONTROLES_DONNEES.md) | Spec et plan d'implémentation des contrôles de données (validation configurable, rapports). |
| [`CAS_CONSO_TEST.md`](./CAS_CONSO_TEST.md) | Cahier de recette pour l'éditeur de règles (cas représentatifs). |
| [`TECHNIQUE.md`](./TECHNIQUE.md) | Architecture et justifications de la stack. |
| [`RECETTE_PYTHON.md`](./RECETTE_PYTHON.md) | Recette boîte noire : lancement et rôle des 3 scripts Python (`smoke` / `rules` / `golden`). |
| [`PLAN_Q54_API_MCP.md`](./PLAN_Q54_API_MCP.md) | Plan Q54 en cours : améliorations API REST (bulk, pagination, recherche) + serveur MCP intégré (`--mcp`). À archiver une fois livré. |

### Articulation des specs

```
EXPRESSION_DE_BESOIN.md          ← doc principal (vision, périmètre, MVP)
  ├── QUESTIONS_OUVERTES.md      ← registre des décisions (Qn), historique
  ├── MODELE_DONNEES.md          ← dimensions, satellites, sémantique des champs
  ├── FLUX_CONSO.md              ← flux F00–F99, pipeline 3 niveaux, schémas de flux
  │     └── A_NOUVEAU.md         ← report d'ouverture (annexe de FLUX_CONSO §4)
  ├── REGLES_CONSO.md            ← éditeur de règles (scope, opérations, destination)
  │     └── CAS_CONSO_TEST.md    ← cahier de recette des règles
  ├── FORMULES.md                ← coefficients + indicateurs/KPI
  ├── CONTROLES_DONNEES.md       ← contrôles de données configurables
  └── TECHNIQUE.md               ← stack, architecture, justifications

ETAT_AVANCEMENT.md               ← point d'entrée synthétique (fait / partiel / reste)
```

## Archive (`archive/`)

Documents **historiques**, conservés pour mémoire mais **non maintenus**. Ne pas
s'y fier comme spec courante : ils reflètent l'état au moment de leur rédaction.

### `archive/specs-livrees/` — bons de travail terminés

Specs d'implémentation déjà livrées (le code correspondant existe). Référence
de conception, pas de la spec vivante.

| Spec | Couvre (livré) |
|---|---|
| `SPEC_REGISTRY.md` | Registre central des dimensions + dimensions custom |
| `SPEC_PROPAGATION.md` | Propagation des dimensions optionnelles (+ renommage `audit_id`→`analysis2`) |
| `SPEC_RULES.md` | Moteur de règles de consolidation (Rust) |
| `SPEC_UI_RULES.md` | UI Règles (React/TypeScript) |
| `SPEC_SCENARIO_V2.md` | Scénario composite + taux pivot (devise pivot, rate sets, `app_config`) |
| `SPEC_SCENARIO_V2_TECH.md` | Annexe technique d'implémentation de Scénario v2 |
| `A_NOUVEAU_IMPL.md` | Tracker d'implémentation de l'à-nouveau (Phases 0–7, livré 2026-06-21) |
| `FLOW_SCHEME_EXPLICITE.md` | Mini-spec `flow_scheme` sans défaut (Q45, livré 2026-06-26) |
| `PLAN_MIGRATION_CSV_JSON.md` | Plan de migration seed CSV → JSON (T1–T5, livré) |
| `PLAN_RENOMMAGE_CODES.md` | Plan de renommage codes → clés techniques B1 (étapes 0–13, livré) |

### `archive/analyses/` — analyses et revues ponctuelles

| Document | Statut |
|---|---|
| `ANALYSE_RECLASSIFICATION_CONVERSION.md` | Décision rendue ([Q26] : reclassification avant conversion) |
| `REVUE_DYNAMISME.md` | Revue post-registre (19/06/2026) ; P3 supersédée par Scénario v2 |
| `REFACTOR_CONSO_RESTE_A_FAIRE.md` | Tracker du chantier Q41/Q42 (scenario→consolidation) — **archivé 2026-06-29** |
