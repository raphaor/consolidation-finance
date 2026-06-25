// Page « Maintenance » (groupe Référentiel) : opérations sur le magasin de
// données — sauvegarde (export), restauration (import de paquet) et reset +
// réimport des CSV. Les deux actions destructives (reset, import) passent par
// une modale de confirmation : un clic seul ne doit jamais écraser tout l'état.

import { useRef, useState, type ChangeEvent } from 'react';
import { api } from '../api';

// Le paquet de sauvegarde est un blob JSON opaque côté front (le serveur en
// porte le schéma) ; on ne le manipule que pour le relayer à l'API.
type BackupBundle = Record<string, unknown>;

type Status =
  | { kind: 'idle' }
  | { kind: 'running'; label: string }
  | { kind: 'done'; label: string }
  | { kind: 'error'; message: string };

// Action destructive en attente de confirmation.
type Pending =
  | { kind: 'reset' }
  | { kind: 'import'; bundle: BackupBundle; fileName: string };

const CONFIRM_COPY: Record<
  Pending['kind'],
  { title: string; body: string; confirmLabel: string }
> = {
  reset: {
    title: 'Reset + réimport des CSV',
    body:
      'Toutes les éditions faites dans l\'interface seront perdues : la base est ' +
      'remise à zéro puis réalimentée depuis les fichiers CSV d\'origine. Cette ' +
      'action est irréversible.',
    confirmLabel: 'Reset',
  },
  import: {
    title: 'Importer un paquet',
    body:
      'L\'intégralité de l\'état courant (référentiels, écritures, règles…) sera ' +
      'remplacée par le contenu du paquet choisi. Cette action est irréversible.',
    confirmLabel: 'Remplacer tout',
  },
};

export function MaintenancePage() {
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [pending, setPending] = useState<Pending | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const busy = status.kind === 'running';

  // Export complet → téléchargement d'un paquet JSON. Lecture seule, sans
  // confirmation.
  async function exportAll() {
    setStatus({ kind: 'running', label: 'Export…' });
    try {
      const bundle = await api.backup.exportAll();
      const blob = new Blob([JSON.stringify(bundle, null, 2)], {
        type: 'application/json',
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `conso_export_${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
      setStatus({ kind: 'done', label: 'Paquet exporté.' });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : 'erreur' });
    }
  }

  // Sélection d'un fichier : on parse puis on demande confirmation (l'import
  // effectif n'a lieu qu'après validation de la modale).
  async function pickImport(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = ''; // autorise la re-sélection du même fichier
    if (!file) return;
    try {
      const bundle = JSON.parse(await file.text()) as BackupBundle;
      setPending({ kind: 'import', bundle, fileName: file.name });
    } catch (err) {
      setStatus({ kind: 'error', message: err instanceof Error ? err.message : 'fichier illisible' });
    }
  }

  // Exécution effective après confirmation de la modale.
  async function confirmPending() {
    if (pending === null) return;
    const action = pending;
    setPending(null);
    if (action.kind === 'reset') {
      setStatus({ kind: 'running', label: 'Reset + réimport…' });
      try {
        await api.reset();
        setStatus({ kind: 'done', label: 'Base réinitialisée depuis les CSV.' });
      } catch (err) {
        setStatus({ kind: 'error', message: err instanceof Error ? err.message : 'erreur' });
      }
    } else {
      setStatus({ kind: 'running', label: 'Import du paquet…' });
      try {
        await api.backup.importAll(action.bundle);
        setStatus({ kind: 'done', label: `Paquet « ${action.fileName} » importé.` });
      } catch (err) {
        setStatus({ kind: 'error', message: err instanceof Error ? err.message : 'erreur' });
      }
    }
  }

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Maintenance du magasin</h1>
      </div>

      <p className="page__meta">
        Sauvegarde, restauration et remise à zéro de l'ensemble des données.
      </p>

      <div className="upload-grid">
        <div className="upload">
          <h2 className="upload__title">Exporter</h2>
          <p className="upload__desc">
            Télécharge un paquet JSON complet : référentiels, écritures, règles,
            coefficients, indicateurs. Lecture seule.
          </p>
          <div className="upload__form">
            <button type="button" className="btn btn--primary" onClick={exportAll} disabled={busy}>
              Tout exporter
            </button>
          </div>
        </div>

        <div className="upload">
          <h2 className="upload__title">Importer un paquet</h2>
          <p className="upload__desc">
            Restaure l'état complet depuis un paquet exporté.{' '}
            <strong>Remplace tout l'état courant.</strong>
          </p>
          <div className="upload__form">
            <button
              type="button"
              className="btn"
              onClick={() => fileInputRef.current?.click()}
              disabled={busy}
            >
              Choisir un paquet…
            </button>
            <input
              ref={fileInputRef}
              type="file"
              accept="application/json,.json"
              style={{ display: 'none' }}
              onChange={pickImport}
            />
          </div>
        </div>

        <div className="upload">
          <h2 className="upload__title">Reset + réimport CSV</h2>
          <p className="upload__desc">
            Remet la base à zéro puis la réalimente depuis les CSV d'origine.{' '}
            <strong>Annule toutes les éditions faites dans l'interface.</strong>
          </p>
          <div className="upload__form">
            <button
              type="button"
              className="btn btn--danger"
              onClick={() => setPending({ kind: 'reset' })}
              disabled={busy}
            >
              Reset + Reimport
            </button>
          </div>
        </div>
      </div>

      {status.kind !== 'idle' && (
        <div className={`status status--${status.kind}`} style={{ marginTop: 16 }}>
          {status.kind === 'running' && status.label}
          {status.kind === 'done' && status.label}
          {status.kind === 'error' && `Erreur : ${status.message}`}
        </div>
      )}

      {pending !== null && (
        <div className="modal__backdrop" onClick={() => setPending(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal__header">{CONFIRM_COPY[pending.kind].title}</div>
            <div className="modal__body">
              <div className="alert alert--warning">
                {CONFIRM_COPY[pending.kind].body}
              </div>
              {pending.kind === 'import' && (
                <p className="page__meta">Fichier : {pending.fileName}</p>
              )}
              <div className="form-actions">
                <button type="button" className="btn" onClick={() => setPending(null)}>
                  Annuler
                </button>
                <button type="button" className="btn btn--danger" onClick={() => void confirmPending()}>
                  {CONFIRM_COPY[pending.kind].confirmLabel}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
