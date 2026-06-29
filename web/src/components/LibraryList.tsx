// Liste « bibliothèque » à gauche d'un éditeur CRUD : bouton « + Nouveau » puis
// table cliquable (colonnes configurables + colonne d'actions facultative).
// Partagée par Coefficients, Postes, Indicateurs. Le clic sur une cellule de
// données sélectionne la ligne ; la colonne d'actions est exclue de ce clic.

import type { ReactNode } from 'react';

export interface LibraryColumn<T> {
  header: string;
  cell: (item: T) => ReactNode;
  title?: (item: T) => string;
}

export function LibraryList<T>({
  items,
  getKey,
  selected,
  onSelect,
  onNew,
  newLabel,
  columns,
  actions,
  width = 300,
}: {
  items: T[];
  getKey: (item: T) => string;
  selected: string | 'new' | null;
  onSelect: (item: T) => void;
  onNew: () => void;
  newLabel: string;
  columns: LibraryColumn<T>[];
  actions?: (item: T) => ReactNode;
  width?: number;
}) {
  return (
    <div style={{ flex: `0 0 ${width}px` }}>
      <button type="button" className="btn btn--primary" onClick={onNew}>
        {newLabel}
      </button>
      <table className="table" style={{ marginTop: 12 }}>
        <thead>
          <tr>
            {columns.map((c) => (
              <th key={c.header}>{c.header}</th>
            ))}
            {actions && <th />}
          </tr>
        </thead>
        <tbody>
          {items.map((item) => {
            const key = getKey(item);
            return (
              <tr
                key={key}
                className={selected === key ? 'row--selected' : ''}
                style={{ cursor: 'pointer' }}
              >
                {columns.map((c) => (
                  <td key={c.header} onClick={() => onSelect(item)} title={c.title?.(item)}>
                    {c.cell(item)}
                  </td>
                ))}
                {actions && <td>{actions(item)}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
