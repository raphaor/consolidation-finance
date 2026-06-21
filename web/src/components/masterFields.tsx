// Helpers de champ partagés pour les formulaires/grilles de master data.
// Extraits de MasterDataPage afin d'être réutilisés par l'éditeur liste→détail
// (MasterDetailEditor). Conversion valeur↔formulaire et rendu cellule.
// L'input typé vit dans FieldInput.tsx (composant séparé pour le fast-refresh).

import type { ReactNode } from 'react';
import type { ColumnDef } from '../types';

export function toFormValue(col: ColumnDef, value: unknown): string {
  if (col.type === 'bool') return value === true ? 'true' : 'false';
  if (value === null || value === undefined) return '';
  return String(value);
}

export function coerceValue(col: ColumnDef, raw: string): unknown {
  if (col.type === 'bool') return raw === 'true';
  if (col.type === 'number') {
    if (raw === '') return col.nullable ? null : 0;
    const n = Number(raw);
    return Number.isFinite(n) ? n : col.nullable ? null : 0;
  }
  if (col.nullable && raw === '') return null;
  return raw;
}

export function renderCell(col: ColumnDef, value: unknown): ReactNode {
  if (value === null || value === undefined) return <span className="muted">—</span>;
  if (col.type === 'bool') return value === true ? 'oui' : 'non';
  return String(value);
}
