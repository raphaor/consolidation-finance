// Page « Pipeline » : sélection d'un scénario v2, affichage des paramètres
// dépliés en lecture seule, et exécution du pipeline (POST /api/run).
//
// La sélection alimente `api.run(scenario)`. Sans sélection, le serveur
// choisit le premier scénario `'ouvert'` (rétro-compatibilité dev).

import { useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import type { LevelCount, PipelineCounts, ScenarioSummary } from '../types';
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

// Lignes paramètres affichées en lecture seule sous le dropdown scénario.
function paramRows(s: ScenarioSummary): { label: string; value: string }[] {
  return [
    { label: 'Catégorie', value: s.category ?? '—' },
    { label: 'Période d\'entrée', value: s.entry_period ?? '—' },
    { label: 'Devise présentation', value: s.presentation_currency ?? '—' },
    { label: 'Variante', value: s.variant ?? '—' },
    { label: 'Jeu de taux', value: s.rate_set ?? '—' },
    { label: 'Ruleset', value: s.ruleset_code ?? '—' },
    { label: 'Statut', value: s.statut ?? '—' },
  ];
}

export function PipelinePage() {
  const [counts, setCounts] = useState<LevelCount[]>([]);
  const [result, setResult] = useState<PipelineCounts | null>(null);
  const [status, setStatus] = useState<RunStatus>({ kind: 'idle' });
  const [loadingCounts, setLoadingCounts] = useState(false);

  const [scenarios, setScenarios] = useState<ScenarioSummary[]>([]);
  const [scenarioCode, setScenarioCode] = useState<string>('');

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

  // Chargement initial : compteurs + liste scénarios. Si aucun scénario n'est
  // sélectionné, on pré-sélectionne le premier `'ouvert'` (même défaut que le
  // serveur) pour que l'utilisateur voie immédiatement les paramètres.
  useEffect(() => {
    void loadCounts();
    void (async () => {
      try {
        const list = await api.scenarios.list();
        setScenarios(list);
        if (list.length > 0) {
          const ouvert = list.find((s) => s.statut === 'ouvert');
          setScenarioCode((ouvert ?? list[0]).code);
        }
      } catch {
        setScenarios([]);
      }
    })();
  }, [loadCounts]);

  const selected = useMemo(
    () => scenarios.find((s) => s.code === scenarioCode) ?? null,
    [scenarios, scenarioCode],
  );

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
      const res = await api.run(scenarioCode);
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
            disabled={busy || scenarios.length === 0}
            title={scenarios.length === 0 ? 'Aucun scénario disponible' : undefined}
          >
            {status.kind === 'running' && status.label.includes('pipeline')
              ? status.label
              : 'Lancer la consolidation'}
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

      <div className="scenario-picker">
        <label className="scenario-picker__label" htmlFor="scenario-select">
          Scénario
        </label>
        <select
          id="scenario-select"
          className="scenario-picker__select"
          value={scenarioCode}
          onChange={(e) => setScenarioCode(e.target.value)}
          disabled={busy || scenarios.length === 0}
        >
          {scenarios.length === 0 && <option value="">—</option>}
          {scenarios.map((s) => (
            <option key={s.code} value={s.code}>
              {s.libelle && s.libelle.trim() !== '' ? `${s.libelle} (${s.code})` : s.code}
            </option>
          ))}
        </select>
      </div>

      {selected && (
        <dl className="scenario-params">
          {paramRows(selected).map((row) => (
            <div key={row.label} className="scenario-params__row">
              <dt className="scenario-params__key">{row.label}</dt>
              <dd className="scenario-params__val">{row.value}</dd>
            </div>
          ))}
        </dl>
      )}

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
