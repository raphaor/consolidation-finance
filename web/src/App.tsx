// Racine de l'application : gère la page active et expose le statut API
// via un hook de polling.

import { useState } from 'react';
import { Layout, type PageId } from './components/Layout';
import { useHealth } from './hooks/useHealth';
import {
  PERIMETER_CONFIG,
  RATES_CONFIG,
  SCHEMES_CONFIG,
} from './config/masterDetailConfigs';
import { CaracteristiquesPage } from './pages/CaracteristiquesPage';
import { CoefficientsPage } from './pages/CoefficientsPage';
import { EcrituresPage } from './pages/EcrituresPage';
import { ExecutionPage } from './pages/ExecutionPage';
import { IndicateursPage, PostesPage } from './pages/IndicatorsPage';
import { ImportPage } from './pages/ImportPage';
import { MaintenancePage } from './pages/MaintenancePage';
import { MasterDataPage } from './pages/MasterDataPage';
import { MasterDetailPage } from './pages/MasterDetailPage';
import { RapportsPage } from './pages/RapportsPage';
import { RulesPage } from './pages/RulesPage';
import { SaisiePage } from './pages/SaisiePage';
import './App.css';

export default function App() {
  const [page, setPage] = useState<PageId>('rapports');
  const health = useHealth();

  return (
    <Layout active={page} onNavigate={setPage} health={health}>
      {/* Restitution */}
      {page === 'rapports' && <RapportsPage />}
      {page === 'ecritures' && <EcrituresPage />}
      {/* Alimentation */}
      {page === 'import' && <ImportPage />}
      {page === 'saisie' && <SaisiePage />}
      {/* Consolidation */}
      {page === 'definitions' && (
        <MasterDataPage fixedTable="consolidations" title="Définitions de consolidation" />
      )}
      {page === 'perimetres' && (
        <MasterDetailPage title="Jeux de périmètre" config={PERIMETER_CONFIG} />
      )}
      {page === 'taux' && <MasterDetailPage title="Jeux de taux" config={RATES_CONFIG} />}
      {page === 'execution' && <ExecutionPage />}
      {/* Calculs */}
      {page === 'schemas' && (
        <MasterDetailPage title="Schémas de flux" config={SCHEMES_CONFIG} />
      )}
      {page === 'regles' && <RulesPage />}
      {page === 'coefficients' && <CoefficientsPage />}
      {page === 'postes' && <PostesPage />}
      {page === 'indicateurs' && <IndicateursPage />}
      {/* Référentiel */}
      {page === 'masterdata' && (
        <MasterDataPage
          // Tables à foyer dédié ailleurs (principe « une table = un seul foyer ») :
          // consolidations → Définitions ; perimeter/rates → Jeux de périmètre/taux ;
          // flow_schemes(+items) → Schémas de flux. Les car_*/lst_* sont écartés en
          // amont (groupedTables ne garde que les natives — foyer = Attributs de dimension).
          hideTables={[
            'consolidations',
            'perimeter',
            'rates',
            'flow_schemes',
            'flow_scheme_items',
          ]}
        />
      )}
      {page === 'caracteristiques' && <CaracteristiquesPage />}
      {page === 'maintenance' && <MaintenancePage />}
    </Layout>
  );
}
