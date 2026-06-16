// Indicateur visuel de l'état de l'API (point coloré + libellé).

import type { HealthState } from '../hooks/useHealth';

interface Props {
  state: HealthState;
}

export function HealthBadge({ state }: Props) {
  if (state.kind === 'loading') {
    return (
      <span className="health health--loading" title="Vérification en cours">
        <span className="dot" /> API…
      </span>
    );
  }
  if (state.kind === 'ok') {
    return (
      <span className="health health--ok" title="L'API répond">
        <span className="dot" /> API en ligne
      </span>
    );
  }
  return (
    <span className="health health--down" title={state.message}>
      <span className="dot" /> API hors ligne
    </span>
  );
}
