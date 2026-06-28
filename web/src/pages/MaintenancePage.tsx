// Page « Maintenance » (groupe Référentiel) : opérations sur le magasin de
// données — sauvegarde (export), restauration (import de paquet) et reset +
// réimport des CSV. Les deux actions destructives (reset, import) passent par
// une modale de confirmation : un clic seul ne doit jamais écraser tout l'état.

import { useRef, useState, type ChangeEvent } from 'react';
import { errMsg } from '../utils/errMessage';
import { api } from '../api';

// Le paquet de sauvegarde est un blob JSON opaque côté front (le serveur en
// porte le schéma) ; on ne le manipule que pour le relayer à l'API.
type BackupBundle = Record<string, unknown>;

type Status =
  | { kind: 'idle' }
  | { kind: 'running'; label: string }
  | { kind: 'done'; label: string }
  | { kind: 'error'; message: string };

// Résultat du preview : tables disponibles dans le paquet.
type PreviewTable = { name: string; label: string; rows: number };
type PreviewResult = { meta: unknown; tables: PreviewTable[] };

// Action destructive en attente de confirmation.
type Pending =
  | { kind: 'reset' }
  | { kind: 'import' };

// État de la sélection d'import : paquet + preview + checkboxes.
type ImportSelection = {
  bundle: BackupBundle;
  fileName: string;
  preview: PreviewResult;
  selected: Set<string>; // noms des tables cochées
};

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
    title: 'Confirmer l\'import',
    body:
      'Les tables sélectionnées seront importées. Les tables non cochées seront ' +
      'ignorées (leur état actuel sera conservé, sauf si elles dépendent de tables ' +
      'importées qui ont été vidées).',
    confirmLabel: 'Importer',
  },
};

export function MaintenancePage() {
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [pending, setPending] = useState<Pending | null>(null);
  const [selection, setSelection] = useState<ImportSelection | null>(null);
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
      setStatus({ kind: 'error', message: errMsg(err, 'erreur') });
    }
  }

  // Sélection d'un fichier : on parse, on appelle le preview, puis on affiche
  // la sélection de tables (pas encore d'import effectif).
  async function pickImport(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = ''; // autorise la re-sélection du même fichier
    if (!file) return;
    try {
      setStatus({ kind: 'running', label: 'Analyse du paquet…' });
      const bundle = JSON.parse(await file.text()) as BackupBundle;
      const preview = await api.backup.preview(bundle);
      // Par défaut, toutes les tables sont cochées.
      const allSelected = new Set(preview.tables.filter((t) => t.rows > 0).map((t) => t.name));
      setSelection({ bundle, fileName: file.name, preview, selected: allSelected });
      setStatus({ kind: 'idle' });
    } catch (err) {
      setStatus({ kind: 'error', message: errMsg(err, 'fichier illisible') });
    }
  }

  // Bascule une table dans la sélection.
  function toggleTable(name: string) {
    setSelection((prev) => {
      if (!prev) return prev;
      const next = new Set(prev.selected);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return { ...prev, selected: next };
    });
  }

  // Tout cocher / tout décocher.
  function selectAll() {
    setSelection((prev) => {
      if (!prev) return prev;
      const all = prev.preview.tables.filter((t) => t.rows > 0).map((t) => t.name);
      const allSelected = all.every((n) => prev.selected.has(n));
      return { ...prev, selected: allSelected ? new Set() : new Set(all) };
    });
  }

  // Demande de confirmation → ouvre la modale.
  function requestImport() {
    if (!selection || selection.selected.size === 0) return;
    setPending({ kind: 'import' });
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
        setStatus({ kind: 'error', message: errMsg(err, 'erreur') });
      }
    } else if (action.kind === 'import' && selection) {
      const sel = selection;
      setSelection(null);
      setStatus({ kind: 'running', label: 'Import du paquet…' });
      try {
        // Tables à exclure = celles non cochées.
        const exclude = sel.preview.tables
          .filter((t) => !sel.selected.has(t.name))
          .map((t) => t.name);
        const result = await api.backup.importAll(sel.bundle, exclude);
        const imported = Object.entries(result.imported)
          .filter(([, n]) => n > 0)
          .map(([t, n]) => `${t}: ${n}`)
          .join(', ');
        setStatus({ kind: 'done', label: `Paquet « ${sel.fileName} » importé. ${imported}` });
      } catch (err) {
        setStatus({ kind: 'error', message: errMsg(err, 'erreur') });
      }
    }
  }

  // Annule la sélection en cours.
  function cancelSelection() {
    setSelection(null);
    setStatus({ kind: 'idle' });
  }

  const selectedCount = selection?.selected.size ?? 0;
  const totalRows = selection?.preview.tables
    .filter((t) => selection.selected.has(t.name))
    .reduce((sum, t) => sum + t.rows, 0) ?? 0;

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
            Restaure l'état depuis un paquet exporté.{' '}
            <strong>Vous choisissez les tables à importer.</strong>
          </p>
          <div className="upload__form">
            <button
              type="button"
              className="btn"
              onClick={() => fileInputRef.current?.click()}
              disabled={busy || selection !== null}
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

      {/* Panneau de sélection des tables à importer */}
      {selection && (
        <div className="import-preview" style={{ marginTop: 24 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
            <h2 style={{ margin: 0 }}>
              Tables du paquet « {selection.fileName} »
            </h2>
            <button type="button" className="btn" onClick={cancelSelection} disabled={busy}>
              Annuler
            </button>
          </div>

          <div style={{ marginBottom: 12 }}>
            <button type="button" className="btn" onClick={selectAll} disabled={busy}>
              {selection.preview.tables.filter((t) => t.rows > 0).every((t) => selection.selected.has(t.name))
                ? 'Tout décocher'
                : 'Tout cocher'}
            </button>
            <span style={{ marginLeft: 12, fontSize: '0.9em', opacity: 0.7 }}>
              {selectedCount} table(s) sélectionnée(s), {totalRows.toLocaleString()} ligne(s)
            </span>
          </div>

          <table className="table" style={{ marginBottom: 12 }}>
            <thead>
              <tr>
                <th style={{ width: 40 }}></th>
                <th>Table</th>
                <th>Libellé</th>
                <th style={{ textAlign: 'right' }}>Lignes</th>
              </tr>
            </thead>
            <tbody>
              {selection.preview.tables.map((t) => (
                <tr key={t.name} style={{ opacity: t.rows === 0 ? 0.4 : 1 }}>
                  <td>
                    <input
                      type="checkbox"
                      checked={selection.selected.has(t.name)}
                      onChange={() => toggleTable(t.name)}
                      disabled={busy || t.rows === 0}
                    />
                  </td>
                  <td><code>{t.name}</code></td>
                  <td>{t.label}</td>
                  <td style={{ textAlign: 'right' }}>{t.rows.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>

          <div className="form-actions">
            <button
              type="button"
              className="btn btn--danger"
              onClick={requestImport}
              disabled={busy || selectedCount === 0}
            >
              Importer {selectedCount} table(s)
            </button>
          </div>
        </div>
      )}

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
              {pending.kind === 'import' && selection && (
                <p className="page__meta">
                  Fichier : {selection.fileName} — {selectedCount} table(s), {totalRows.toLocaleString()} ligne(s)
                </p>
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
