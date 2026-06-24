// Page « Schémas & jeux » : trois référentiels composites édités selon le même
// patron liste d'objets → grille type Excel (cf. MasterDetailEditor) :
//  - Schémas de flux  : un schéma → ses flux (taux, écart, report, à-nouveau).
//  - Jeux de périmètre : un (jeu × période) → les entités (méthode, %, entrée/sortie).
//  - Jeux de taux      : un (jeu × période) → les devises (taux ouverture / moyen / clôture).

import { useState } from 'react';
import {
  MasterDetailEditor,
  type MasterDetailConfig,
} from '../components/MasterDetailEditor';

type SubView = 'schemas' | 'perimeter' | 'rates';

const SCHEMES_CONFIG: MasterDetailConfig = {
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

const PERIMETER_CONFIG: MasterDetailConfig = {
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

const RATES_CONFIG: MasterDetailConfig = {
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

const SUBVIEWS: { id: SubView; label: string; config: MasterDetailConfig }[] = [
  { id: 'schemas', label: 'Schémas de flux', config: SCHEMES_CONFIG },
  { id: 'perimeter', label: 'Jeux de périmètre', config: PERIMETER_CONFIG },
  { id: 'rates', label: 'Jeux de taux', config: RATES_CONFIG },
];

export function SchemasJeuxPage() {
  const [view, setView] = useState<SubView>('schemas');
  const active = SUBVIEWS.find((s) => s.id === view)!;

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Schémas &amp; jeux</h1>
        <div className="page__actions">
          <div className="segmented">
            {SUBVIEWS.map((s) => (
              <button
                key={s.id}
                type="button"
                className={`segmented__btn ${view === s.id ? 'is-active' : ''}`}
                onClick={() => setView(s.id)}
              >
                {s.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* `key` force le remontage à chaque sous-vue → état (objet ouvert,
          brouillon) réinitialisé proprement. */}
      <MasterDetailEditor key={view} config={active.config} />
    </section>
  );
}
