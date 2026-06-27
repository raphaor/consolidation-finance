import { type FormEvent, useState } from 'react';

interface Props {
  oldCode: string;
  entityLabel: string;
  onConfirm: (newCode: string) => Promise<void>;
  onCancel: () => void;
}

export function RenameModal({ oldCode, entityLabel, onConfirm, onCancel }: Props) {
  const [newCode, setNewCode] = useState(oldCode);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const trimmed = newCode.trim();
    if (!trimmed || trimmed === oldCode) return;
    setSubmitting(true);
    setError(null);
    try {
      await onConfirm(trimmed);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setSubmitting(false);
    }
  }

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          Renommer {entityLabel} «&nbsp;{oldCode}&nbsp;»
        </div>
        <form className="modal__body" onSubmit={submit}>
          <label className="field">
            <span>Nouveau code •</span>
            <input
              type="text"
              value={newCode}
              onChange={(e) => setNewCode(e.target.value)}
              required
              // eslint-disable-next-line jsx-a11y/no-autofocus
              autoFocus
            />
          </label>
          {error && (
            <div className="alert alert--error" style={{ marginTop: 8 }}>
              {error}
            </div>
          )}
          <div className="form-actions">
            <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
              Annuler
            </button>
            <button
              type="submit"
              className="btn btn--primary"
              disabled={submitting || newCode.trim() === oldCode || !newCode.trim()}
            >
              {submitting ? 'Renommage…' : 'Renommer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
