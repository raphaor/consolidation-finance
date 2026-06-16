import type { BilanRow, FlowCode } from '../types';
import { FLOW_COLUMNS } from '../types';

export type Pivot = Map<string, Record<FlowCode, number>>;

export interface PivotResult {
  pivot: Pivot;
  accounts: string[];
  totals: Record<FlowCode, number>;
}

export function buildPivot(rows: BilanRow[]): PivotResult {
  const pivot: Pivot = new Map();
  const totals = Object.fromEntries(
    FLOW_COLUMNS.map((f) => [f, 0]),
  ) as Record<FlowCode, number>;

  for (const row of rows) {
    if (!FLOW_COLUMNS.includes(row.flow as FlowCode)) continue;
    const flow = row.flow as FlowCode;
    let line = pivot.get(row.account);
    if (!line) {
      line = Object.fromEntries(
        FLOW_COLUMNS.map((f) => [f, 0]),
      ) as Record<FlowCode, number>;
      pivot.set(row.account, line);
    }
    line[flow] += row.amount;
    totals[flow] += row.amount;
  }

  const accounts = Array.from(pivot.keys()).sort((a, b) => a.localeCompare(b));
  return { pivot, accounts, totals };
}
