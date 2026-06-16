// Page « Bilan par flux » : table pivot comptes (lignes) x flux (colonnes).
// Les montants proviennent de /api/bilan, agrégés côté client par sécurité
// (le serveur peut renvoyer plusieurs lignes par couple account/flow).

import { useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { Filters } from '../components/Filters';
import type { BilanRow, FlowCode, Level } from '../types';
import { FLOW_COLUMNS, LEVELS } from '../types';
import { formatAmount } from '../utils/format';

type Pivot = Map<string, Record<FlowCode, number>>;

function buildPivot(rows: BilanRow[]): {
  pivot: Pivot;
  accounts: string[];
  totals: Record<FlowCode, number>;
} {
  const pivot: Pivot = new Map();
  const totals: Record<FlowCode, number> = Object.fromEntries(
    FLOW_COLUMNS.map((f) => [f, 0]),
  ) as Record<FlowCode, number>;

  for (const row of rows) {
    if (!FLOW_COLUMNS.includes(row.flow as FlowCode)) continue;
    const flow = row.flow as FlowCode;
    let line = pivot.get(row.account);
    if (!line) {
      line = Object.fromEntries(FLOW_COLUMNS.map((f) => [f, 0])) as Record<FlowCode, number>;
      pivot.set(row.account, line);
    }
    line[flow] += row.amount;
    totals[flow] += row.amount;
  }

  const accounts = Array.from(pivot.keys()).sort((a, b) => a.localeCompare(b));
  return { pivot, accounts, totals };
}

export function BilanPage() {
  const [level, setLevel] = useState<Level>('consolidated');
  const [scenario, setScenario] = useState('');
  const [period, setPeriod] = useState('');
  const [rows, setRows] = useState<BilanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.bilan(level, { scenario, period });
      setRows(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, [level, scenario, period]);

  useEffect(() => {
    void load();
  }, [load]);

  const { pivot, accounts, totals } = useMemo(() => buildPivot(rows), [rows]);

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Bilan par flux</h1>
        <div className="page__actions">
          <Filters
            scenario={scenario}
            period={period}
            onScenarioChange={setScenario}
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
                  Aucune donnée pour ce niveau.
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
