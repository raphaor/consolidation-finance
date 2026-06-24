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
  phase: string;
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

// Saisie manuelle d'une écriture (POST/PUT /api/entries). Toutes les valeurs
// sont en `string` (le back parse le montant, tolère la virgule décimale).
// Les champs optionnels (partner, share, analysis, analysis2) peuvent être ''.
export interface EntryInput {
  phase: string;
  entity: string;
  entry_period: string;
  period: string;
  account: string;
  flow: string;
  currency: string;
  nature: string;
  partner: string;
  share: string;
  analysis: string;
  analysis2: string;
  amount: string;
}

export interface PipelineCounts {
  corporate: number;
  converted: number;
  consolidated: number;
}

export interface HealthStatus {
  status: string;
}

export interface Consolidation {
  id: number;
  libelle: string;
  phase: string;
  exercice: string;
  perimeter_set: string;
  variant: string;
  presentation_currency: string;
  perimeter_period: string;
  rate_set: string;
  rate_period: string;
  ruleset_code: string | null;
  a_nouveau_consolidation_id: number | null;
  statut: string;
}

// Réponse de GET /api/consolidations — consolidation « dépliée » pour le
// dropdown PipelinePage et les filtres partagés (Filters). Même forme que
// `Consolidation` (les champs sont tous renseignés côté serveur).
export interface ConsolidationSummary {
  id: number;
  libelle: string;
  phase: string;
  exercice: string;
  perimeter_set: string;
  variant: string;
  presentation_currency: string;
  perimeter_period: string;
  rate_set: string;
  rate_period: string;
  ruleset_code: string | null;
  a_nouveau_consolidation_id: number | null;
  statut: string;
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
  consolidation?: number;
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

// Liste statique des tables master data natives (built-in). Servie aussi par
// `GET /api/md` côté backend. Les tables dynamiques (`car_<code>`,
// `lst_<code>`) y sont ajoutées à l'exécution (cf. characteristics / value_lists).
export const NATIVE_MASTER_TABLES = [
  'consolidations',
  'entities',
  'periods',
  'accounts',
  'flows',
  'currencies',
  'natures',
  'methods',
  'perimeter',
  'rates',
  'sous_classes',
  'scenario_categories',
  'variants',
  'rate_sets',
  'perimeter_sets',
  'flow_schemes',
  'flow_scheme_items',
] as const;

export type NativeMasterTable = (typeof NATIVE_MASTER_TABLES)[number];

// Nom d'API d'une table master data : native OU dynamique (`car_<code>`,
// `lst_<code>`). Les valeurs proviennent de `GET /api/md` (source serveur).
export type MasterTable = string;

export interface ColumnDef {
  name: string;
  label: string;
  type: 'text' | 'number' | 'bool' | 'date' | 'select';
  options?: string[];
  optionsFrom?: { table: MasterTable; value: string; label?: string };
  // Options chargées via une API dédiée (hors tables master data) : ex.
  // 'rulesets' alimente le select depuis GET /api/rulesets (cf. masterFields).
  optionsApi?: 'rulesets';
  nullable?: boolean;
  pk?: boolean;
  // Clé primaire auto-générée par la base (ex : `consolidations.id`). Sur
  // création, le champ est masqué et omis du payload (INSERT ... RETURNING id) ;
  // sur édition, il est verrouillé (pk). Sur suppression, on identifie par ce pk.
  auto?: boolean;
}

// ---------- Schéma dynamique (GET /api/md et GET /api/md/{table}/schema) ----------

// Résumé d'une table navigable (renvoyé par `GET /api/md`).
export interface TableSummary {
  table: string;
  label: string;
  kind: 'native' | 'characteristic' | 'value_list';
}

// Cible d'une FK portée par une colonne : permet au front de configurer un
// dropdown d'options depuis `GET /api/md/{fk.table}`.
export interface FkTarget {
  // Nom d'API de la table cible (`accounts`, `car_comportement`, `lst_incoterm`…).
  table: string;
  // Colonne clé de la table cible (`code`, `code_iso`…).
  column: string;
  // `true` si non-nullable (rejette une valeur vide à l'écriture).
  required: boolean;
}

// Métadonnées d'une colonne telles qu'exposées par `GET /api/md/{table}/schema`.
export interface ColumnSchema {
  name: string;
  pk: boolean;
  fk: FkTarget | null;
}

// Schéma complet d'une table : colonnes natives + dynamiques, avec FK.
export interface TableSchema {
  table: string;
  label: string;
  sql_name: string;
  columns: ColumnSchema[];
  pk: string[];
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
    table: 'perimeter_sets',
    label: 'Jeux de périmètre',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
    ],
  },
  {
    table: 'consolidations',
    label: 'Définitions de consolidation',
    columns: [
      // `id` est auto-généré (INSERT ... RETURNING id) : masqué à la création,
      // verrouillé (pk) à l'édition.
      { name: 'id', label: 'ID', type: 'number', pk: true, auto: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      {
        name: 'phase',
        label: 'Phase',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'scenario_categories', value: 'code', label: 'libelle' },
      },
      {
        name: 'exercice',
        label: 'Exercice',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
      {
        name: 'perimeter_set',
        label: 'Jeu de périmètre',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'perimeter_sets', value: 'code', label: 'libelle' },
      },
      {
        name: 'variant',
        label: 'Variante',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'variants', value: 'code', label: 'libelle' },
      },
      {
        name: 'presentation_currency',
        label: 'Devise présentation',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'currencies', value: 'code_iso', label: 'libelle' },
      },
      {
        name: 'perimeter_period',
        label: 'Période de périmètre',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
      {
        name: 'rate_set',
        label: 'Jeu de taux',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'rate_sets', value: 'code', label: 'libelle' },
      },
      {
        name: 'rate_period',
        label: 'Période des taux',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'periods', value: 'code' },
      },
      {
        name: 'ruleset_code',
        label: 'Jeu de règles',
        type: 'select',
        nullable: true,
        optionsApi: 'rulesets',
      },
      {
        name: 'a_nouveau_consolidation_id',
        label: 'Conso d\'à-nouveau',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'consolidations', value: 'id', label: 'libelle' },
      },
      {
        name: 'statut',
        label: 'Statut',
        type: 'select',
        options: ['brouillon', 'ouvert', 'verrouillé'],
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
      {
        name: 'flow_scheme',
        label: 'Schéma de flux',
        type: 'select',
        nullable: true,
        optionsFrom: { table: 'flow_schemes', value: 'code', label: 'libelle' },
      },
      // Le regroupement par nature (caractéristique) et le compte parent
      // (référence directe) se gèrent dans la page « Attributs de dimension ».
    ],
  },
  {
    table: 'flows',
    label: 'Flux',
    columns: [
      // Dimension nue : tout le comportement (taux, écart, report, à-nouveau)
      // se gère dans « Schémas de flux » (cf. Q32).
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
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
    table: 'methods',
    label: 'Méthodes de consolidation',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
      // `consolidated` = true : méthode intégrée (reprise au niveau consolidated,
      // pondérée par pct_integration) ; false : mise en équivalence (cf. dim_method).
      { name: 'consolidated', label: 'Intégrée à la conso', type: 'bool' },
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
        name: 'perimeter_set',
        label: 'Jeu de périmètre',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'perimeter_sets', value: 'code', label: 'libelle' },
      },
      {
        name: 'entity',
        label: 'Entité',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'entities', value: 'code', label: 'libelle' },
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
        optionsFrom: { table: 'methods', value: 'code', label: 'libelle' },
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
  {
    table: 'flow_schemes',
    label: 'Schémas de flux',
    columns: [
      { name: 'code', label: 'Code', type: 'text', pk: true },
      { name: 'libelle', label: 'Libellé', type: 'text' },
    ],
  },
  {
    table: 'flow_scheme_items',
    label: 'Articulation des flux (par schéma)',
    columns: [
      {
        name: 'scheme',
        label: 'Schéma',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'flow_schemes', value: 'code', label: 'libelle' },
      },
      {
        name: 'flow',
        label: 'Flux',
        type: 'select',
        pk: true,
        optionsFrom: { table: 'flows', value: 'code', label: 'libelle' },
      },
      {
        name: 'taux_conversion',
        label: 'Taux conversion',
        type: 'select',
        options: ['close_n1', 'avg', 'close_n'],
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
        optionsFrom: { table: 'flows', value: 'code', label: 'libelle' },
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

// Une condition de sélection sur fact_entry.
//
// Par défaut, la condition porte directement sur la colonne dimensionnelle de
// `fact_entry` (`e.<dim>`). Deux traversées optionnelles permettent de filtrer
// par **attribut** de la dimension (mutuellement exclusives) :
//   - `via` : traverse une caractéristique N1 (regroupement). Le filtre porte
//     alors sur la valeur N1 du membre (ex : tous les comptes dont le
//     `comportement` = `VENTES_IC`).
//   - `ref` : traverse une référence directe (patron B, colonne sur la master
//     data). Ex : tous les comptes dont le `compte_parent` = `60`. Couvre aussi
//     les **FK natives** auto-peuplées (ex : `account.sous_classe`,
//     `entity.entite_parent`) via `seed_native`.
//   - `attr` : traverse un **enum natif** (CHECK du DDL, ex : `account.classe`
//     ∈ {bilan, resultat, flux}). Pas de table cible, filtre direct sur la
//     colonne master data. Cf. `references::NATIVE_ENUMS`.
// Cf. `docs/REGLES_CONSO.md` §4.1 et `rules.rs::parse_selection_cond`.
export interface SelectionCond {
  dim: string; // phase, entity, account, flow, etc.
  op: string;
  val: unknown;
  via?: string;
  ref?: string;
  attr?: string;
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
      // Modes de destination :
      // - `inherit` : conserve la valeur source (`e.<dim>`).
      // - `override` : force une constante (`value`).
      // - `null` : vide la valeur (`NULL`).
      // - `map` : traverse une caractéristique N1 (`via`) pour lire un attribut
      //   N2 (`attr`) dont la valeur surcharge la dimension.
      // - `map_ref` : traverse une référence directe patron B (`ref`) portée par
      //   la dimension écrite (ex : `compte_parent` sur `account`). La valeur
      //   est résolue par un seul JOIN sur la master data.
      mode: 'inherit' | 'override' | 'null' | 'map' | 'map_ref';
      value?: string;
      via?: string;
      attr?: string;
      ref?: string;
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
//
// `native = true` : FK native auto-peuplée par `seed_native` (ex :
// `account.sous_classe`, `entity.entite_parent`). Reflète le DDL statique,
// verrouillée contre édition/suppression via l'API.

export interface CustomReference {
  host_dimension: string;
  column: string;
  target_dimension: string;
  native?: boolean;
}

// ---------- Enums natifs (CHECK du DDL, GET /api/meta/native-enums) -------------
// Attribut natif d'une master data dont les valeurs sont une liste fermée
// (ex : `account.classe` ∈ {bilan, resultat, flux}). Pas de table cible —
// exposé via le mode `attr` de `SelectionCond` (filtre direct sur la colonne).
// Cf. `references::NATIVE_ENUMS`.

export interface NativeEnum {
  host_dimension: string;
  column: string;
  values: string[];
}

// ---------- Listes de valeurs (référentiels, GET /api/meta/value-lists) ---------
// Nomenclature `code/libellé` autonome (`lst_<code>`), réutilisable comme cible
// d'un attribut N2 de caractéristique, mais qui n'est pas une dimension (aucun
// axe d'écriture). Cf. value_lists.rs.

export interface ValueList {
  code: string;
  libelle: string;
  value_table: string;
}

// ---------- Coefficients (moteur de formules — volet 1, GET /api/coefficients) ----------
// Une formule nommée évaluée au grain d'une écriture de règle. `kind` distingue
// les coefficients natifs ('builtin', verrouillés) des coefficients utilisateur
// ('user', modifiables en place). Cf. docs/FORMULES.md §3.
export interface Coefficient {
  code: string;
  libelle: string | null;
  expression: string;
  kind: 'builtin' | 'user';
}

// Opérande disponible (catalogue de périmètre) : `token` s'insère entre [ ] dans
// une formule ; `label` est l'affichage du panneau de références.
export interface CoefficientOperand {
  token: string;
  label: string;
}

// Réponse de la preview live (POST /api/coefficients/preview).
export interface CoefficientPreview {
  ok: boolean;
  value?: number;
  error?: string;
  sql?: string;
  operands: string[];
}

// ---------- Indicateurs / KPI (moteur de formules — volet 2) ----------
// Un poste (dim_aggregate) = sélection nommée sur fact_entry agrégée en montant.
// Un indicateur (dim_indicator) = formule combinant des postes, calculée au grain.
// Cf. docs/FORMULES.md §4.
export interface AggregateCond {
  dim: string;
  op: string;
  val?: unknown;
  via?: string; // caractéristique N1
  ref?: string; // référence directe (patron B)
  attr?: string; // enum natif (classe, sous_classe…)
}

export interface AggregateDef {
  level: string;
  selection: AggregateCond[];
}

export interface Aggregate {
  code: string;
  libelle: string | null;
  level: string;
  definition: AggregateDef;
}

export interface Indicator {
  code: string;
  libelle: string | null;
  expression: string;
  grain: string[];
  format: string | null;
}

// Opérande référençable dans une formule d'indicateur (poste ou autre indicateur).
export interface IndicatorOperand {
  token: string;
  label: string;
  kind: 'poste' | 'indicateur';
}

// Une ligne de résultat de preview : valeurs de grain + valeur calculée.
export interface IndicatorRow {
  grain: Record<string, string | null>;
  value: number | null;
}

export interface IndicatorPreview {
  ok: boolean;
  error?: string;
  sql?: string;
  rows: IndicatorRow[];
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
