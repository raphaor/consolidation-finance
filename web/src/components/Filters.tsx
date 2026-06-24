import { useEffect, useState } from 'react';
import { api } from '../api';
import type { ConsolidationSummary, Entity, Nature, Period } from '../types';
import { formatOptionLabel, sortForDisplay } from '../utils/format';

interface Props {
  consolidation: number | undefined;
  entity: string;
  entryPeriod: string;
  period: string;
  nature: string;
  onConsolidationChange: (v: number | undefined) => void;
  // Remonte la liste des consolidations chargée en interne : permet au parent
  // de dériver la phase / l'exercice de la consolidation sélectionnée (utile
  // pour filtrer le niveau raw par `phase=<phase>`).
  onConsolidationsLoaded?: (list: ConsolidationSummary[]) => void;
  onEntityChange: (v: string) => void;
  onEntryPeriodChange: (v: string) => void;
  onPeriodChange: (v: string) => void;
  onNatureChange: (v: string) => void;
  disabled?: boolean;
}

export function Filters({
  consolidation,
  entity,
  entryPeriod,
  period,
  nature,
  onConsolidationChange,
  onConsolidationsLoaded,
  onEntityChange,
  onEntryPeriodChange,
  onPeriodChange,
  onNatureChange,
  disabled,
}: Props) {
  const [consolidations, setConsolidations] = useState<ConsolidationSummary[]>([]);
  const [entities, setEntities] = useState<Entity[]>([]);
  const [periods, setPeriods] = useState<Period[]>([]);
  const [natures, setNatures] = useState<Nature[]>([]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [c, e, p, n] = await Promise.all([
          api.consolidations.list(),
          api.masterData.list('entities'),
          api.masterData.list('periods'),
          api.masterData.list('natures'),
        ]);
        if (cancelled) return;
        setConsolidations(c);
        setEntities(e as Entity[]);
        setPeriods(p as Period[]);
        setNatures(n as Nature[]);
        onConsolidationsLoaded?.(c);
      } catch {
        if (cancelled) return;
        setConsolidations([]);
        setEntities([]);
        setPeriods([]);
        setNatures([]);
        onConsolidationsLoaded?.([]);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const exercisePeriods = periods.filter((p) => p.type === 'exercice');
  const entryOptions = exercisePeriods.length > 0 ? exercisePeriods : periods;

  // Tris alphabétiques pour l'affichage. Consolidations par libellé (l'id n'est
  // pas signifiant) ; les autres par « code - libellé ».
  const sortedConsolidations = sortForDisplay(consolidations, (c) => c.libelle);
  const sortedEntities = sortForDisplay(entities, (e) => formatOptionLabel(e.code, e.libelle));
  const sortedEntryOptions = sortForDisplay(entryOptions, (p) =>
    formatOptionLabel(p.code, p.libelle),
  );
  const sortedPeriods = sortForDisplay(periods, (p) => formatOptionLabel(p.code, p.libelle));
  const sortedNatures = sortForDisplay(natures, (n) => formatOptionLabel(n.code, n.libelle));

  return (
    <>
      <label className="field">
        <span>Définition de consolidation</span>
        <select
          value={consolidation ?? ''}
          onChange={(e) =>
            onConsolidationChange(e.target.value === '' ? undefined : Number(e.target.value))
          }
          disabled={disabled}
        >
          <option value="">Toutes</option>
          {sortedConsolidations.map((c) => (
            <option key={c.id} value={c.id}>
              {formatOptionLabel(String(c.id), c.libelle)}
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
          {sortedEntities.map((e) => (
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
          {sortedEntryOptions.map((p) => (
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
          {sortedPeriods.map((p) => (
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
          {sortedNatures.map((n) => (
            <option key={n.code} value={n.code}>
              {formatOptionLabel(n.code, n.libelle)}
            </option>
          ))}
        </select>
      </label>
    </>
  );
}
