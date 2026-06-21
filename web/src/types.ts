// Types échangés avec l'API REST (cf. EXPRESSION_DE_BESOIN.md).

export interface LevelCount {
  level: string;
  count: number;
}

export interface BilanRow {
  account: string;
  flow: string;
  nature: string;
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
  nature: string;
  partner: string | null;
  share: string | null;
  analysis: string | null;
  analysis2: string;
  level: string;
  amount: number;
}

export interface PipelineCounts {
  corporate: number;
  converted: number;
  consolidated: number;
}

export interface HealthStatus {
  status: string;
}

export interface Scenario {
  code: string;
  libelle: string;
  category: string | null;
  entry_period: string | null;
  presentation_currency: string | null;
  variant: string | null;
  ruleset_code: string | null;
  rate_set: string | null;
  statut: string | null;
  a_nouveau_scenario: string | null;
}

// Réponse de GET /api/scenarios — scénario v2 « déplié » pour le dropdown
// PipelinePage (cf. SPEC_SCENARIO_V2_TECH §4.6).
export interface ScenarioSummary {
  code: string;
  libelle: string | null;
  category: string | null;
  entry_period: string | null;
  presentation_currency: string | null;
  variant: string | null;
  ruleset_code: string | null;
  rate_set: string | null;
  statut: string | null;
  a_nouveau_scenario: string | null;
}

export interface Period {
  code: string;
  libelle: string;
  type: string;
  date_debut: string;
  date_fin: string;
  statut: string;
}

export interface Entity {
  code: string;
  libelle: string;
  devise_fonctionnelle: string;
  entite_parent: string | null;
  statut: string;
}

export interface Nature {
  code: string;
  libelle: string;
  rules: string | null;
}

export interface ReportFilters {
  scenario?: string;
  entity?: string;
  entry_period?: string;
  period?: string;
  nature?: string;
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
  'converted',
  'consolidated',
] as const;

export type Level = (typeof LEVELS)[number];

// ---------- Master data (CRUD tables référentielles) ----------

export type MasterTable =
  | 'scenarios'
  | 'entities'
  | 'periods'
  | 'accounts'
  | 'flows'
  | 'currencies'
  | 'natures'
  | 'methods'
  | 'perimeter'
  | 'rates'
  | 'sous_classes'
  | 'scenario_categories'
  | 'variants'
  | 'rate_sets';

export interface ColumnDef {
  name: string;
  label: string;
  type: 'text' | 'number' | 'bool' | 'date' | 'select';
  options?: string[];
  optionsFrom?: { table: MasterTable; value: string; label?: string };
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
    table: 'scenario_categories',
    label: 'Phases',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
    ],
  },
  {
    table: 'variants',
    label: 'Variantes',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
    ],
  },
  {
    table: 'rate_sets',
    label: 'Jeux de taux',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
    ],
  },
  {
    table: 'scenarios',
    label: 'Définitions de consolidation',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'category',
        label: 'Phase',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'scenario_categories', value: 'code', label: 'libelle' },
      },
      {
        name: 'entry_period',
        label: 'Période d\'entrée',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
      {
        name: 'presentation_currency',
        label: 'Devise présentation',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'currencies', value: 'code_iso', label: 'libelle' },
      },
      {
        name: 'variant',
        label: 'Variante',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'variants', value: 'code', label: 'libelle' },
      },
      { name: 'ruleset_code', label: 'Ruleset', type: 'text', nullable: true },
      {
        name: 'rate_set',
        label: 'Jeu de taux',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'rate_sets', value: 'code', label: 'libelle' },
      },
      { name: 'statut', label: 'Statut', type: 'text' },
      {
        name: 'a_nouveau_scenario',
        label: 'Conso d\'à-nouveau',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'scenarios', value: 'code', label: 'libelle' },
      },
    ],
  },
  {
    table: 'entities',
    label: 'Entités',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'devise_fonctionnelle',
        label: 'Devise fonct.',
        type: 'select',
        optionsFrom: { table: 'currencies', value: 'code_iso', label: 'libelle' },
      },
      {
        name: 'entite_parent',
        label: 'Entité parent',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'entities', value: 'code', label: 'libelle' },
      },
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
        options: ['bilan', 'resultat', 'flux'],
      },
      {
        name: 'sous_classe',
        label: 'Sous-classe',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'sous_classes', value: 'code', label: 'libelle' },
      },
      // Le regroupement par nature (caractéristique) et le compte parent
      // (référence directe) se gèrent dans la page « Attributs de dimension ».
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
      {
        name: 'flux_ecart',
        label: 'Flux écart',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'flows', value: 'code', label: 'libelle' },
      },
      {
        name: 'flux_de_report',
        label: 'Flux de report',
        type: 'select',
        nullable: true,
        options: ['F00', 'F01', 'F20', 'F80', 'F81', 'F98', 'F99'],
      },
      {
        name: 'flux_a_nouveau',
        label: 'Flux d\'à-nouveau',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'flows', value: 'code', label: 'libelle' },
      },
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
    table: 'natures',
    label: 'Natures',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      { name: 'rules', label: 'Règles', type: 'text', nullable: true },
    ],
  },
  {
    table: 'sous_classes',
    label: 'Sous-classes',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'classe',
        label: 'Classe',
        type: 'select',
        options: ['bilan', 'resultat', 'flux'],
      },
    ],
  },
  {
    table: 'perimeter',
    label: 'Périmètre',
    columns: [
      {
        name: 'entity',
        label: 'Entité',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'entities', value: 'code', label: 'libelle' },
      },
      {
        name: 'scenario',
        label: 'Définition de consolidation',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'scenarios', value: 'code' },
      },
      {
        name: 'period',
        label: 'Période',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
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
      {
        name: 'rate_set',
        label: 'Jeu de taux',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'rate_sets', value: 'code', label: 'libelle' },
      },
      {
        name: 'currency_source',
        label: 'Devise source',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'currencies', value: 'code_iso', label: 'libelle' },
      },
      {
        name: 'period',
        label: 'Période',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
      { name: 'taux_close', label: 'Taux clôture', type: 'number' },
      { name: 'taux_moyen', label: 'Taux moyen', type: 'number', nullable: true },
    ],
  },
];

