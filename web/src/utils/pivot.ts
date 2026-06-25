import type { BilanRow } from '../types';

// Les colonnes de flux ne sont plus figées : le rapport passe la liste des flux
// à afficher (catalogue `dim_flow` filtré sur le motif Fxx, cf. RapportsPage).
// Les enregistrements de montants sont donc indexés par code de flux (string).
export type FlowRecord = Record<string, number>;

export type Pivot = Map<string, FlowRecord>;

// Signe du sens comptable pour le total Σ(C) − Σ(D) : créditeur +1, débiteur −1,
// inconnu 0. Au bilan ce total tend vers 0 (équilibre actif/passif) ; en P&L il
// donne le résultat (produits − charges).
function sensSign(sens: BilanRow['sens']): number {
  return sens === 'C' ? 1 : sens === 'D' ? -1 : 0;
}

function emptyFlowRecord(flowColumns: string[]): FlowRecord {
  return Object.fromEntries(flowColumns.map((f) => [f, 0]));
}

export interface PivotResult {
  pivot: Pivot;
  accounts: string[];
  totals: FlowRecord;
  // Total signé Σ(C) − Σ(D) par flux (ligne de total des rapports).
  signedTotals: FlowRecord;
}

export function buildPivot(rows: BilanRow[], flowColumns: string[]): PivotResult {
  const known = new Set(flowColumns);
  const pivot: Pivot = new Map();
  const totals = emptyFlowRecord(flowColumns);
  const signedTotals = emptyFlowRecord(flowColumns);

  for (const row of rows) {
    if (!known.has(row.flow)) continue;
    const flow = row.flow;
    let line = pivot.get(row.account);
    if (!line) {
      line = emptyFlowRecord(flowColumns);
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

export interface AccountDetail {
  total: FlowRecord; // total compte (somme des natures)
  natures: { nature: string; values: FlowRecord }[]; // trié par code
}

export interface NaturePivotResult {
  byAccount: Map<string, AccountDetail>;
  accounts: string[]; // codes comptes triés
  totals: FlowRecord; // totaux généraux (tous comptes)
  signedTotals: FlowRecord; // total signé Σ(C) − Σ(D) par flux
}

export function buildNaturePivot(
  rows: BilanRow[],
  flowColumns: string[],
): NaturePivotResult {
  const known = new Set(flowColumns);
  // account -> nature -> Record<flow, montant>
  const acc = new Map<string, Map<string, FlowRecord>>();
  const accTotals = new Map<string, FlowRecord>();
  const totals = emptyFlowRecord(flowColumns);
  const signedTotals = emptyFlowRecord(flowColumns);

  for (const row of rows) {
    if (!known.has(row.flow)) continue;
    const flow = row.flow;

    let natures = acc.get(row.account);
    if (!natures) {
      natures = new Map();
      acc.set(row.account, natures);
      accTotals.set(row.account, emptyFlowRecord(flowColumns));
    }
    let line = natures.get(row.nature);
    if (!line) {
      line = emptyFlowRecord(flowColumns);
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
