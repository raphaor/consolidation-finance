// Racine de l'application : gère l'onglet actif et expose le statut API
// via un hook de polling.

import { useState } from 'react';
import { Layout, type PageId } from './components/Layout';
import { useHealth } from './hooks/useHealth';
import { BilanPage } from './pages/BilanPage';
import { CompteResultatPage } from './pages/CompteResultatPage';
import { EcrituresPage } from './pages/EcrituresPage';
import { ImportPage } from './pages/ImportPage';
import { MasterDataPage } from './pages/MasterDataPage';
import { PipelinePage } from './pages/PipelinePage';
import './App.css';

export default function App() {
  const [page, setPage] = useState<PageId>('bilan');
  const health = useHealth();

  return (
    <Layout active={page} onNavigate={setPage} health={health}>
      {page === 'bilan' && <BilanPage />}
      {page === 'cr' && <CompteResultatPage />}
      {page === 'ecritures' && <EcrituresPage />}
      {page === 'pipeline' && <PipelinePage />}
      {page === 'masterdata' && <MasterDataPage />}
      {page === 'import' && <ImportPage />}
    </Layout>
  );
}
