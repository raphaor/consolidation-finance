// Configs « liste d'objets → grille type Excel » (cf. MasterDetailEditor),
// partagées entre les pages qui les consomment depuis des groupes différents :
//  - Schémas de flux   → groupe Calculs.
//  - Jeux de périmètre → groupe Consolidation.
//  - Jeux de taux      → groupe Consolidation.

import type { MasterDetailConfig } from '../components/MasterDetailEditor';

export const SCHEMES_CONFIG: MasterDetailConfig = {
  childTable: 'flow_scheme_items',
  childKey: ['scheme', 'flow'],
  objectKey: [{ name: 'scheme', label: 'Schéma', type: 'text' }],
  gridColumns: [
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
  objectMode: { kind: 'table', table: 'flow_schemes', objectPk: 'code', labelCol: 'libelle' },
  describe: (k) => k.scheme,
};

export const PERIMETER_CONFIG: MasterDetailConfig = {
  childTable: 'perimeter',
  childKey: ['perimeter_set', 'entity', 'period'],
  objectKey: [
    {
      name: 'perimeter_set',
      label: 'Jeu de périmètre',
      type: 'select',
      optionsFrom: { table: 'perimeter_sets', value: 'code', label: 'libelle' },
    },
    {
      name: 'period',
      label: 'Période',
      type: 'select',
      optionsFrom: { table: 'periods', value: 'code' },
    },
  ],
  gridColumns: [
    {
      name: 'entity',
      label: 'Entité',
      type: 'select',
      pk: true,
      optionsFrom: { table: 'entities', value: 'code', label: 'libelle' },
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
  objectMode: { kind: 'combo' },
  describe: (k) => `${k.perimeter_set} — ${k.period}`,
};

export const RATES_CONFIG: MasterDetailConfig = {
  childTable: 'rates',
  childKey: ['rate_set', 'currency_source', 'period'],
  objectKey: [
    {
      name: 'rate_set',
      label: 'Jeu de taux',
      type: 'select',
      optionsFrom: { table: 'rate_sets', value: 'code', label: 'libelle' },
    },
    {
      name: 'period',
      label: 'Période',
      type: 'select',
      optionsFrom: { table: 'periods', value: 'code' },
    },
  ],
  gridColumns: [
    {
      name: 'currency_source',
      label: 'Devise source',
      type: 'select',
      pk: true,
      optionsFrom: { table: 'currencies', value: 'code_iso', label: 'libelle' },
    },
    { name: 'taux_ouverture', label: 'Taux ouverture', type: 'number', nullable: true },
    { name: 'taux_moyen', label: 'Taux moyen', type: 'number', nullable: true },
    { name: 'taux_close', label: 'Taux clôture', type: 'number' },
  ],
  objectMode: { kind: 'combo' },
  describe: (k) => `${k.rate_set} — ${k.period}`,
};
