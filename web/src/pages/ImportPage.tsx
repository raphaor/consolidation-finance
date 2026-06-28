// Page « Import » : chargement CSV des liasses (stg_entry) et des taux de change.

import { type FormEvent, useState } from 'react';
import { errMsg } from '../utils/errMessage';
import { api } from '../api';

type Status = 'idle' | 'loading' | 'success' | 'error';

interface UploadZoneProps {
  title: string;
  description: string;
  header: string;
  onImport: (file: File) => Promise<{ imported: number }>;
}

function UploadZone({ title, description, header, onImport }: UploadZoneProps) {
  const [file, setFile] = useState<File | null>(null);
  const [status, setStatus] = useState<Status>('idle');
  const [message, setMessage] = useState('');

  function choose(f: File | null) {
    setFile(f);
    setStatus('idle');
    setMessage('');
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!file) return;
    setStatus('loading');
    setMessage('');
    try {
      const res = await onImport(file);
      setStatus('success');
      setMessage(`${res.imported} ligne(s) importée(s).`);
      setFile(null);
    } catch (err) {
      setStatus('error');
      setMessage(errMsg(err, 'erreur'));
    }
  }

  return (
    <div className="upload">
      <h2 className="upload__title">{title}</h2>
      <p className="upload__desc">{description}</p>
      <form className="upload__form" onSubmit={submit}>
        <input
          type="file"
          accept=".csv,text/csv"
          onChange={(e) => choose(e.target.files?.[0] ?? null)}
        />
        <button
          type="submit"
          className="btn btn--primary"
          disabled={!file || status === 'loading'}
        >
          {status === 'loading' ? 'Import…' : 'Importer'}
        </button>
      </form>
      <code className="upload__header">{header}</code>
      {status === 'success' && (
        <div className="alert alert--success">{message}</div>
      )}
      {status === 'error' && (
        <div className="alert alert--error">Erreur : {message}</div>
      )}
    </div>
  );
}

export function ImportPage() {
  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Import CSV</h1>
      </div>

      <div className="upload-grid">
        <UploadZone
          title="Liasses (écritures)"
          description="Ajout (append) dans la table stg_entry. Les colonnes Partner, Share et Analysis sont optionnelles."
          header="Phase, Entity, Entry_period, Period, Account, Flow, Currency, Nature, Partner*, Share*, Analysis*, Analysis2*, Source*, Amount"
          onImport={(f) => api.importEntries(f)}
        />
        <UploadZone
          title="Taux de change"
          description="Upsert dans la table rates (clé : rate_set + currency_source + period)."
          header="rate_set, currency_source, period, taux_close, taux_moyen"
          onImport={(f) => api.importRates(f)}
        />
        <UploadZone
          title="Périmètre de consolidation"
          description="Upsert dans la table perimeter (clé : perimeter_set + entity + period). Les colonnes entree/sortie acceptent true/false."
          header="perimeter_set, entity, period, methode, pct_interet, pct_integration, entree, sortie"
          onImport={(f) => api.importPerimeter(f)}
        />
      </div>
    </section>
  );
}
