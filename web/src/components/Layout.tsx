// Coquille de l'application : barre supérieure (titre + statut API),
// onglets de navigation, puis le contenu de la page active.

import type { ReactNode } from 'react';
import { HealthBadge } from './HealthBadge';
import type { HealthState } from '../hooks/useHealth';

export type PageId = 'bilan' | 'cr' | 'ecritures' | 'pipeline' | 'masterdata' | 'import';

interface Props {
  active: PageId;
  onNavigate: (page: PageId) => void;
  health: HealthState;
  children: ReactNode;
}

const TABS: { id: PageId; label: string }[] = [
  { id: 'bilan', label: 'Bilan par flux' },
  { id: 'cr', label: 'Compte de résultat' },
  { id: 'ecritures', label: 'Écritures' },
  { id: 'pipeline', label: 'Pipeline' },
  { id: 'masterdata', label: 'Master data' },
  { id: 'import', label: 'Import' },
];

export function Layout({ active, onNavigate, health, children }: Props) {
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
        {TABS.map((tab) => (
          <button
            key={tab.id}
            type="button"
            className={`tab ${active === tab.id ? 'tab--active' : ''}`}
            onClick={() => onNavigate(tab.id)}
          >
            {tab.label}
          </button>
        ))}
      </nav>

      <main className="app__main">{children}</main>
    </div>
  );
}
