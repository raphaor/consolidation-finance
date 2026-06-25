// Page « Exécution » (groupe Consolidation) : sélection d'une consolidation,
// affichage de ses paramètres en lecture seule, et exécution du pipeline
// (POST /api/run). Les opérations destructives sur le magasin (reset, import de
// paquet) vivent désormais dans Référentiel › Maintenance.
//
// La sélection alimente `api.run(consolidationId)`. Sans sélection, le serveur
// choisit la première consolidation `'ouvert'` (rétro-compatibilité dev).

import { useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import type {
  ConsolidationSummary,
  LevelCount,
  PipelineCounts,
  PipelineRunResult,
} from '../types';
import { formatInt, formatOptionLabel } from '../utils/format';

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
    description: 'Liasses chargées dans le plan de compte du groupe (+ report d\'à-nouveau).',
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

// Lignes paramètres affichées en lecture seule sous le dropdown consolidation.
function paramRows(s: ConsolidationSummary): { label: string; value: string }[] {
  const dash = (x: string | null) => (x === null || x === '' ? '—' : x);
  return [
    { label: 'Phase', value: dash(s.phase) },
    { label: 'Exercice', value: dash(s.exercice) },
    { label: 'Devise présentation', value: dash(s.presentation_currency) },
    { label: 'Variante', value: dash(s.variant) },
    { label: 'Jeu de périmètre', value: dash(s.perimeter_set) },
    { label: 'Période de périmètre', value: dash(s.perimeter_period) },
    { label: 'Jeu de taux', value: dash(s.rate_set) },
    { label: 'Période des taux', value: dash(s.rate_period) },
    { label: 'Ruleset', value: dash(s.ruleset_code) },
    { label: 'Conso d\'à-nouveau', value: s.a_nouveau_consolidation_id != null ? String(s.a_nouveau_consolidation_id) : '—' },
    { label: 'Statut', value: dash(s.statut) },
  ];
}

export function ExecutionPage() {
  const [counts, setCounts] = useState<LevelCount[]>([]);
  const [result, setResult] = useState<PipelineRunResult | null>(null);
  const [status, setStatus] = useState<RunStatus>({ kind: 'idle' });
  const [loadingCounts, setLoadingCounts] = useState(false);

  const [consolidations, setConsolidations] = useState<ConsolidationSummary[]>([]);
  const [consolidationId, setConsolidationId] = useState<number | undefined>(undefined);

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

  // Chargement initial : compteurs + liste consolidations. Si aucune
  // consolidation n'est sélectionnée, on pré-sélectionne la première `'ouvert'`
  // (même défaut que le serveur) pour que l'utilisateur voie immédiatement les
  // paramètres.
  useEffect(() => {
    void loadCounts();
    void (async () => {
      try {
        const list = [...(await api.consolidations.list())].sort((a, b) =>
          a.libelle.localeCompare(b.libelle),
        );
        setConsolidations(list);
        if (list.length > 0) {
          const ouvert = list.find((c) => c.statut === 'ouvert');
          setConsolidationId((ouvert ?? list[0]).id);
        }
      } catch {
        setConsolidations([]);
      }
    })();
  }, [loadCounts]);

  const selected = useMemo(
    () => consolidations.find((c) => c.id === consolidationId) ?? null,
    [consolidations, consolidationId],
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
      const res = await api.run(consolidationId);
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

  const busy = status.kind === 'running';

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Exécution de la consolidation</h1>
        <div className="page__actions">
          <button
            type="button"
            className="btn btn--primary"
            onClick={run}
            disabled={busy || consolidations.length === 0}
            title={consolidations.length === 0 ? 'Aucune consolidation disponible' : undefined}
          >
            {busy ? 'Exécution du pipeline…' : 'Lancer la consolidation'}
          </button>
        </div>
      </div>

      <div className="scenario-picker">
        <label className="scenario-picker__label" htmlFor="consolidation-select">
          Définition de consolidation
        </label>
        <select
          id="consolidation-select"
          className="scenario-picker__select"
          value={consolidationId ?? ''}
          onChange={(e) =>
            setConsolidationId(e.target.value === '' ? undefined : Number(e.target.value))
          }
          disabled={busy || consolidations.length === 0}
        >
          {consolidations.length === 0 && <option value="">—</option>}
          {consolidations.map((c) => (
            <option key={c.id} value={c.id}>
              {formatOptionLabel(String(c.id), c.libelle)}
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

      {/* Avertissements de cohérence de l'à-nouveau (non bloquants) remontés par
          /api/run, affichés ici plutôt qu'en console serveur. */}
      {result && result.a_nouveau_warnings.length > 0 && (
        <div className="alert alert--warning" style={{ marginTop: 12 }}>
          <strong>
            ⚠ À-nouveau : {result.a_nouveau_warnings.length} incohérence(s) de
            périmètre (non bloquant)
          </strong>
          <ul style={{ margin: '6px 0 0', paddingLeft: 20 }}>
            {result.a_nouveau_warnings.map((w, i) => (
              <li key={`${w.kind}-${w.entity}-${i}`}>
                {w.entity && <code>{w.entity}</code>} {w.detail}{' '}
                <span className="muted">[{w.kind}]</span>
              </li>
            ))}
          </ul>
        </div>
      )}

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
