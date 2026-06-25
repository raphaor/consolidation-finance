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
| [`MODELE_DONNEES.md`](./MODELE_DONNEES.md) | Sémantique des champs CSV, dimensions, satellites. |
| [`FLUX_CONSO.md`](./FLUX_CONSO.md) | Catalogue des flux F00–F99, schémas de flux, et leur traitement générateur. |
| [`A_NOUVEAU.md`](./A_NOUVEAU.md) | Spec du report d'ouverture (à-nouveau) : snapshot N-1 figé → F00 N. |
| [`A_NOUVEAU_IMPL.md`](./A_NOUVEAU_IMPL.md) | Notes d'implémentation de l'à-nouveau (runtime confirmé). |
| [`REGLES_CONSO.md`](./REGLES_CONSO.md) | Spécification de l'éditeur de règles de consolidation. |
| [`FORMULES.md`](./FORMULES.md) | Spécification du moteur de formules. **Volets 1 (coefficients) et 2 (indicateurs/KPI) implémentés.** |
| [`FLOW_SCHEME_EXPLICITE.md`](./FLOW_SCHEME_EXPLICITE.md) | Mini-spec : `flow_scheme` sans défaut (Q45), user-driven. |
| [`TECHNIQUE.md`](./TECHNIQUE.md) | Architecture et justifications de la stack. |
| [`RECETTE_PYTHON.md`](./RECETTE_PYTHON.md) | Recette boîte noire : lancement et rôle des 3 scripts Python (`smoke` / `rules` / `golden`). |

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

### `archive/analyses/` — analyses et revues ponctuelles

| Document | Statut |
|---|---|
| `ANALYSE_RECLASSIFICATION_CONVERSION.md` | Décision rendue ([Q26] : reclassification avant conversion) |
| `REVUE_DYNAMISME.md` | Revue post-registre (19/06/2026) ; P3 supersédée par Scénario v2 |
