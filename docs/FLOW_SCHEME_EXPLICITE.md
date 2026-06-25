# Mini-spec — `flow_scheme` explicite (sans défaut)

> Statut : **décision prise** (Q45, 2026-06-25) — `flow_scheme` devient 100 %
> user-driven. Cette spec cadre le **sous-choix restant** (comportement d'un
> compte sans schéma) et l'impact sur le golden, avant implémentation.
> Motivation : retirer le dur `COALESCE(…, CASE classe → RESULTAT/BILAN)` de la
> vue `v_flow_behavior` (`schema.rs:225`) — prérequis au flip B1 de `flow_scheme`.

## 1. Décision actée

La vue `v_flow_behavior` ne dérive plus un schéma par défaut de la `classe` du
compte. Jointure directe :

```sql
CREATE VIEW v_flow_behavior AS
SELECT a.code AS account, si.flow, si.taux_conversion,
       si.flux_ecart, si.flux_de_report, si.flux_a_nouveau
FROM dim_account a
JOIN sat_flow_scheme_item si
  ON si.scheme = a.flow_scheme;   -- plus de COALESCE / CASE sur la classe
```

Conséquence : `RESULTAT` / `BILAN` ne sont plus des codes « magiques » référencés
par le moteur — ils redeviennent de simples schémas utilisateur (renommables).

## 2. Sous-choix ouvert : compte sans `flow_scheme`

La levée du défaut fait surgir une question : que faire d'un compte dont
`flow_scheme IS NULL` ?

| Option | Sémantique | Avantage | Inconvénient |
|---|---|---|---|
| **(a) Obligatoire** | validation rejette `flow_scheme NULL` à la saisie/import d'un compte | pas de zone grise ; tout compte a un comportement explicite | rupture de données existantes (comptes NULL à compléter) ; un peu plus contraignant |
| **(b) Toléré, exclu** | `NULL` accepté ; la vue est un `LEFT JOIN` → un compte sans schéma est **exclu** de la conversion et de la reconstruction de clôture | non bloquant pour la saisie | silencieux : un compte oublié disparaît des sorties (piège) |

**Recommandation : (a) obligatoire.** Alignée sur l'invariant « un schéma doit
être complet » (Q32) et sur la philosophie B1/user-driven : pas de comportement
implicite. Le coût (compléter les comptes existants) est ponctuel et lève une
ambiguïté durable. Le `LEFT JOIN` (b) ne se justifie que si un cas métier
réclame des comptes « hors flux » — à défaut, (a).

## 3. Impact golden

Le seed actuel contient des comptes `flow_scheme = NULL` qui héritent du défaut
(`BILAN` ou `RESULTAT` selon la classe). Après implémentation :

- si **(a)** : le seed doit **peupler `flow_scheme`** sur tous les comptes
  (affectation par classe : `bilan`→`BILAN`, `resultat`→`RESULTAT`, `flux`→
  convenu). La sortie consolidée est **inchangée** (même comportement que le
  défaut) → golden **stable** si le mapping est fidèle ;
- si **(b)** : les comptes sans schéma disparaissent → golden **bouge** et doit
  être régénéré puis **relu** (un snapshot qui bouge sans raison métier =
  régression, cf. procédure `UPDATE_SNAPSHOTS`).

## 4. Implémentation (une fois le sous-choix tranché)

1. `schema.rs` : vue `v_flow_behavior` (suppression du `COALESCE`).
2. `seed.rs` / `bench.rs` : peupler `dim_account.flow_scheme` (option a).
3. Si (a) : validation `flow_scheme` obligatoire (master data + import CSV).
4. Puis flip B1 de `flow_scheme` : `ri()`, vue jointe sur id, `sat_flow_scheme_item.scheme` (PK → reconstruction).

## 5. Hors scope

- L'enum `taux_conversion` (`close_n1`/`avg`/`close_n` → colonnes de taux) reste
  en dur dans `convert.rs` : structurelle par nature (mappe vers des colonnes
  physiques de `sat_exchange_rate`), pas un « code master data ». Immuable.
- La `classe` (enum compte) : son rôle « sens » est retiré par Q44, mais
  l'attribut reste (immuable).
