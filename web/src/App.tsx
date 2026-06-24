// Racine de l'application : gère l'onglet actif et expose le statut API
// via un hook de polling.

import { useState } from 'react';
import { Layout, type PageId } from './components/Layout';
import { useHealth } from './hooks/useHealth';
import { CaracteristiquesPage } from './pages/CaracteristiquesPage';
import { CoefficientsPage } from './pages/CoefficientsPage';
import { EcrituresPage } from './pages/EcrituresPage';
import { ImportPage } from './pages/ImportPage';
import { MasterDataPage } from './pages/MasterDataPage';
import { PipelinePage } from './pages/PipelinePage';
import { RapportsPage } from './pages/RapportsPage';
import { RulesPage } from './pages/RulesPage';
import { SaisiePage } from './pages/SaisiePage';
import { SchemasJeuxPage } from './pages/SchemasJeuxPage';
import './App.css';

export default function App() {
  const [page, setPage] = useState<PageId>('rapports');
  const health = useHealth();

  return (
    <Layout active={page} onNavigate={setPage} health={health}>
      {page === 'rapports' && <RapportsPage />}
      {page === 'ecritures' && <EcrituresPage />}
      {page === 'saisie' && <SaisiePage />}
      {page === 'pipeline' && <PipelinePage />}
      {page === 'masterdata' && <MasterDataPage />}
      {page === 'schemas' && <SchemasJeuxPage />}
      {page === 'caracteristiques' && <CaracteristiquesPage />}
      {page === 'coefficients' && <CoefficientsPage />}
      {page === 'regles' && <RulesPage />}
      {page === 'import' && <ImportPage />}
    </Layout>
  );
}
