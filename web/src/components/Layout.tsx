// Coquille de l'application : barre supérieure (titre + statut API),
// navigation à deux niveaux (groupes puis sous-onglets), puis le contenu de la
// page active.

import type { ReactNode } from 'react';
import { HealthBadge } from './HealthBadge';
import type { HealthState } from '../hooks/useHealth';

export type PageId =
  // Restitution
  | 'rapports'
  | 'ecritures'
  // Alimentation
  | 'import'
  | 'saisie'
  // Consolidation
  | 'definitions'
  | 'perimetres'
  | 'taux'
  | 'execution'
  // Calculs
  | 'schemas'
  | 'regles'
  | 'coefficients'
  | 'postes'
  | 'indicateurs'
  // Référentiel
  | 'masterdata'
  | 'caracteristiques'
  | 'maintenance';

interface Group {
  id: string;
  label: string;
  pages: { id: PageId; label: string }[];
}

// Navigation à deux niveaux : une barre de groupes, puis les sous-onglets du
// groupe actif. L'ordre des pages ici fait foi pour le rendu.
const GROUPS: Group[] = [
  {
    id: 'restitution',
    label: 'Restitution',
    pages: [
      { id: 'rapports', label: 'Rapports' },
      { id: 'ecritures', label: 'Écritures' },
    ],
  },
  {
    id: 'alimentation',
    label: 'Alimentation',
    pages: [
      { id: 'import', label: 'Import' },
      { id: 'saisie', label: 'Saisie' },
    ],
  },
  {
    id: 'consolidation',
    label: 'Consolidation',
    pages: [
      { id: 'definitions', label: 'Définitions' },
      { id: 'perimetres', label: 'Jeux de périmètre' },
      { id: 'taux', label: 'Jeux de taux' },
      { id: 'execution', label: 'Exécution' },
    ],
  },
  {
    id: 'calculs',
    label: 'Calculs',
    pages: [
      { id: 'schemas', label: 'Schémas de flux' },
      { id: 'regles', label: 'Règles' },
      { id: 'coefficients', label: 'Coefficients' },
      { id: 'postes', label: 'Postes' },
      { id: 'indicateurs', label: 'Indicateurs' },
    ],
  },
  {
    id: 'referentiel',
    label: 'Référentiel',
    pages: [
      { id: 'masterdata', label: 'Master data' },
      { id: 'caracteristiques', label: 'Attributs de dimension' },
      { id: 'maintenance', label: 'Maintenance' },
    ],
  },
];

// Index page → groupe, dérivé une fois pour situer l'onglet actif.
const GROUP_OF_PAGE = new Map<PageId, Group>(
  GROUPS.flatMap((g) => g.pages.map((p) => [p.id, g] as const)),
);

interface Props {
  active: PageId;
  onNavigate: (page: PageId) => void;
  health: HealthState;
  children: ReactNode;
}

export function Layout({ active, onNavigate, health, children }: Props) {
  const activeGroup = GROUP_OF_PAGE.get(active) ?? GROUPS[0];

  return (
    <div className="app">
      <header className="app__topbar">
        <div className="app__brand">
          <span className="app__logo">Σ</span>
          <div>
            <div className="app__title">Consolidation par les flux</div>
            <div className="app__subtitle">Prototype — moteur + UI</div>
          </div>
        </div>
        <HealthBadge state={health} />
      </header>

      <nav className="app__tabs">
        {GROUPS.map((group) => (
          <button
            key={group.id}
            type="button"
            className={`tab ${group.id === activeGroup.id ? 'tab--active' : ''}`}
            // Cliquer un groupe ouvre sa première page.
            onClick={() => onNavigate(group.pages[0].id)}
          >
            {group.label}
          </button>
        ))}
      </nav>

      <nav className="app__subtabs">
        {activeGroup.pages.map((p) => (
          <button
            key={p.id}
            type="button"
            className={`subtab ${active === p.id ? 'subtab--active' : ''}`}
            onClick={() => onNavigate(p.id)}
          >
            {p.label}
          </button>
        ))}
      </nav>

      <main className="app__main">{children}</main>
    </div>
  );
}
