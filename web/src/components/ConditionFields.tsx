// Briques d'UI partagées pour éditer une condition de sélection
// (dim / traverser / opérateur / valeur). Utilisées par l'éditeur de règles et
// par les postes d'indicateurs. Les constantes (OPS, OP_SYMBOL…) vivent dans
// `operators.ts`.

import { useEffect, useRef, useState } from 'react';
import { OP_LABEL, OP_SYMBOL, OPS } from './operators';
import type { Characteristic, CustomReference, NativeEnum, SelectionCond } from '../types';
import { useDimValues, useSelectionValues, type DimValue } from '../hooks/useDimValues';
import { formatCondVal } from '../utils/conditionValue';

/// Sélecteur d'opérateur compact : affiche le symbole (=, ≠, ∈, ∅…), conserve
/// la chaîne d'origine en valeur. Rendu avec son `<label className="field">`
/// pour s'aligner dans `.rule-condition` ; modificateur `field--op` = largeur
/// réduite (cf. App.css).
export function OpSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (op: string) => void;
}) {
  return (
    <label className="field field--op">
      <span>Op</span>
      <select
        className="op-select"
        value={value}
        title={OP_LABEL[value] ?? value}
        onChange={(e) => onChange(e.target.value)}
      >
        {OPS.map((o) => (
          <option key={o} value={o} title={OP_LABEL[o]}>
            {OP_SYMBOL[o] ?? o}
          </option>
        ))}
      </select>
    </label>
  );
}

