// Palette d'opérandes insérables au clic, partagée par les éditeurs de
// formules (coefficients & indicateurs). Liste triée par token, filtrable par
// une recherche (token ou libellé). Le clic insère `[token]` via `onPick`.
//
// Composant d'appoint (pas de logique métier) : la résolution/preview reste
// côté page. Pensé pour rester lisible quand le catalogue grandit.

import { useMemo, useState } from 'react';

interface OperandLike {
  token: string;
  label: string;
}

export function OperandPalette({
  title,
  hint,
  operands,
  disabled,
  onPick,
}: {
  title: string;
  hint?: string;
  operands: OperandLike[];
  disabled?: boolean;
  onPick: (token: string) => void;
}) {
  const [q, setQ] = useState('');

  const filtered = useMemo<OperandLike[]>(() => {
    const query = q.trim().toLowerCase();
    const list =
      query === ''
        ? operands
        : operands.filter(
            (o) => o.token.toLowerCase().includes(query) || o.label.toLowerCase().includes(query),
          );
    return [...list].sort((a, b) => a.token.localeCompare(b.token));
  }, [operands, q]);

  return (
    <div className="operand-palette">
      <div className="operand-palette__head">
        <h3 className="operand-palette__title">{title}</h3>
        {hint && <p className="muted operand-palette__hint">{hint}</p>}
        <input
          type="text"
          className="operand-palette__search"
          placeholder="Rechercher un opérande…"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          disabled={disabled}
        />
      </div>
      <div className="operand-palette__chips">
        {filtered.length === 0 && (
          <span className="muted">
            {operands.length === 0 ? 'Aucun opérande — créez-en d’abord.' : 'Aucun opérande correspondant.'}
          </span>
        )}
        {filtered.map((o) => (
          <button
            key={o.token}
            type="button"
            className="chip"
            disabled={disabled}
            title={`${o.token}${o.label ? ` — ${o.label}` : ''}`}
            onClick={() => onPick(o.token)}
          >
            <span className="chip__token">{o.token}</span>
            {o.label ? <span className="chip__label">· {o.label}</span> : null}
          </button>
        ))}
      </div>
    </div>
  );
}
