// Page « Dimensions » (groupe Référentiel) — registre des axes de la table de
// faits. Liste toutes les dimensions (built-in + custom), indique pour chacune
// d'où viennent ses valeurs (référentiel emprunté, ou « libre »), et permet de
// créer / supprimer les dimensions custom.
//
// Déplacée ici depuis l'ancien sous-onglet « Dimensions » de la page Règles :
// une dimension relève du référentiel, pas de la logique de calcul (cf. typologie
// Dimensions / Attributs / Paramétrage).
//
// La colonne « Valeurs depuis » est dérivée du graphe de références serveur
// (`GET /api/meta/references`) : c'est ce qui révèle que `partner` / `share`
// empruntent les valeurs de `entity`.

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
import type { DimensionInfo, ReferenceInfo } from '../types';

type Notice = { kind: 'success' | 'error'; text: string } | null;

// Dimension enrichie de sa source de valeurs (référentiel) pour l'affichage.
interface DimRow extends DimensionInfo {
  // Table master data dont la dimension tire ses valeurs (`entities`,
  // `accounts`…), ou null si dimension libre (saisie texte : analysis, customs).
  source: string | null;
}

export function DimensionsPage() {
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  // Map colonne d'écriture → table référentiel, dérivée du graphe de références.
  const [sources, setSources] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');
  const [newLabel, setNewLabel] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [rows, refs] = await Promise.all([api.dimensions.list(), api.references()]);
      setDims(rows);
      // Référentiel d'une dimension = la cible de sa référence depuis `stg_entry`.
      const map: Record<string, string> = {};
      for (const r of refs as ReferenceInfo[]) {
        if (r.table === 'stg_entry') map[r.column] = r.target_table;
      }
      setSources(map);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setDims([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    setNotice(null);
    void load();
  }, [load]);

  const handleCreate = useCallback(
    async (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      setCreating(true);
      try {
        await api.dimensions.create({ name: newName, label: newLabel });
        setNotice({ kind: 'success', text: `Dimension « ${newName} » créée.` });
        setNewName('');
        setNewLabel('');
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      } finally {
        setCreating(false);
      }
    },
    [newName, newLabel, load],
  );

  const handleDelete = useCallback(
    async (name: string) => {
      if (!window.confirm(`Supprimer la dimension « ${name} » ?`)) return;
      try {
        await api.dimensions.remove(name);
        setNotice({ kind: 'success', text: `Dimension « ${name} » supprimée.` });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [load],
  );

  const data = useMemo<DimRow[]>(
    () => dims.map((d) => ({ ...d, source: sources[d.name] ?? null })),
    [dims, sources],
  );

  const columns = useMemo<RTColumnDef<DimRow>[]>(
    () => [
      { header: 'Nom technique', accessorKey: 'name' },
      { header: 'Libellé', accessorKey: 'label' },
      { header: 'Catégorie', accessorKey: 'category' },
      {
        header: 'Valeurs depuis',
        accessorKey: 'source',
        cell: (info) => {
          const src = info.getValue() as string | null;
          return src ? src : <span className="muted">libre (texte)</span>;
        },
      },
      {
        header: 'Perso.',
        accessorKey: 'custom',
        cell: (info) => (info.getValue() ? 'oui' : 'non'),
      },
      {
        header: 'Pilotable',
        accessorKey: 'pilotable',
        cell: (info) => (info.getValue() ? 'oui' : 'non'),
      },
      {
        id: '__actions',
        header: 'Actions',
        enableSorting: false,
        cell: (info) => {
          const dim = info.row.original;
          if (!dim.custom) {
            return <span className="muted" title="Dimension built-in verrouillée">—</span>;
          }
          return (
            <button
              type="button"
              className="btn btn--sm btn--danger"
              onClick={() => void handleDelete(dim.name)}
            >
              Supprimer
            </button>
          );
        },
      },
    ],
    [handleDelete],
  );

  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Dimensions</h1>
        <div className="page__actions">
          <button type="button" className="btn" onClick={load} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      <p className="page__meta">
        Les axes de la table de faits. « Valeurs depuis » indique le référentiel dont la dimension
        tire ses valeurs (ex. <strong>partner</strong> / <strong>share</strong> empruntent{' '}
        <strong>entities</strong>) ; « libre » = saisie texte. Les dimensions built-in sont
        verrouillées ; les dimensions custom (catégorie Analytical) sont créables/supprimables
        ci-dessous.
      </p>

      {error && <div className="alert alert--error">Erreur : {error}</div>}
      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {table.getHeaderGroups().map((hg) => (
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
                          {flexRender(header.column.columnDef.header, header.getContext())}
                          <span className="th-sort__mark">
                            {sorted === 'asc' ? '▲' : sorted === 'desc' ? '▼' : canSort ? '↕' : ''}
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
                  {loading ? 'Chargement…' : 'Aucune dimension.'}
                </td>
              </tr>
            )}
            {table.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id}>{flexRender(cell.column.columnDef.cell, cell.getContext())}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="rule-section" style={{ marginTop: 24 }}>
        <h3 className="rule-section__title">Ajouter une dimension</h3>
        <p className="rule-section__hint">
          Crée une dimension <strong>libre</strong> (colonne texte, catégorie Analytical). Lui faire
          emprunter une autre dimension, lui donner sa propre table de valeurs ou des attributs sera
          proposé dans une prochaine évolution.
        </p>
        <form className="form-grid" onSubmit={handleCreate}>
          <label className="field">
            <span>Nom technique •</span>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="ex : segment"
              pattern="[A-Za-z_][A-Za-z0-9_]{0,49}"
              title="Lettre ou _ en premier, puis alphanumérique / _ (max 50). Réservés : level, amount, id."
              required
            />
          </label>
          <label className="field">
            <span>Libellé</span>
            <input
              type="text"
              value={newLabel}
              onChange={(e) => setNewLabel(e.target.value)}
              placeholder="ex : Segment produit"
              required
            />
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn--primary" disabled={creating}>
              {creating ? 'Création…' : 'Créer la dimension'}
            </button>
          </div>
        </form>
      </div>
    </section>
  );
}
