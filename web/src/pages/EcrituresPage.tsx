// Page « Écritures » : table TanStack Table filtrable / triable / paginée.
//
// Stratégie : on charge un lot large (10 000 lignes) pour le niveau choisi
// via /api/entries?level=…&limit=…, puis tout le tri / filtrage / pagination
// est fait côté client. Suffisant pour le prototype mono-utilisateur.

import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table';
import { api } from '../api';
import { Filters } from '../components/Filters';
import type { DimensionInfo } from '../types';
import { LEVELS } from '../types';
import { formatAmount, formatInt } from '../utils/format';

// Une écriture = objet générique (colonnes dynamiques : dimensions built-in +
// custom + level + amount), cf. /api/entries.
type EntryRow = Record<string, unknown>;

const PAGE_SIZE = 100;
const FETCH_LIMIT = 10_000;

const ENTRY_LEVELS = ['raw', ...LEVELS] as const;
type EntryLevel = (typeof ENTRY_LEVELS)[number];

export function EcrituresPage() {
  const [level, setLevel] = useState<EntryLevel>('consolidated');
  const [scenario, setScenario] = useState('');
  const [entity, setEntity] = useState('');
  const [entryPeriod, setEntryPeriod] = useState('');
  const [period, setPeriod] = useState('');
  const [nature, setNature] = useState('');
  const [data, setData] = useState<EntryRow[]>([]);
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);

  // Liste des dimensions (built-in + custom) pour construire les colonnes.
  // Les libellés (ex. « Définition de consolidation » pour scenario) viennent
  // du registre serveur.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await api.dimensions.list();
        if (!cancelled) setDims(rows);
      } catch {
        if (!cancelled) setDims([]);
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
      const rows = await api.entries({
        level,
        limit: FETCH_LIMIT,
        offset: 0,
        scenario: scenario || undefined,
        entity: entity || undefined,
        entry_period: entryPeriod || undefined,
        period: period || undefined,
        nature: nature || undefined,
      });
      setData(rows);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setData([]);
    } finally {
      setLoading(false);
    }
  }, [level, scenario, entity, entryPeriod, period, nature]);

  useEffect(() => {
    void load();
  }, [load]);

  // Colonnes dynamiques : une par dimension du registre (built-in + custom),
  // dans l'ordre du registre, puis Level et Amount. Inclut donc toutes les
  // dimensions d'analyse (le « dont ») et les dimensions custom.
  const columns = useMemo<ColumnDef<EntryRow>[]>(() => {
    const dimCols: ColumnDef<EntryRow>[] = dims.map((d) => ({
      header: d.label,
      accessorKey: d.name,
    }));
    return [
      ...dimCols,
      { header: 'Level', accessorKey: 'level' },
      {
        header: 'Amount',
        accessorKey: 'amount',
        cell: (info) => (
          <span className="num">{formatAmount(Number(info.getValue()))}</span>
        ),
        sortingFn: 'alphanumeric',
      },
    ];
  }, [dims]);

  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: { pagination: { pageSize: PAGE_SIZE, pageIndex: 0 } },
  });

  // Filtrage par compte via la column filter de TanStack.
  const accountFilter =
    (table.getColumn('account')?.getFilterValue() as string | undefined) ?? '';

  const pageIndex = table.getState().pagination.pageIndex;
  const pageCount = table.getPageCount();
  const rowCount = table.getFilteredRowModel().rows.length;

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Écritures</h1>
        <div className="page__actions">
          <Filters
            scenario={scenario}
            entity={entity}
            entryPeriod={entryPeriod}
            period={period}
            nature={nature}
            onScenarioChange={setScenario}
            onEntityChange={setEntity}
            onEntryPeriodChange={setEntryPeriod}
            onPeriodChange={setPeriod}
            onNatureChange={setNature}
            disabled={loading}
          />
          <label className="field">
            <span>Niveau</span>
            <select
              value={level}
              onChange={(e) => setLevel(e.target.value as EntryLevel)}
              disabled={loading}
            >
              {ENTRY_LEVELS.map((lvl) => (
                <option key={lvl} value={lvl}>
                  {lvl}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Compte</span>
            <input
              type="text"
              placeholder="ex. 100_"
              value={accountFilter}
              onChange={(e) =>
                table.getColumn('account')?.setFilterValue(e.target.value)
              }
              disabled={loading}
            />
          </label>
          <button type="button" className="btn" onClick={load} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      <div className="page__meta">
        {rowCount} écriture(s) — {formatInt(data.length)} chargées sur ce
        niveau (limit {FETCH_LIMIT}).
      </div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((header) => {
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th
                      key={header.id}
                      className={
                        header.column.id === 'amount' ? 'num' : undefined
                      }
                    >
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
            {table.getRowModel().rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={columns.length}>
                  {loading ? 'Chargement…' : 'Aucune écriture.'}
                </td>
              </tr>
            )}
            {table.getRowModel().rows.map((row) => (
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

      <div className="pager">
        <div className="pager__info">
          Page {pageIndex + 1} / {Math.max(pageCount, 1)}
        </div>
        <div className="pager__buttons">
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => table.setPageIndex(0)}
            disabled={!table.getCanPreviousPage()}
          >
            «
          </button>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => table.previousPage()}
            disabled={!table.getCanPreviousPage()}
          >
            ‹ Préc.
          </button>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => table.nextPage()}
            disabled={!table.getCanNextPage()}
          >
            Suiv. ›
          </button>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => table.setPageIndex(pageCount - 1)}
            disabled={!table.getCanNextPage()}
          >
            »
          </button>
        </div>
      </div>
    </section>
  );
}
