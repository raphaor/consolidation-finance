// Page « Master data » : CRUD générique sur les tables de référentiel.
// Les colonnes et le formulaire sont générés depuis MASTER_TABLES (types.ts).

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
  DataHealthReport,
  MasterTable,
  TableDef,
} from '../types';
import { MASTER_TABLES } from '../types';
import { FieldInput } from '../components/FieldInput';
import { coerceValue, renderCell, toFormValue } from '../components/masterFields';
import { findReferenceDrift } from '../utils/referenceCheck';

type Row = Record<string, unknown>;
type FormState = { mode: 'create' } | { mode: 'edit'; row: Row } | null;
type Notice = { kind: 'success' | 'error'; text: string } | null;

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

export function MasterDataPage() {
  const [table, setTable] = useState<MasterTable>('accounts');
  const tableDef = useMemo(
    () => MASTER_TABLES.find((t) => t.table === table)!,
    [table],
  );

  const [data, setData] = useState<Row[]>([]);
  const [optionsData, setOptionsData] = useState<Record<string, Row[]>>({});
  const [loading, setLoading] = useState(false);
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
      const sourceTables = Array.from(
        new Set(
          tableDef.columns
            .map((c) => c.optionsFrom?.table)
            .filter((t): t is MasterTable => t !== undefined),
        ),
      );
      // Options chargées hors master data (ex. rulesets via /api/rulesets).
      const needsRulesets = tableDef.columns.some((c) => c.optionsApi === 'rulesets');
      const [rows, rulesets, ...optResults] = await Promise.all([
        api.masterData.list(table),
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
    } finally {
      setLoading(false);
    }
  }, [table, tableDef]);

  useEffect(() => {
    setNotice(null);
    setFormState(null);
    void load();
  }, [load]);

  const handleDelete = useCallback(
    async (row: Row) => {
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

  const columns = useMemo<RTColumnDef<Row>[]>(() => {
    const cols: RTColumnDef<Row>[] = tableDef.columns.map((col) => ({
      header: col.label,
      accessorKey: col.name,
      cell: (info) => renderCell(col, info.getValue()),
    }));
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
  }, [tableDef, handleDelete]);

  const tableState = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  async function handleSubmit(values: Record<string, string>) {
    const payload: Record<string, unknown> = {};
    for (const col of tableDef.columns) {
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

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Master data</h1>
        <div className="page__actions">
          <label className="field">
            <span>Table</span>
            <select
              value={table}
              onChange={(e) => setTable(e.target.value as MasterTable)}
              disabled={loading}
            >
              {MASTER_TABLES.map((t) => (
                <option key={t.table} value={t.table}>
                  {t.label}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className="btn btn--primary"
            onClick={() => setFormState({ mode: 'create' })}
            disabled={loading}
          >
            Ajouter
          </button>
          <button type="button" className="btn" onClick={load} disabled={loading}>
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
        {data.length} ligne(s) — <span className="muted">• = clé primaire</span>
      </div>

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
                <td className="grid__empty" colSpan={columns.length}>
                  {loading ? 'Chargement…' : 'Aucune ligne.'}
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

      {formState !== null && (
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
