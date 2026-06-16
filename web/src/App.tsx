// Racine de l'application : gère l'onglet actif et expose le statut API
// via un hook de polling.

import { useState } from 'react';
import { Layout, type PageId } from './components/Layout';
import { useHealth } from './hooks/useHealth';
import { BilanPage } from './pages/BilanPage';
import { EcrituresPage } from './pages/EcrituresPage';
import { PipelinePage } from './pages/PipelinePage';
import './App.css';

export default function App() {
  const [page, setPage] = useState<PageId>('bilan');
  const health = useHealth();

  return (
    <Layout active={page} onNavigate={setPage} health={health}>
      {page === 'bilan' && <BilanPage />}
      {page === 'ecritures' && <EcrituresPage />}
      {page === 'pipeline' && <PipelinePage />}
    </Layout>
  );
}
