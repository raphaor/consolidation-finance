// Coquille de l'application : barre supérieure (titre + statut API),
// navigation à deux niveaux (groupes puis sous-onglets), puis le contenu de la
// page active.

import type { ReactNode } from 'react';
import { HealthBadge } from './HealthBadge';
import type { HealthState } from '../hooks/useHealth';
import { useTheme } from '../hooks/useTheme';

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
  | 'jeux-regles'
  | 'controles'
  | 'coefficients'
  | 'postes'
  | 'indicateurs'
  // Référentiel
  | 'dimensions'
  | 'masterdata'
  | 'caracteristiques'
  | 'maintenance'
  // Aide
  | 'help';

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
      { id: 'jeux-regles', label: 'Jeux de règles' },
      { id: 'controles', label: 'Contrôles' },
      { id: 'coefficients', label: 'Coefficients' },
      { id: 'postes', label: 'Postes' },
      { id: 'indicateurs', label: 'Indicateurs' },
    ],
  },
  {
    id: 'referentiel',
    label: 'Référentiel',
    pages: [
      { id: 'dimensions', label: 'Dimensions' },
      { id: 'masterdata', label: 'Master data' },
      { id: 'caracteristiques', label: 'Attributs de dimension' },
      { id: 'maintenance', label: 'Maintenance' },
    ],
  },
  {
    id: 'aide',
    label: 'Aide',
    pages: [
      { id: 'help', label: 'Documentation' },
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
  const { theme, toggle } = useTheme();

  return (
    <div className="app">
      <header className="app__topbar">
        <div className="app__brand">
          <img className="app__logo" src="/orbis.png" alt="Orbis" />
          <div>
            <div className="app__title">Orbis</div>
            <div className="app__subtitle">Consolidation par les flux</div>
          </div>
        </div>
        <div className="app__topbar-right">
          <label className="theme-toggle" title="Affichage sombre">
            <input
              type="checkbox"
              checked={theme === 'dark'}
              onChange={toggle}
            />
            <span>{theme === 'dark' ? '🌙' : '☀️'} Sombre</span>
          </label>
          <HealthBadge state={health} />
        </div>
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
