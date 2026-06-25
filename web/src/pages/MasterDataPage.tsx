// Page « Master data » : CRUD générique sur les tables de référentiel.
//
// Les tables et colonnes sont découvertes dynamiquement :
// - `GET /api/md` fournit la liste des tables navigables (natives +
//   `car_<code>` caractéristiques + `lst_<code>` listes de valeurs) ;
// - `GET /api/md/{table}/schema` fournit le schéma complet (colonnes natives +
//   dynamiques avec métadonnées FK).
//
// Pour les tables natives, on fusionne le schéma runtime avec MASTER_TABLES
// (qui porte les labels et types riches : bool, date, options codées en dur
// comme `classe` ou `statut`). Pour les tables dynamiques (`car_*`, `lst_*`),
// tout est construit depuis le schéma (colonnes `code`, `libelle` + attributs
// N2 typés comme FK).

import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from 'react';
import {
  type ColumnDef as RTColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table';
import { api } from '../api';
import type {
  ColumnDef,
  ColumnSchema,
  DataHealthReport,
  MasterTable,
  TableDef,
  TableSchema,
  TableSummary,
} from '../types';
import { MASTER_TABLES } from '../types';
import { FieldInput } from '../components/FieldInput';
import { coerceValue, renderCell, toFormValue } from '../components/masterFields';
import { findReferenceDrift } from '../utils/referenceCheck';

type Row = Record<string, unknown>;
type FormState = { mode: 'create' } | { mode: 'edit'; row: Row } | null;
type Notice = { kind: 'success' | 'error'; text: string } | null;

// Tables natives qui sont des **dimensions de la table de faits** (axes de
// `fact_entry`), par opposition aux **référentiels de paramétrage**. La source de
// vérité est le registre des dimensions (engine `dimensions.rs`) + leurs tables de
// référence (`references.rs`) ; on la reflète ici côté front car le menu Master
// data mêle des concepts purement UI (certaines tables ont une page dédiée).
// Cf. typologie « Dimensions / Attributs / Paramétrage ». Tout ce qui est natif et
// hors de cet ensemble est rangé sous « Référentiels de paramétrage ».
const DIMENSION_TABLES = new Set<string>([
  'scenario_categories', // phase
  'entities', //            entity (+ partner, share)
  'periods', //             period, entry_period
  'accounts', //            account
  'flows', //               flow
  'currencies', //          currency
  'natures', //             nature
]);

// Dimensions empruntant les valeurs d'une autre dimension (pas de table propre) :
// `partner` / `share` partagent le référentiel `entities`. On les expose ici comme
// **vues en lecture** sur la table empruntée, pour les retrouver dans Master data ;
// leur édition se fait dans la table cible (Entités). `table` est l'identifiant de
// vue (distinct), `resolves` la table master data réellement chargée.
const DIMENSION_VIEWS: Record<string, { label: string; resolves: string }> = {
  partner: { label: 'Partenaire (→ Entités)', resolves: 'entities' },
  share: { label: 'Titre (→ Entités)', resolves: 'entities' },
};

// ───────────── Construction runtime d'un TableDef depuis le schéma ─────────────

// Capitalise un identifiant technique pour fallback de libellé : `compte_parent`
// → `Compte parent`, `code` → `Code`.
function capitalize(name: string): string {
  return name
    .split('_')
    .map((p) => (p.length === 0 ? p : p[0].toUpperCase() + p.slice(1)))
    .join(' ');
}

// Convertit une colonne du schéma serveur en ColumnDef front. Les colonnes avec
// FK deviennent un `select` alimenté par la table cible ; les autres sont du
// texte libre (les types riches natifs — bool, number, date — sont fournis par
// MASTER_TABLES pour les tables natives).
function columnSchemaToColumnDef(cs: ColumnSchema): ColumnDef {
  if (cs.fk) {
    return {
      name: cs.name,
      label: capitalize(cs.name),
      type: 'select',
      nullable: !cs.fk.required,
      optionsFrom: { table: cs.fk.table, value: cs.fk.column, label: 'libelle' },
      pk: cs.pk,
    };
  }
  return {
    name: cs.name,
    label: capitalize(cs.name),
    type: cs.pk ? 'text' : 'text',
    nullable: !cs.pk,
    pk: cs.pk,
  };
}

// Construit un `TableDef` runtime en fusionnant :
// - la définition statique MASTER_TABLES si elle existe (labels et types riches
//   pour les tables natives : bool, date, options codées en dur…) ;
// - les colonnes dynamiques du schéma (caractéristiques N1, références directes
//   patron B, attributs N2 des `car_*`) non déjà présentes dans la définition
//   statique.
function buildTableDef(
  schema: TableSchema,
  staticDef: TableDef | undefined,
): TableDef {
  const staticCols = staticDef?.columns ?? [];
  const staticColNames = new Set(staticCols.map((c) => c.name));
  const dynCols: ColumnDef[] = schema.columns
    .filter((cs) => !staticColNames.has(cs.name))
    .map(columnSchemaToColumnDef);
  return {
    table: schema.table,
    label: staticDef?.label ?? schema.label,
    columns: [...staticCols, ...dynCols],
  };
}

function initialValues(
  def: TableDef,
  seed: Row | null,
): Record<string, string> {
  const v: Record<string, string> = {};
  for (const col of def.columns) {
    if (seed !== null && seed[col.name] !== undefined && seed[col.name] !== null) {
      v[col.name] = toFormValue(col, seed[col.name]);
    } else if (col.type === 'select' && col.options && col.options.length > 0) {
      v[col.name] = col.options[0];
    } else if (col.type === 'select' && col.optionsFrom && !col.nullable) {
      // Choix explicite obligatoire : on laisse vide pour forcer la sélection
      // d'une valeur réelle. `FieldInput` affiche alors le placeholder
      // « — choisir — » (et la soumission est bloquée tant que vide, cf.
      // RowForm.submit). On NE pré-remplit PAS sur la 1ʳᵉ valeur de la table
      // source : ça évitait le « vide silencieux » mais introduisait un
      // « mauvais défaut silencieux » (ex. périmètre.period pré-rempli à 2023
      // au lieu de 2024 → règle interco qui ne matchait rien).
      v[col.name] = '';
    } else if (col.type === 'bool') {
      v[col.name] = 'false';
    } else {
      v[col.name] = '';
    }
  }
  return v;
}

interface RowFormProps {
  tableDef: TableDef;
  initial: Row | null;
  optionsData: Record<string, Row[]>;
  onSubmit: (values: Record<string, string>) => Promise<void>;
  onCancel: () => void;
}

function RowForm({
  tableDef,
  initial,
  optionsData,
  onSubmit,
  onCancel,
}: RowFormProps) {
  const isEdit = initial !== null;
  const [values, setValues] = useState<Record<string, string>>(() =>
    initialValues(tableDef, initial),
  );
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  function setField(name: string, val: string) {
    setValues((prev) => ({ ...prev, [name]: val }));
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setFormError(null);

    // Choix explicite obligatoire : tout select `optionsFrom` non-nullable doit
    // avoir une valeur (cf. initialValues — ils partent vides). Bloque la
    // soumission plutôt que d'envoyer une FK vide.
    const missing = tableDef.columns.filter(
      (col) =>
        col.type === 'select' &&
        col.optionsFrom &&
        !col.nullable &&
        (values[col.name] ?? '') === '',
    );
    if (missing.length > 0) {
      setFormError(
        `Champ(s) obligatoire(s) à renseigner : ${missing
          .map((c) => c.label)
          .join(', ')}.`,
      );
      return;
    }

    setSubmitting(true);
    try {
      await onSubmit(values);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : 'erreur');
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          {isEdit ? 'Éditer la ligne' : 'Ajouter une ligne'} — {tableDef.label}
        </div>
        <form className="modal__body" onSubmit={submit}>
          <div className="form-grid">
            {tableDef.columns.map((col) => {
              // Champ auto-généré (ex : consolidations.id) masqué à la création.
              if (!isEdit && col.auto) return null;
              const locked = isEdit && col.pk === true;
              const optSource = col.optionsApi ?? col.optionsFrom?.table;
              const optRows = optSource ? optionsData[optSource] : undefined;
              return (
                <label key={col.name} className="field">
                  <span>
                    {col.label}
                    {col.pk ? ' •' : ''}
                  </span>
                  <FieldInput
                    col={col}
                    value={values[col.name]}
                    disabled={locked}
                    optionsRows={optRows}
                    allValues={values}
                    onChange={(v) => setField(col.name, v)}
                  />
                </label>
              );
            })}
          </div>
          {formError && (
            <div className="alert alert--error" style={{ marginTop: 12 }}>
              {formError}
            </div>
          )}
          <div className="form-actions">
            <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
              Annuler
            </button>
            <button type="submit" className="btn btn--primary" disabled={submitting}>
              {submitting ? 'Enregistrement…' : 'Enregistrer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

interface MasterDataPageProps {
  // Verrouille l'éditeur sur une seule table (dropdown masqué). Sert la page
  // « Définitions » pointée sur `consolidations`.
  fixedTable?: MasterTable;
  // Tables retirées du sélecteur (ex. `consolidations`, éditée ailleurs).
  hideTables?: string[];
  // Titre de page (défaut « Master data »).
  title?: string;
}

export function MasterDataPage({
  fixedTable,
  hideTables,
  title,
}: MasterDataPageProps = {}) {
  const [table, setTable] = useState<MasterTable>(fixedTable ?? 'accounts');
  // Liste des tables navigables servie par `GET /api/md` (natives + car_* + lst_*).
  // Tant qu'elle n'est pas chargée, on retombe sur MASTER_TABLES (labels statiques).
  const [tableList, setTableList] = useState<TableSummary[]>([]);
  // `tableDef` est construit au runtime en fusionnant MASTER_TABLES (si la table
  // est native) et le schéma serveur (`GET /api/md/{table}/schema`). Nul tant que
  // le schéma n'est pas chargé.
  const [tableDef, setTableDef] = useState<TableDef | null>(null);
  const [schema, setSchema] = useState<TableSchema | null>(null);

  const [data, setData] = useState<Row[]>([]);
  const [optionsData, setOptionsData] = useState<Record<string, Row[]>>({});
  const [loading, setLoading] = useState(false);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [formState, setFormState] = useState<FormState>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);
  // Cohérence des optionsFrom front vs graphe de références serveur (autoritaire).
  const [refDrift, setRefDrift] = useState<string[]>([]);
  // Rapport « santé des données » (orphelins), à la demande.
  const [health, setHealth] = useState<DataHealthReport | null>(null);
  const [healthLoading, setHealthLoading] = useState(false);

  const runHealth = useCallback(async () => {
    setHealthLoading(true);
    try {
      const report = await api.dataHealth();
      setHealth(report);
    } catch (err) {
      setNotice({
        kind: 'error',
        text: err instanceof Error ? err.message : 'erreur',
      });
    } finally {
      setHealthLoading(false);
    }
  }, []);

  // Charge une fois la liste des tables navigables au montage. Si le serveur
  // expose de nouvelles tables (caractéristiques/listes créées ailleurs dans
  // l'app), on les découvre ici. On ne force pas le re-chargement au moindre
  // changement de table : un `Rafraîchir` manuel reste possible.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await api.masterData.listTables();
        if (cancelled) return;
        // Les natives d'abord (ordre backend), puis caractéristiques et listes.
        setTableList(list);
      } catch {
        // Serveur obsolète sans endpoint /api/md : on retombe sur MASTER_TABLES.
        if (!cancelled) {
          setTableList(
            MASTER_TABLES.map((t) => ({
              table: t.table,
              label: t.label,
              kind: 'native' as const,
            })),
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const refs = await api.references();
        if (cancelled) return;
        const drift = findReferenceDrift(refs);
        setRefDrift(drift);
        if (drift.length > 0) {
          console.warn('Dérive optionsFrom vs références serveur :', drift);
        }
      } catch {
        // Endpoint indisponible (serveur obsolète) : on ne signale rien.
        if (!cancelled) setRefDrift([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Vue de dimension empruntée (partner/share) : on charge la table cible
      // (entities) tout en gardant `table` comme identifiant de vue.
      const sqlTable = DIMENSION_VIEWS[table]?.resolves ?? table;
      // Schéma runtime + données en parallèle. Le schéma apporte les colonnes
      // dynamiques (caractéristiques + patron B) absentes de MASTER_TABLES.
      const [schemaResult, rows] = await Promise.all([
        api.masterData.schema(sqlTable),
        api.masterData.list(sqlTable),
      ]);
      setSchema(schemaResult);
      const staticDef = MASTER_TABLES.find((t) => t.table === sqlTable);
      const def = buildTableDef(schemaResult, staticDef);
      setTableDef(def);

      // Tri par défaut alphabétique sur la 1ʳᵉ colonne affichée (le plus souvent
      // le code) ; conserve un tri manuel encore applicable lors d'un simple
      // rafraîchissement. L'utilisateur peut re-trier en cliquant les en-têtes.
      const firstCol = def.columns[0]?.name;
      if (firstCol) {
        setSorting((prev) =>
          prev.length > 0 && def.columns.some((c) => c.name === prev[0].id)
            ? prev
            : [{ id: firstCol, desc: false }],
        );
      }

      // Options pour les select basés sur optionsFrom (tables master data).
      // On ne recharge pas les options API dédiées (rulesets) tant qu'aucune
      // colonne n'en dépend.
      const sourceTables = Array.from(
        new Set(
          def.columns
            .map((c) => c.optionsFrom?.table)
            .filter((t): t is string => t !== undefined),
        ),
      );
      const needsRulesets = def.columns.some((c) => c.optionsApi === 'rulesets');
      const [rulesets, ...optResults] = await Promise.all([
        needsRulesets ? api.rulesets.list() : Promise.resolve([]),
        ...sourceTables.map((t) => api.masterData.list(t)),
      ]);
      setData(rows as Row[]);
      const opts: Record<string, Row[]> = {};
      sourceTables.forEach((t, i) => {
        opts[t] = optResults[i] as Row[];
      });
      if (needsRulesets) opts['rulesets'] = rulesets as unknown as Row[];
      setOptionsData(opts);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setData([]);
      setOptionsData({});
      setTableDef(null);
      setSchema(null);
    } finally {
      setLoading(false);
    }
  }, [table]);

  useEffect(() => {
    setNotice(null);
    setFormState(null);
    setSchemaLoading(true);
    void load().finally(() => setSchemaLoading(false));
  }, [load]);

  const handleDelete = useCallback(
    async (row: Row) => {
      if (tableDef === null) return;
      const pk: Record<string, string> = {};
      for (const col of tableDef.columns) {
        if (col.pk) pk[col.name] = String(row[col.name] ?? '');
      }
      if (!window.confirm(`Supprimer cette ligne de « ${tableDef.label} » ?`)) return;
      try {
        await api.masterData.remove(table, pk);
        setNotice({ kind: 'success', text: 'Ligne supprimée.' });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [table, tableDef, load],
  );

  // Vue en lecture seule (dimension empruntée : partner/share → entities) :
  // pas d'ajout / édition / suppression ici — l'édition se fait dans la table cible.
  const view = DIMENSION_VIEWS[table];
  const isView = view !== undefined;

  // Colonne « code » renommable : la PK non auto-générée (ex. `code`,
  // `code_iso`). Absente pour les tables à PK technique auto (ex. consolidations
  // dont l'identité est l'id) → pas de renommage proposé.
  const codeCol = useMemo(
    () => tableDef?.columns.find((c) => c.pk && !c.auto)?.name ?? null,
    [tableDef],
  );

  const handleRename = useCallback(
    async (row: Row) => {
      if (codeCol === null) return;
      const oldCode = String(row[codeCol] ?? '');
      const newCode = window.prompt(
        `Nouveau code pour « ${oldCode} » ?\n\n` +
          `Renommage possible uniquement si plus aucune référence ne pointe vers ce code.`,
        oldCode,
      );
      if (newCode === null || newCode.trim() === '' || newCode === oldCode) return;
      try {
        await api.masterData.rename(table, oldCode, newCode.trim());
        setNotice({ kind: 'success', text: `Code renommé : ${oldCode} → ${newCode.trim()}` });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [table, codeCol, load],
  );

  const columns = useMemo<RTColumnDef<Row>[]>(() => {
    if (tableDef === null) return [];
    const cols: RTColumnDef<Row>[] = tableDef.columns.map((col) => ({
      header: col.label,
      accessorKey: col.name,
      cell: (info) => renderCell(col, info.getValue()),
    }));
    // Vue en lecture seule : pas de colonne d'actions (édition dans la table cible).
    if (isView) return cols;
    cols.push({
      id: '__actions',
      header: 'Actions',
      enableSorting: false,
      cell: (info) => (
        <div className="row-actions">
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => setFormState({ mode: 'edit', row: info.row.original })}
          >
            Éditer
          </button>
          {codeCol !== null && (
            <button
              type="button"
              className="btn btn--sm"
              title="Changer le code de cet objet (si plus aucune référence ne le cite)"
              onClick={() => void handleRename(info.row.original)}
            >
              Renommer
            </button>
          )}
          <button
            type="button"
            className="btn btn--sm btn--danger"
            onClick={() => void handleDelete(info.row.original)}
          >
            Suppr.
          </button>
        </div>
      ),
    });
    return cols;
  }, [tableDef, handleDelete, handleRename, codeCol, isView]);

  const tableState = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  async function handleSubmit(values: Record<string, string>) {
    if (tableDef === null) return;
    const payload: Record<string, unknown> = {};
    for (const col of tableDef.columns) {
      // Clé primaire auto-générée (ex : consolidations.id) : omise à la création
      // (le serveur fait INSERT ... RETURNING id). Présente et verrouillée à l'édition.
      if (formState?.mode === 'create' && col.auto) continue;
      payload[col.name] = coerceValue(col, values[col.name] ?? '');
    }
    if (formState?.mode === 'create') {
      await api.masterData.create(table, payload);
      setNotice({ kind: 'success', text: 'Ligne créée.' });
    } else {
      await api.masterData.update(table, payload);
      setNotice({ kind: 'success', text: 'Ligne mise à jour.' });
    }
    setFormState(null);
    await load();
  }

  // Tables groupées pour le dropdown, selon la typologie : « Dimensions » (axes de
  // faits) puis « Référentiels de paramétrage » (le reste des natives). Les tables
  // dynamiques `car_*` / `lst_*` (kind characteristic/value_list) ne sont plus
  // listées ici : leur foyer unique est la page « Attributs de dimension ». Si
  // l'endpoint /api/md n'a rien renvoyé (serveur obsolète), tableList contient les
  // natives via MASTER_TABLES.
  const groupedTables = useMemo(() => {
    const natives = tableList.filter((t) => t.kind === 'native');
    const dims = natives.filter((t) => DIMENSION_TABLES.has(t.table));
    const refs = natives.filter((t) => !DIMENSION_TABLES.has(t.table));
    // Vues des dimensions empruntées (partner/share) rattachées au groupe Dimensions.
    const views: TableSummary[] = Object.entries(DIMENSION_VIEWS).map(([t, v]) => ({
      table: t,
      label: v.label,
      kind: 'native' as const,
    }));
    const dimItems = [...dims, ...views];
    const groups: { label: string; items: TableSummary[] }[] = [];
    if (dimItems.length > 0) groups.push({ label: 'Dimensions', items: dimItems });
    if (refs.length > 0) {
      groups.push({ label: 'Référentiels de paramétrage', items: refs });
    }
    if (hideTables && hideTables.length > 0) {
      const hidden = new Set(hideTables);
      return groups
        .map((g) => ({ ...g, items: g.items.filter((t) => !hidden.has(t.table)) }))
        .filter((g) => g.items.length > 0);
    }
    return groups;
  }, [tableList, hideTables]);

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">{title ?? 'Master data'}</h1>
        <div className="page__actions">
          {fixedTable === undefined && (
            <label className="field">
              <span>Table</span>
              <select
                value={table}
                onChange={(e) => setTable(e.target.value)}
                disabled={loading || schemaLoading}
              >
                {groupedTables.map((g) => (
                  <optgroup key={g.label} label={g.label}>
                    {g.items.map((t) => (
                      <option key={t.table} value={t.table}>
                        {t.label}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </label>
          )}
          {!isView && (
            <button
              type="button"
              className="btn btn--primary"
              onClick={() => setFormState({ mode: 'create' })}
              disabled={loading || schemaLoading || tableDef === null}
            >
              Ajouter
            </button>
          )}
          <button
            type="button"
            className="btn"
            onClick={load}
            disabled={loading || schemaLoading}
          >
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
          <button
            type="button"
            className="btn"
            onClick={() => void runHealth()}
            disabled={healthLoading}
            title="Vérifie l'intégrité référentielle de toutes les données (orphelins)"
          >
            {healthLoading ? 'Vérification…' : 'Santé des données'}
          </button>
        </div>
      </div>

      <div className="page__meta">
        {tableDef === null
          ? 'Chargement du schéma…'
          : `${data.length} ligne(s) — `}
        {tableDef !== null && (
          <span className="muted">• = clé primaire</span>
        )}
        {tableDef !== null && schema !== null && schema.sql_name !== table && (
          <span className="muted"> (table SQL : {schema.sql_name})</span>
        )}
      </div>

      {view && (
        <div className="alert alert--warning" role="status">
          Vue en lecture seule : « {view.label.split(' (')[0]} » emprunte les valeurs de cette
          table. Pour les modifier, éditez la table cible directement.
        </div>
      )}

      {error && <div className="alert alert--error">Erreur : {error}</div>}
      {notice && (
        <div className={`alert alert--${notice.kind}`}>{notice.text}</div>
      )}
      {refDrift.length > 0 && (
        <div className="alert alert--error" role="alert">
          ⚠ Incohérence config front vs références serveur :
          <ul style={{ margin: '4px 0 0', paddingLeft: 20 }}>
            {refDrift.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </div>
      )}

      {health !== null && (
        <div
          className={`alert alert--${health.ok ? 'success' : 'error'}`}
          role="alert"
        >
          {health.ok ? (
            <>✓ Intégrité référentielle OK — aucun orphelin.</>
          ) : (
            <>
              ⚠ {health.total} valeur(s) orpheline(s) détectée(s) :
              <ul style={{ margin: '4px 0 0', paddingLeft: 20 }}>
                {health.checks.map((c) => (
                  <li key={`${c.table}.${c.column}`}>
                    <strong>
                      {c.table}.{c.column}
                    </strong>{' '}
                    → {c.target_table}.{c.target_column} : {c.count} orphelin(s)
                    {c.sample.length > 0 && (
                      <> ({c.sample.join(', ')}
                        {c.count > c.sample.length ? ', …' : ''})</>
                    )}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {tableState.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((header) => {
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th key={header.id}>
                      {header.isPlaceholder ? null : (
                        <button
                          type="button"
                          className={`th-sort ${canSort ? 'th-sort--sortable' : ''}`}
                          onClick={header.column.getToggleSortingHandler()}
                          disabled={!canSort}
                        >
                          {flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                          <span className="th-sort__mark">
                            {sorted === 'asc'
                              ? '▲'
                              : sorted === 'desc'
                                ? '▼'
                                : canSort
                                  ? '↕'
                                  : ''}
                          </span>
                        </button>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {tableState.getRowModel().rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={Math.max(columns.length, 1)}>
                  {loading || schemaLoading || tableDef === null
                    ? 'Chargement…'
                    : 'Aucune ligne.'}
                </td>
              </tr>
            )}
            {tableState.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {formState !== null && tableDef !== null && (
        <RowForm
          tableDef={tableDef}
          initial={formState.mode === 'edit' ? formState.row : null}
          optionsData={optionsData}
          onSubmit={handleSubmit}
          onCancel={() => setFormState(null)}
        />
      )}
    </section>
  );
}
