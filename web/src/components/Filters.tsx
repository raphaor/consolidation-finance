// Sélecteurs Scenario + Période réutilisables.
// Charge au mount la liste via /api/md/scenarios et /api/md/periods.
// Une valeur `''` (= option « Tous » / « Toutes ») signifie : pas de filtre.

import { useEffect, useState } from 'react';
import { api } from '../api';
import type { Period, Scenario } from '../types';

interface Props {
  scenario: string;
  period: string;
  onScenarioChange: (value: string) => void;
  onPeriodChange: (value: string) => void;
  disabled?: boolean;
}

export function Filters({
  scenario,
  period,
  onScenarioChange,
  onPeriodChange,
  disabled,
}: Props) {
  const [scenarios, setScenarios] = useState<Scenario[]>([]);
  const [periods, setPeriods] = useState<Period[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [s, p] = await Promise.all([
          api.masterData.list('scenarios'),
          api.masterData.list('periods'),
        ]);
        if (cancelled) return;
        setScenarios(s as Scenario[]);
        setPeriods(p as Period[]);
      } catch {
        if (cancelled) return;
        setScenarios([]);
        setPeriods([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <label className="field">
        <span>Scénario</span>
        <select
          value={scenario}
          onChange={(e) => onScenarioChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Tous</option>
          {scenarios.map((s) => (
            <option key={s.code} value={s.code}>
              {s.libelle || s.code}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Période</span>
        <select
          value={period}
          onChange={(e) => onPeriodChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Toutes</option>
          {periods.map((p) => (
            <option key={p.code} value={p.code}>
              {p.libelle || p.code}
            </option>
          ))}
        </select>
      </label>
    </>
  );
}
