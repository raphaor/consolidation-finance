// Types échangés avec l'API REST (cf. EXPRESSION_DE_BESOIN.md).

export interface LevelCount {
  level: string;
  count: number;
}

export interface BilanRow {
  account: string;
  flow: string;
  amount: number;
}

export interface Entry {
  id: number;
  scenario: string;
  entity: string;
  entry_period: string;
  period: string;
  account: string;
  flow: string;
  currency: string;
  partner: string | null;
  share: string | null;
  analysis: string | null;
  audit_id: string;
  level: string;
  amount: number;
}

export interface PipelineCounts {
  corporate: number;
  reclassified: number;
  converted: number;
  consolidated: number;
}

export interface HealthStatus {
  status: string;
}

// Catalogue des flux attendus en colonnes du bilan par flux
// (voir docs/FLUX_CONSO.md).
export const FLOW_COLUMNS = [
  'F00',
  'F01',
  'F20',
  'F80',
  'F81',
  'F98',
  'F99',
] as const;

export type FlowCode = (typeof FLOW_COLUMNS)[number];

export const LEVELS = [
  'corporate',
  'reclassified',
  'converted',
  'consolidated',
] as const;

export type Level = (typeof LEVELS)[number];
