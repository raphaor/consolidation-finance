// Surveille le statut de l'API via /api/health (polling toutes les 5 s).

import { useEffect, useState } from 'react';
import { errMsg } from '../utils/errMessage';
import { api } from '../api';

export type HealthState =
  | { kind: 'loading' }
  | { kind: 'ok' }
  | { kind: 'down'; message: string };

export function useHealth(intervalMs = 5000): HealthState {
  const [state, setState] = useState<HealthState>({ kind: 'loading' });

  useEffect(() => {
    const controller = new AbortController();
    let active = true;

    async function check() {
      try {
        const res = await api.health(controller.signal);
        if (!active) return;
        setState(
          res.status === 'ok' ? { kind: 'ok' } : { kind: 'down', message: res.status },
        );
      } catch (err) {
        if (!active) return;
        setState({ kind: 'down', message: errMsg(err, 'erreur') });
      }
    }

    check();
    const id = window.setInterval(check, intervalMs);
    return () => {
      active = false;
      controller.abort();
      window.clearInterval(id);
    };
  }, [intervalMs]);

  return state;
}
