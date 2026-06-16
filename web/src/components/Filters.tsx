import { useEffect, useState } from 'react';
import { api } from '../api';
import type { Entity, Period, Scenario } from '../types';

interface Props {
  scenario: string;
  entity: string;
  entryPeriod: string;
  period: string;
  onScenarioChange: (v: string) => void;
  onEntityChange: (v: string) => void;
  onEntryPeriodChange: (v: string) => void;
  onPeriodChange: (v: string) => void;
  disabled?: boolean;
}

export function Filters({
  scenario,
  entity,
  entryPeriod,
  period,
  onScenarioChange,
  onEntityChange,
  onEntryPeriodChange,
  onPeriodChange,
  disabled,
}: Props) {
  const [scenarios, setScenarios] = useState<Scenario[]>([]);
  const [entities, setEntities] = useState<Entity[]>([]);
  const [periods, setPeriods] = useState<Period[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [s, e, p] = await Promise.all([
          api.masterData.list('scenarios'),
          api.masterData.list('entities'),
          api.masterData.list('periods'),
        ]);
        if (cancelled) return;
        setScenarios(s as Scenario[]);
        setEntities(e as Entity[]);
        setPeriods(p as Period[]);
      } catch {
        if (cancelled) return;
        setScenarios([]);
        setEntities([]);
        setPeriods([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const exercisePeriods = periods.filter((p) => p.type === 'exercice');
  const entryOptions = exercisePeriods.length > 0 ? exercisePeriods : periods;

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
        <span>Entité</span>
        <select
          value={entity}
          onChange={(e) => onEntityChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Toutes</option>
          {entities.map((e) => (
            <option key={e.code} value={e.code}>
              {e.libelle || e.code}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Exercice</span>
        <select
          value={entryPeriod}
          onChange={(e) => onEntryPeriodChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Tous</option>
          {entryOptions.map((p) => (
            <option key={p.code} value={p.code}>
              {p.libelle || p.code}
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
