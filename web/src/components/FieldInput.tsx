// Input typé partagé pour les formulaires/grilles de master data : select FK
// (optionsFrom), select chargé via API (optionsApi, ex. rulesets), select énuméré
// (options), case à cocher (bool), number, date, texte. Extrait de MasterDataPage
// pour être réutilisé par l'éditeur liste→détail (MasterDetailEditor).

import type { ReactNode } from 'react';
import type { ColumnDef } from '../types';

type Row = Record<string, unknown>;

// Clés value/label des options chargées dynamiquement (`optionsApi`).
// Aujourd'hui seul `rulesets` est branché : { code, libelle }.
const API_OPTION_KEYS: Record<string, { value: string; label: string }> = {
  rulesets: { value: 'code', label: 'libelle' },
};

export function FieldInput({
  col,
  value,
  disabled,
  onChange,
  optionsRows,
  allValues,
}: {
  col: ColumnDef;
  value: string;
  disabled: boolean;
  onChange: (v: string) => void;
  optionsRows?: Row[];
  allValues: Record<string, string>;
}): ReactNode {
  // Options chargées via une API dédiée (ex. rulesets) — même rendu qu'un
  // select FK mais sans table master data sous-jacente.
  if (col.type === 'select' && col.optionsApi) {
    const keys = API_OPTION_KEYS[col.optionsApi] ?? { value: 'code', label: 'libelle' };
    const rows = optionsRows ?? [];
    return (
      <select value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
        {col.nullable && <option value="">—</option>}
        {rows.map((r) => {
          const v = String(r[keys.value] ?? '');
          const l = String(r[keys.label] ?? v);
          // On affiche le code ; le libellé reste accessible en infobulle.
          return (
            <option key={v} value={v} title={l !== v ? l : undefined}>
              {v}
            </option>
          );
        })}
      </select>
    );
  }
  if (col.type === 'select' && col.optionsFrom) {
    const valueKey = col.optionsFrom.value;
    const labelKey = col.optionsFrom.label ?? valueKey;
    const rows = optionsRows ?? [];
    let filtered = rows;
    if (col.optionsFrom.table === 'sous_classes' && col.name === 'sous_classe') {
      const currentClasse = allValues['classe'] ?? '';
      filtered =
        currentClasse === ''
          ? rows
          : rows.filter((r) => String(r['classe'] ?? '') === currentClasse);
    }
    // Placeholder visible quand la valeur courante n'existe pas dans la liste
    // (ex. donnée vide d'un ancien bug) — évite qu'un select non-nullable fasse
    // croire que sa 1ʳᵉ option est sélectionnée alors que la valeur est vide.
    const known = new Set(filtered.map((r) => String(r[valueKey] ?? '')));
    const showPlaceholder = !col.nullable && !known.has(value);
    return (
      <select value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
        {col.nullable && <option value="">—</option>}
        {showPlaceholder && (
          <option value="" disabled>
            — choisir —
          </option>
        )}
        {filtered.map((r) => {
          const v = String(r[valueKey] ?? '');
          const l = String(r[labelKey] ?? v);
          // On affiche le code ; le libellé reste accessible en infobulle.
          return (
            <option key={v} value={v} title={l !== v ? l : undefined}>
              {v}
            </option>
          );
        })}
      </select>
    );
  }
  if (col.type === 'select' && col.options) {
    return (
      <select value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)}>
        {col.nullable && <option value="">—</option>}
        {col.options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }
  if (col.type === 'bool') {
    return (
      <input
        type="checkbox"
        checked={value === 'true'}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked ? 'true' : 'false')}
      />
    );
  }
  const inputType =
    col.type === 'number' ? 'number' : col.type === 'date' ? 'date' : 'text';
  return (
    <input
      type={inputType}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