/// Sélecteur de valeur(s) repliable, unifié pour les choix uniques et `IN`.
/// Bouton qui affiche la (les) valeur(s) en **pastilles** et ouvre un panel
/// absolu ; fermeture au clic extérieur. Le panel liste `code` + `libellé` sur
/// une ligne, l'état sélectionné par un fond teinté + ✓ (pas de checkbox
/// encombrante). En mode simple (`multiple=false`), choisir une valeur referme.
export function ValueDropdown({
  options,
  selected,
  onChange,
  multiple = false,
  loading,
}: {
  options: DimValue[];
  selected: string[]; // codes ; en mode simple, longueur ≤ 1
  onChange: (next: string[]) => void;
  multiple?: boolean;
  loading?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Fermeture au clic extérieur (listener global tant que le panel est ouvert).
  useEffect(() => {
    if (!open) return;
    function handler(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const pick = (code: string) => {
    if (multiple) {
      if (selected.includes(code)) onChange(selected.filter((c) => c !== code));
      else onChange([...selected, code]);
    } else {
      onChange([code]);
      setOpen(false);
    }
  };

  return (
    <div className={`multiselect${multiple ? '' : ' multiselect--single'}`} ref={ref}>
      <button
        type="button"
        className="multiselect__btn"
        onClick={() => setOpen((o) => !o)}
        disabled={loading}
      >
        {loading ? (
          '…'
        ) : selected.length === 0 ? (
          <span className="multiselect__placeholder">— choisir —</span>
        ) : (
          <span className="multiselect__chips">
            {selected.map((code) => (
              <span key={code} className="dim-chip">
                {code}
              </span>
            ))}
          </span>
        )}
        <span className="multiselect__caret" aria-hidden="true">
          ▾
        </span>
      </button>
      {open && (
        <div className="multiselect__panel">
          {options.length === 0 && (
            <div className="multiselect__empty">(aucune valeur)</div>
          )}
          {options.map((o) => {
            const checked = selected.includes(o.code);
            return (
              <button
                type="button"
                key={o.code}
                className={`multiselect__item${checked ? ' is-selected' : ''}`}
                onClick={() => pick(o.code)}
              >
                <span className="multiselect__code">{o.code}</span>
                <span className="multiselect__lib">{o.libelle ?? ''}</span>
                <span className="multiselect__tick" aria-hidden="true">
                  {checked ? '✓' : ''}
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

/// Champ « Traverser » repliable : par défaut une simple icône ↳ (gain de
/// place, la traversée étant l'exception) ; se déplie en `<select>` au clic, ou
/// d'emblée si la condition utilise déjà une traversée (`value ≠ ''`). La valeur
/// est de la forme `''` (direct) | `via:<code>` | `ref:<col>` | `attr:<col>`.
export function TraverseField({
  value,
  disabled,
  viaOptions,
  refOptions,
  attrOptions,
  onSelect,
}: {
  value: string;
  disabled: boolean;
  viaOptions: Characteristic[];
  refOptions: CustomReference[];
  attrOptions: NativeEnum[];
  onSelect: (v: string) => void;
}) {
  const [expanded, setExpanded] = useState(value !== '');
  const hasOptions =
    viaOptions.length > 0 || refOptions.length > 0 || attrOptions.length > 0;

  if (!expanded) {
    return (
      <label className="field field--traverse">
        <span>Traverser</span>
        <button
          type="button"
          className="rule-traverse-btn"
          disabled={disabled || !hasOptions}
          title={
            hasOptions
              ? 'Filtrer par attribut : caractéristique N1 ou référence directe'
              : 'Aucune traversée disponible pour cette dimension'
          }
          onClick={() => setExpanded(true)}
        >
          ↳
        </button>
      </label>
    );
  }

  return (
    <label className="field">
      <span>Traverser</span>
      <select
        value={value}
        disabled={disabled}
        onChange={(e) => {
          const v = e.target.value;
          if (v === '') setExpanded(false);
          onSelect(v);
        }}
      >
        <option value="">(direct)</option>
        {viaOptions.length > 0 && (
          <optgroup label="Caractéristique N1">
            {viaOptions.map((c) => (
              <option key={`via:${c.code}`} value={`via:${c.code}`}>
                {c.code}
              </option>
            ))}
          </optgroup>
        )}
        {refOptions.length > 0 && (
          <optgroup label="Référence directe">
            {refOptions.map((r) => (
              <option key={`ref:${r.column}`} value={`ref:${r.column}`}>
                {r.column}
                {r.native ? ' (natif)' : ''}
              </option>
            ))}
          </optgroup>
        )}
        {attrOptions.length > 0 && (
          <optgroup label="Attribut natif">
            {attrOptions.map((e) => (
              <option key={`attr:${e.column}`} value={`attr:${e.column}`}>
                {e.column}
              </option>
            ))}
          </optgroup>
        )}
      </select>
    </label>
  );
}

// Réutilise une valeur `IN` (tableau ou chaîne « a,b ») en tableau de codes,
// pour alimenter un `ValueDropdown` multiple. Le moteur accepte les deux
// formes ; on normalise côté affichage.
function toInArray(value: unknown): string[] {
  if (Array.isArray(value)) return value.filter((v): v is string => typeof v === 'string');
  if (typeof value === 'string' && value !== '') {
    return value
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
  }
  return [];
}

/// Champ de saisie pour l'opérateur `IN` en saisie libre (dimensions sans
/// master data). Maintient un **buffer texte local** : on ne reformate PAS
/// depuis la valeur parsée à chaque frappe, sinon la virgule en cours de saisie
/// serait supprimée immédiatement. Le texte brut est conservé ; le parent
/// reçoit la chaîne via `onRawChange` (à parser avec `parseCondVal` au save).
export function InValueField({
  value,
  onRawChange,
}: {
  value: unknown;
  onRawChange: (raw: string) => void;
}) {
  const [text, setText] = useState(() => formatCondVal(value));
  return (
    <input
      type="text"
      value={text}
      placeholder="valeurs séparées par virgules"
      onChange={(e) => {
        setText(e.target.value);
        onRawChange(e.target.value);
      }}
    />
  );
}

/// Champ de saisie de valeur de condition, adaptatif :
/// - op = IN → multi-select repliable si la dimension a une master data, sinon
///   `<input type="text">` (saisie multi-valeurs séparées par virgules).
/// - dimension avec master data et op ≠ IN → dropdown à valeur unique.
/// - sinon → saisie libre.
export function ValueField({
  dim,
  op,
  value,
  onRawChange,
}: {
  dim: string;
  op: string;
  value: unknown;
  onRawChange: (raw: string) => void;
}) {
  const { values, loading } = useDimValues(dim);

  if (op === 'IN') {
    if (values.length > 0) {
      return (
        <ValueDropdown
          multiple
          options={values}
          selected={toInArray(value)}
          loading={loading}
          onChange={(newArr) => onRawChange(newArr.join(','))}
        />
      );
    }
    return <InValueField value={value} onRawChange={onRawChange} />;
  }

  if (values.length > 0) {
    const current = formatCondVal(value);
    return (
      <ValueDropdown
        options={values}
        selected={current ? [current] : []}
        loading={loading}
        onChange={(arr) => onRawChange(arr[0] ?? '')}
      />
    );
  }

  return (
    <input
      type="text"
      value={formatCondVal(value)}
      onChange={(e) => onRawChange(e.target.value)}
    />
  );
}

/// Champ de valeur d'une destination `override` : dropdown si la dimension a
/// une master data, sinon saisie libre. Comme `ValueField` mais pour une
/// valeur unique (pas d'opérateur).
export function OverrideValueField({
  dim,
  value,
  onChange,
}: {
  dim: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const { values, loading } = useDimValues(dim);
  if (values.length > 0) {
    return (
      <ValueDropdown
        options={values}
        selected={value ? [value] : []}
        loading={loading}
        onChange={(arr) => onChange(arr[0] ?? '')}
      />
    );
  }
  return (
    <input
      type="text"
      className="rule-dest-input"
      placeholder={`valeur ${dim}`}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

/// Champ de valeur unifié pour une `SelectionCond`, adaptatif selon l'opérateur
/// et la source des valeurs (résolue via `useSelectionValues` : traversée `via`
/// N1, `ref` patron B, `attr` enum natif, ou direct sur la dimension). C'est le
/// champ utilisé par les sélections de règles ET par les postes d'indicateurs.
export function SelectionValueField({
  sel,
  customReferences,
  nativeEnums,
  op,
  value,
  onRawChange,
}: {
  sel: SelectionCond;
  customReferences: CustomReference[];
  nativeEnums: NativeEnum[];
  op: string;
  value: unknown;
  onRawChange: (raw: string) => void;
}) {
  const { values, loading } = useSelectionValues(sel, customReferences, nativeEnums);

  if (op === 'IN') {
    if (values.length > 0) {
      return (
        <ValueDropdown
          multiple
          options={values}
          selected={toInArray(value)}
          loading={loading}
          onChange={(newArr) => onRawChange(newArr.join(','))}
        />
      );
    }
    return <InValueField value={value} onRawChange={onRawChange} />;
  }

  if (values.length > 0) {
    const current = formatCondVal(value);
    return (
      <ValueDropdown
        options={values}
        selected={current ? [current] : []}
        loading={loading}
        onChange={(arr) => onRawChange(arr[0] ?? '')}
      />
    );
  }
  return (
    <input
      type="text"
      value={formatCondVal(value)}
      onChange={(e) => onRawChange(e.target.value)}
    />
  );
}
