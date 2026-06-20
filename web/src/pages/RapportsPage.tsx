import { Fragment, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import type { BilanRow, Level, ReportFilters } from '../types';
import { FLOW_COLUMNS, LEVELS } from '../types';
import { Filters } from '../components/Filters';
import { buildPivot, buildNaturePivot } from '../utils/pivot';
import { formatAmount } from '../utils/format';
import { usePersistentState } from '../utils/usePersistentState';

type ReportType = 'bilan' | 'cr' | 'bilan-detaille';

export function RapportsPage() {
  // Filtres persistés (survivent au changement d'onglet et au rechargement).
  const [reportType, setReportType] = usePersistentState<ReportType>('rapports.reportType', 'bilan');
  const [level, setLevel] = usePersistentState<Level>('rapports.level', 'consolidated');
  const [scenario, setScenario] = usePersistentState('rapports.scenario', '');
  const [entity, setEntity] = usePersistentState('rapports.entity', '');
  const [entryPeriod, setEntryPeriod] = usePersistentState('rapports.entryPeriod', '');
  const [period, setPeriod] = usePersistentState('rapports.period', '');
  const [nature, setNature] = usePersistentState('rapports.nature', '');
  const [rows, setRows] = useState<BilanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Comptes dépliés dans le rapport détaillé (état éphémère, par code compte).
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const detailed = reportType === 'bilan-detaille';

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const filters: ReportFilters = {
        scenario: scenario || undefined,
        entity: entity || undefined,
        entry_period: entryPeriod || undefined,
        period: period || undefined,
        nature: nature || undefined,
      };
      // Le bilan détaillé consomme le même endpoint que le bilan (qui renvoie
      // déjà le grain account × flux × nature) ; seule la mise en forme diffère.
      const data =
        reportType === 'cr'
          ? await api.compteResultat(level, filters)
          : await api.bilan(level, filters);
      setRows(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, [reportType, level, scenario, entity, entryPeriod, period, nature]);

  useEffect(() => {
    void load();
  }, [load]);

  const { pivot, accounts, totals } = useMemo(() => buildPivot(rows), [rows]);
  const naturePivot = useMemo(() => buildNaturePivot(rows), [rows]);

  const allExpanded =
    naturePivot.accounts.length > 0 &&
    naturePivot.accounts.every((a) => expanded.has(a));

  function toggleAccount(account: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(account)) next.delete(account);
      else next.add(account);
      return next;
    });
  }

  function toggleAll() {
    setExpanded(allExpanded ? new Set() : new Set(naturePivot.accounts));
  }

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
              <option value="bilan-detaille">Bilan détaillé par nature</option>
            </select>
          </label>
          <Filters
            scenario={scenario}
            entity={entity}
            entryPeriod={entryPeriod}
            period={period}
            nature={nature}
            onScenarioChange={setScenario}
            onEntityChange={setEntity}
            onEntryPeriodChange={setEntryPeriod}
            onPeriodChange={setPeriod}
            onNatureChange={setNature}
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
          {detailed && (
            <button
              type="button"
              className="btn"
              onClick={toggleAll}
              disabled={loading || naturePivot.accounts.length === 0}
            >
              {allExpanded ? 'Tout replier' : 'Tout déplier'}
            </button>
          )}
          <button type="button" className="btn" onClick={load} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}

      {detailed ? (
        <div className="table-wrap">
          <table className="grid">
            <thead>
              <tr>
                <th className="grid__rowhead">Compte / Nature</th>
                {FLOW_COLUMNS.map((flow) => (
                  <th key={flow} className="num">
                    {flow}
                  </th>
                ))}
                <th className="num grid__total">Total</th>
              </tr>
            </thead>
            <tbody>
              {naturePivot.accounts.length === 0 && !loading && (
                <tr>
                  <td className="grid__empty" colSpan={FLOW_COLUMNS.length + 2}>
                    Aucune donnée pour cette sélection.
                  </td>
                </tr>
              )}
              {naturePivot.accounts.map((account) => {
                const detail = naturePivot.byAccount.get(account)!;
                const accTotal = FLOW_COLUMNS.reduce(
                  (sum, f) => sum + detail.total[f],
                  0,
                );
                const isOpen = expanded.has(account);
                return (
                  <Fragment key={account}>
                    <tr>
                      <td className="grid__rowhead">
                        <button
                          type="button"
                          className="tree-toggle"
                          onClick={() => toggleAccount(account)}
                          aria-expanded={isOpen}
                          title={isOpen ? 'Replier' : 'Déplier'}
                        >
                          {isOpen ? '▾' : '▸'} {account}
                        </button>
                      </td>
                      {FLOW_COLUMNS.map((flow) => (
                        <td key={flow} className="num">
                          {detail.total[flow] !== 0
                            ? formatAmount(detail.total[flow])
                            : ''}
                        </td>
                      ))}
                      <td className="num num--strong">
                        {formatAmount(accTotal)}
                      </td>
                    </tr>
                    {isOpen &&
                      detail.natures.map((nat) => {
                        const natTotal = FLOW_COLUMNS.reduce(
                          (sum, f) => sum + nat.values[f],
                          0,
                        );
                        return (
                          <tr key={`${account}|${nat.nature}`} className="tree-child">
                            <td className="grid__rowhead grid__rowhead--child">
                              {nat.nature}
                            </td>
                            {FLOW_COLUMNS.map((flow) => (
                              <td key={flow} className="num">
                                {nat.values[flow] !== 0
                                  ? formatAmount(nat.values[flow])
                                  : ''}
                              </td>
                            ))}
                            <td className="num">{formatAmount(natTotal)}</td>
                          </tr>
                        );
                      })}
                  </Fragment>
                );
              })}
            </tbody>
            {naturePivot.accounts.length > 0 && (
              <tfoot>
                <tr>
                  <td className="grid__rowhead grid__total">Total flux</td>
                  {FLOW_COLUMNS.map((flow) => (
                    <td key={flow} className="num num--strong">
                      {naturePivot.totals[flow] !== 0
                        ? formatAmount(naturePivot.totals[flow])
                        : ''}
                    </td>
                  ))}
                  <td className="num num--strong">
                    {formatAmount(
                      FLOW_COLUMNS.reduce(
                        (sum, f) => sum + naturePivot.totals[f],
                        0,
                      ),
                    )}
                  </td>
                </tr>
              </tfoot>
            )}
          </table>
        </div>
      ) : (
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
      )}
    </section>
  );
}
