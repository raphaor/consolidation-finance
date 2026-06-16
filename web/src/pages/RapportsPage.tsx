import { useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import type { BilanRow, Level, ReportFilters } from '../types';
import { FLOW_COLUMNS, LEVELS } from '../types';
import { Filters } from '../components/Filters';
import { buildPivot } from '../utils/pivot';
import { formatAmount } from '../utils/format';

type ReportType = 'bilan' | 'cr';

export function RapportsPage() {
  const [reportType, setReportType] = useState<ReportType>('bilan');
  const [level, setLevel] = useState<Level>('consolidated');
  const [scenario, setScenario] = useState('');
  const [entity, setEntity] = useState('');
  const [entryPeriod, setEntryPeriod] = useState('');
  const [period, setPeriod] = useState('');
  const [rows, setRows] = useState<BilanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const filters: ReportFilters = {
        scenario: scenario || undefined,
        entity: entity || undefined,
        entry_period: entryPeriod || undefined,
        period: period || undefined,
      };
      const data =
        reportType === 'bilan'
          ? await api.bilan(level, filters)
          : await api.compteResultat(level, filters);
      setRows(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, [reportType, level, scenario, entity, entryPeriod, period]);

  useEffect(() => {
    void load();
  }, [load]);

  const { pivot, accounts, totals } = useMemo(() => buildPivot(rows), [rows]);

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Rapports</h1>
        <div className="page__actions">
          <label className="field">
            <span>Rapport</span>
            <select
              value={reportType}
              onChange={(e) => setReportType(e.target.value as ReportType)}
              disabled={loading}
            >
              <option value="bilan">Bilan</option>
              <option value="cr">Compte de résultat</option>
            </select>
          </label>
          <Filters
            scenario={scenario}
            entity={entity}
            entryPeriod={entryPeriod}
            period={period}
            onScenarioChange={setScenario}
            onEntityChange={setEntity}
            onEntryPeriodChange={setEntryPeriod}
            onPeriodChange={setPeriod}
            disabled={loading}
          />
          <label className="field">
            <span>Niveau</span>
            <select
              value={level}
              onChange={(e) => setLevel(e.target.value as Level)}
              disabled={loading}
            >
              {LEVELS.map((lvl) => (
                <option key={lvl} value={lvl}>
                  {lvl}
                </option>
              ))}
            </select>
          </label>
          <button type="button" className="btn" onClick={load} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            <tr>
              <th className="grid__rowhead">Compte</th>
              {FLOW_COLUMNS.map((flow) => (
                <th key={flow} className="num">
                  {flow}
                </th>
              ))}
              <th className="num grid__total">Total compte</th>
            </tr>
          </thead>
          <tbody>
            {accounts.length === 0 && !loading && (
              <tr>
                <td className="grid__empty" colSpan={FLOW_COLUMNS.length + 2}>
                  Aucune donnée pour cette sélection.
                </td>
              </tr>
            )}
            {accounts.map((account) => {
              const line = pivot.get(account)!;
              const total = FLOW_COLUMNS.reduce((sum, f) => sum + line[f], 0);
              return (
                <tr key={account}>
                  <td className="grid__rowhead">{account}</td>
                  {FLOW_COLUMNS.map((flow) => (
                    <td key={flow} className="num">
                      {line[flow] !== 0 ? formatAmount(line[flow]) : ''}
                    </td>
                  ))}
                  <td className="num num--strong">{formatAmount(total)}</td>
                </tr>
              );
            })}
          </tbody>
          {accounts.length > 0 && (
            <tfoot>
              <tr>
                <td className="grid__rowhead grid__total">Total flux</td>
                {FLOW_COLUMNS.map((flow) => (
                  <td key={flow} className="num num--strong">
                    {totals[flow] !== 0 ? formatAmount(totals[flow]) : ''}
                  </td>
                ))}
                <td className="num num--strong">
                  {formatAmount(
                    FLOW_COLUMNS.reduce((sum, f) => sum + totals[f], 0),
                  )}
                </td>
              </tr>
            </tfoot>
          )}
        </table>
      </div>
    </section>
  );
}