// ---------- Règles de consolidation ----------

export interface RuleSummary {
  code: string;
  libelle: string;
}

export interface RuleDetail {
  code: string;
  libelle: string;
  definition: object;
}

// Une condition de périmètre
export interface ScopeCond {
  target: 'entity' | 'partner' | 'share';
  dim: string; // methode, pct_interet, pct_integration, entree, sortie
  op: string; // =, !=, >, <, >=, <=, IN, IS NULL, IS NOT NULL
  val: unknown;
}

// Une condition de sélection sur fact_entry
export interface SelectionCond {
  dim: string; // scenario, entity, account, flow, etc.
  op: string;
  val: unknown;
}

// Une opération
export interface Operation {
  seq: number;
  level: string; // corporate, converted, consolidated
  selection: SelectionCond[];
  coefficient: { type: string; value?: number }; // pct_integration | pct_interet | constant
  multiplicateur: number;
  destination: Record<
    string,
    {
      mode: 'inherit' | 'override' | 'null' | 'map';
      value?: string;
      // Mode `map` : caractéristique N1 traversée (`via`) et attribut N2 (`attr`)
      // dont la valeur surcharge la dimension (cf. characteristics.rs).
      via?: string;
      attr?: string;
    }
  >;
}

// Définition complète d'une règle
export interface RuleDefinition {
  scope: ScopeCond[];
  operations: Operation[];
}

export interface RulesetSummary {
  code: string;
  libelle: string;
}

export interface RulesetItem {
  ordre: number;
  rule_code: string;
  rule_libelle?: string;
}

export interface RulesetDetail {
  code: string;
  libelle: string;
  items: RulesetItem[];
}

export interface RuleResult {
  rule_code: string;
  level: string;
  generated: number;
}

export interface RulesetReport {
  ruleset: string;
  rules: RuleResult[];
  total_generated: number;
}

// ---------- Dimensions (registre central) ----------

export type DimCategory = 'Fixed' | 'Active' | 'Analytical';

export interface DimensionInfo {
  name: string;
  category: DimCategory;
  custom: boolean;
  label: string;
  pilotable: boolean;
}

// ---------- Caractéristiques N1/N2 (GET /api/meta/characteristics) ----------
// Une caractéristique N1 regroupe les membres d'une dimension de base
// (`base_dimension`) ; ses attributs N2 (`attributes`) pointent chacun vers une
// dimension cible (`target_dimension`). Les valeurs vivent dans `value_table`
// (`car_<code>`).

export interface CharacteristicAttribute {
  name: string;
  libelle: string;
  target_dimension: string;
}

export interface Characteristic {
  code: string;
  libelle: string;
  base_dimension: string;
  value_table: string;
  attributes: CharacteristicAttribute[];
}

// ---------- Références directes (patron B, GET /api/meta/references-custom) -----
// Une colonne ajoutée à l'exécution sur la master data d'une dimension hôte,
// pointant directement vers une dimension cible (y compris elle-même :
// hiérarchie). Pas de table intermédiaire (cf. custom_references.rs).

export interface CustomReference {
  host_dimension: string;
  column: string;
  target_dimension: string;
}

// ---------- Références (graphe de jointures, GET /api/meta/references) ----------
// Source de vérité serveur (engine/src/references.rs). `table` est en nom SQL
// (ex. stg_entry, sat_perimeter, dim_*) ; `target_table` est traduit en nom de
// table master data (MasterTable) quand la cible en a un.
export interface ReferenceInfo {
  table: string;
  column: string;
  target_table: string;
  target_column: string;
  required: boolean;
}

// ---------- Santé des données (GET /api/meta/health) ----------
// Rapport d'orphelins : valeurs présentes en source mais absentes de la cible.
export interface OrphanCheck {
  table: string;
  column: string;
  target_table: string;
  target_column: string;
  count: number; // nb de valeurs orphelines distinctes
  sample: string[]; // échantillon (max 20)
}

export interface DataHealthReport {
  ok: boolean;
  total: number;
  checks: OrphanCheck[];
}
