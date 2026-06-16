// Page « Pipeline » : affiche les 4 étapes de consolidation avec leur
// nombre de lignes, et expose les actions /api/run et /api/reset.

import { useCallback, useEffect, useState } from 'react';
import { api } from '../api';
import type { LevelCount, PipelineCounts } from '../types';
import { formatInt } from '../utils/format';

type RunStatus =
  | { kind: 'idle' }
  | { kind: 'running'; label: string }
  | { kind: 'done' }
  | { kind: 'error'; message: string };

interface Step {
  key: keyof PipelineCounts;
  label: string;
  description: string;
}

const STEPS: Step[] = [
  {
    key: 'corporate',
    label: 'Corporate',
    description: 'Liasses chargées dans le plan de compte du groupe.',
  },
  {
    key: 'reclassified',
    label: 'Reclassified',
    description: 'Après reclassements / retraitements.',
  },
  {
    key: 'converted',
    label: 'Converted',
    description: 'Après conversion multi-devises (taux clôture / moyen).',
  },
  {
    key: 'consolidated',
    label: 'Consolidated',
    description: 'Après agrégation et méthodes de consolidation.',
  },
];

export function PipelinePage() {
  const [counts, setCounts] = useState<LevelCount[]>([]);
  const [result, setResult] = useState<PipelineCounts | null>(null);
  const [status, setStatus] = useState<RunStatus>({ kind: 'idle' });
  const [loadingCounts, setLoadingCounts] = useState(false);

  const loadCounts = useCallback(async () => {
    setLoadingCounts(true);
    try {
      setCounts(await api.levels());
    } catch {
      setCounts([]);
    } finally {
      setLoadingCounts(false);
    }
  }, []);

  useEffect(() => {
    void loadCounts();
  }, [loadCounts]);

  // Récupère le count d'une étape depuis /api/levels (snapshot courant),
  // sinon depuis le dernier résultat d'exécution.
  function countFor(step: Step): number | null {
    if (result && result[step.key] !== undefined) return result[step.key];
    const found = counts.find((c) => c.level === step.key);
    return found ? found.count : null;
  }

  async function run() {
    setStatus({ kind: 'running', label: 'Exécution du pipeline…' });
    setResult(null);
    try {
      const res = await api.run();
      setResult(res);
      setStatus({ kind: 'done' });
      void loadCounts();
    } catch (err) {
      setStatus({
        kind: 'error',
        message: err instanceof Error ? err.message : 'erreur',
      });
    }
  }

  async function reset() {
    setStatus({ kind: 'running', label: 'Reset + réimport…' });
    setResult(null);
    try {
      await api.reset();
      setStatus({ kind: 'done' });
      void loadCounts();
    } catch (err) {
      setStatus({
        kind: 'error',
        message: err instanceof Error ? err.message : 'erreur',
      });
    }
  }

  const busy = status.kind === 'running';

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Pipeline de consolidation</h1>
        <div className="page__actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={run}
            disabled={busy}
          >
            {status.kind === 'running' && status.label.includes('pipeline')
              ? status.label
              : 'Exécuter le pipeline'}
          </button>
          <button
            type="button"
            className="btn btn--danger"
            onClick={reset}
            disabled={busy}
          >
            {status.kind === 'running' && status.label.includes('Reset')
              ? status.label
              : 'Reset + Reimport'}
          </button>
        </div>
      </div>

      <div className={`status status--${status.kind}`}>
        {status.kind === 'idle' && 'En attente.'}
        {status.kind === 'running' && status.label}
        {status.kind === 'done' && 'Terminé.'}
        {status.kind === 'error' && `Erreur : ${status.message}`}
      </div>

      <ol className="steps">
        {STEPS.map((step, idx) => {
          const value = countFor(step);
          return (
            <li key={step.key} className="step">
              <div className="step__index">{idx + 1}</div>
              <div className="step__body">
                <div className="step__head">
                  <span className="step__title">{step.label}</span>
                  <span className="step__count">
                    {value === null
                      ? loadingCounts
                        ? '…'
                        : '—'
                      : formatInt(value)}
                  </span>
                </div>
                <p className="step__desc">{step.description}</p>
              </div>
            </li>
          );
        })}
      </ol>
    </section>
  );
}
