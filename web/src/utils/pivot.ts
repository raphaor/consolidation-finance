import type { BilanRow, FlowCode } from '../types';
import { FLOW_COLUMNS } from '../types';

export type Pivot = Map<string, Record<FlowCode, number>>;

// Signe du sens comptable pour le total Σ(C) − Σ(D) : créditeur +1, débiteur −1,
// inconnu 0. Au bilan ce total tend vers 0 (équilibre actif/passif) ; en P&L il
// donne le résultat (produits − charges).
function sensSign(sens: BilanRow['sens']): number {
  return sens === 'C' ? 1 : sens === 'D' ? -1 : 0;
}

export interface PivotResult {
  pivot: Pivot;
  accounts: string[];
  totals: Record<FlowCode, number>;
  // Total signé Σ(C) − Σ(D) par flux (ligne de total des rapports).
  signedTotals: Record<FlowCode, number>;
}

export function buildPivot(rows: BilanRow[]): PivotResult {
  const pivot: Pivot = new Map();
  const totals = Object.fromEntries(
    FLOW_COLUMNS.map((f) => [f, 0]),
  ) as Record<FlowCode, number>;
  const signedTotals = Object.fromEntries(
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
    signedTotals[flow] += sensSign(row.sens) * row.amount;
  }

  const accounts = Array.from(pivot.keys()).sort((a, b) => a.localeCompare(b));
  return { pivot, accounts, totals, signedTotals };
}

// ── Pivot détaillé par nature ────────────────────────────────────────────────
// Comme `buildPivot`, mais conserve la dimension `nature` : chaque compte porte
// un total (somme des natures, = la ligne du bilan classique) plus le détail par
// nature. Sert au rapport « Bilan détaillé par nature » (lignes compte
// dépliables → sous-lignes par nature).

function emptyFlowRecord(): Record<FlowCode, number> {
  return Object.fromEntries(FLOW_COLUMNS.map((f) => [f, 0])) as Record<
    FlowCode,
    number
  >;
}

export interface AccountDetail {
  total: Record<FlowCode, number>; // total compte (somme des natures)
  natures: { nature: string; values: Record<FlowCode, number> }[]; // trié par code
}

export interface NaturePivotResult {
  byAccount: Map<string, AccountDetail>;
  accounts: string[]; // codes comptes triés
  totals: Record<FlowCode, number>; // totaux généraux (tous comptes)
  signedTotals: Record<FlowCode, number>; // total signé Σ(C) − Σ(D) par flux
}

export function buildNaturePivot(rows: BilanRow[]): NaturePivotResult {
  // account -> nature -> Record<flow, montant>
  const acc = new Map<string, Map<string, Record<FlowCode, number>>>();
  const accTotals = new Map<string, Record<FlowCode, number>>();
  const totals = emptyFlowRecord();
  const signedTotals = emptyFlowRecord();

  for (const row of rows) {
    if (!FLOW_COLUMNS.includes(row.flow as FlowCode)) continue;
    const flow = row.flow as FlowCode;

    let natures = acc.get(row.account);
    if (!natures) {
      natures = new Map();
      acc.set(row.account, natures);
      accTotals.set(row.account, emptyFlowRecord());
    }
    let line = natures.get(row.nature);
    if (!line) {
      line = emptyFlowRecord();
      natures.set(row.nature, line);
    }
    line[flow] += row.amount;
    accTotals.get(row.account)![flow] += row.amount;
    totals[flow] += row.amount;
    signedTotals[flow] += sensSign(row.sens) * row.amount;
  }

  const byAccount = new Map<string, AccountDetail>();
  for (const [account, natures] of acc) {
    const natureRows = Array.from(natures.entries())
      .map(([nature, values]) => ({ nature, values }))
      .sort((a, b) => a.nature.localeCompare(b.nature));
    byAccount.set(account, { total: accTotals.get(account)!, natures: natureRows });
  }

  const accounts = Array.from(byAccount.keys()).sort((a, b) =>
    a.localeCompare(b),
  );
  return { byAccount, accounts, totals, signedTotals };
}
