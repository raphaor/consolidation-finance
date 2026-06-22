import { useEffect, useState } from 'react';
import { api } from '../api';
import type { Entity, Nature, Period, Scenario } from '../types';
import { formatOptionLabel } from '../utils/format';

interface Props {
  scenario: string;
  entity: string;
  entryPeriod: string;
  period: string;
  nature: string;
  onScenarioChange: (v: string) => void;
  onEntityChange: (v: string) => void;
  onEntryPeriodChange: (v: string) => void;
  onPeriodChange: (v: string) => void;
  onNatureChange: (v: string) => void;
  disabled?: boolean;
}

export function Filters({
  scenario,
  entity,
  entryPeriod,
  period,
  nature,
  onScenarioChange,
  onEntityChange,
  onEntryPeriodChange,
  onPeriodChange,
  onNatureChange,
  disabled,
}: Props) {
  const [scenarios, setScenarios] = useState<Scenario[]>([]);
  const [entities, setEntities] = useState<Entity[]>([]);
  const [periods, setPeriods] = useState<Period[]>([]);
  const [natures, setNatures] = useState<Nature[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [s, e, p, n] = await Promise.all([
          api.masterData.list('scenarios'),
          api.masterData.list('entities'),
          api.masterData.list('periods'),
          api.masterData.list('natures'),
        ]);
        if (cancelled) return;
        setScenarios(s as Scenario[]);
        setEntities(e as Entity[]);
        setPeriods(p as Period[]);
        setNatures(n as Nature[]);
      } catch {
        if (cancelled) return;
        setScenarios([]);
        setEntities([]);
        setPeriods([]);
        setNatures([]);
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
        <span>Définition de consolidation</span>
        <select
          value={scenario}
          onChange={(e) => onScenarioChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Tous</option>
          {scenarios.map((s) => (
            <option key={s.code} value={s.code}>
              {formatOptionLabel(s.code, s.libelle)}
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
              {formatOptionLabel(e.code, e.libelle)}
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
              {formatOptionLabel(p.code, p.libelle)}
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
              {formatOptionLabel(p.code, p.libelle)}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Nature</span>
        <select
          value={nature}
          onChange={(e) => onNatureChange(e.target.value)}
          disabled={disabled}
        >
          <option value="">Toutes</option>
          {natures.map((n) => (
            <option key={n.code} value={n.code}>
              {formatOptionLabel(n.code, n.libelle)}
            </option>
          ))}
        </select>
      </label>
    </>
  );
}
