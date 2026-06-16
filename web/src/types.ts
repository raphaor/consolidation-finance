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

// ---------- Master data (CRUD 8 tables) ----------

export type MasterTable =
  | 'scenarios'
  | 'entities'
  | 'periods'
  | 'accounts'
  | 'flows'
  | 'currencies'
  | 'perimeter'
  | 'rates';

export interface ColumnDef {
  name: string;
  label: string;
  type: 'text' | 'number' | 'bool' | 'date' | 'select';
  options?: string[];
  nullable?: boolean;
  pk?: boolean;
}

export interface TableDef {
  table: MasterTable;
  label: string;
  columns: ColumnDef[];
}

export const MASTER_TABLES: TableDef[] = [
  {
    table: 'scenarios',
    label: 'Scénarios',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      { name: 'type', label: 'Type', type: 'text' },
      { name: 'statut', label: 'Statut', type: 'text' },
    ],
  },
  {
    table: 'entities',
    label: 'Entités',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      { name: 'devise_fonctionnelle', label: 'Devise fonct.', type: 'text' },
      { name: 'entite_parent', label: 'Entité parent', type: 'text', nullable: true },
      { name: 'statut', label: 'Statut', type: 'text' },
    ],
  },
  {
    table: 'periods',
    label: 'Périodes',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      { name: 'type', label: 'Type', type: 'text' },
      { name: 'date_debut', label: 'Début', type: 'date' },
      { name: 'date_fin', label: 'Fin', type: 'date' },
      { name: 'statut', label: 'Statut', type: 'text' },
    ],
  },
  {
    table: 'accounts',
    label: 'Comptes',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'classe',
        label: 'Classe',
        type: 'select',
        options: ['bilan', 'resultat', 'equity', 'flux'],
      },
      { name: 'capitaux_propres', label: 'Cap. propres', type: 'bool' },
      { name: 'compte_parent', label: 'Compte parent', type: 'text', nullable: true },
    ],
  },
  {
    table: 'flows',
    label: 'Flux',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'taux_conversion',
        label: 'Taux conversion',
        type: 'select',
        options: ['close_n1', 'avg', 'close_n', 'terminal'],
      },
      { name: 'flux_ecart', label: 'Flux écart', type: 'text', nullable: true },
    ],
  },
  {
    table: 'currencies',
    label: 'Devises',
    columns: [
      { name: 'code_iso', label: 'Code ISO', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      { name: 'decimales', label: 'Décimales', type: 'number' },
    ],
  },
  {
    table: 'perimeter',
    label: 'Périmètre',
    columns: [
      { name: 'entity', label: 'Entité', type: 'text', pk: true },
      { name: 'scenario', label: 'Scénario', type: 'text', pk: true },
      { name: 'period', label: 'Période', type: 'text', pk: true },
      {
        name: 'methode',
        label: 'Méthode',
        type: 'select',
        options: ['globale', 'proportionnelle', 'équivalence'],
      },
      { name: 'pct_interet', label: '% intérêt', type: 'number' },
      { name: 'pct_integration', label: '% intégration', type: 'number' },
      { name: 'entree', label: 'Entrée', type: 'bool' },
      { name: 'sortie', label: 'Sortie', type: 'bool' },
    ],
  },
  {
    table: 'rates',
    label: 'Taux de change',
    columns: [
      { name: 'currency_source', label: 'Devise source', type: 'text', pk: true },
      { name: 'period', label: 'Période', type: 'text', pk: true },
      { name: 'taux_close', label: 'Taux clôture', type: 'number' },
      { name: 'taux_moyen', label: 'Taux moyen', type: 'number', nullable: true },
    ],
  },
];
