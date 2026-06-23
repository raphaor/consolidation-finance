// Page « Saisie » : saisie manuelle d'écritures dans `stg_entry` (niveau raw).
//
// Deux sections :
//   1. Nouvelles saisies — batch de lignes éditables (grille inline type Excel),
//      ajout/retirer, validation locale puis POST /api/entries (insert-only).
//   2. Saisies manuelles enregistrées — lecture de stg_entry où source = MANUAL,
//      avec actions Éditer (PUT) et Supprimer (DELETE).
//
// Protections :
// - Insert-only côté création : aucune écriture existante ne peut être
//   écrasée par une nouvelle saisie.
// - Édition / suppression restreintes aux lignes source = MANUAL (refusé côté
//   back sur les imports CSV).
// - Champs obligatoires validés avant envoi (front + back).

import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from 'react';
import { api } from '../api';
import {
  DimRefProvider,
  useDimValues,
} from '../hooks/useDimValues';
import type { DimensionInfo, EntryInput, ReferenceInfo } from '../types';
import { formatAmount, formatInt, formatOptionLabel } from '../utils/format';
import { usePersistentState } from '../utils/usePersistentState';

// Colonnes dans l'ordre canonique du schéma (cf. DIM_COLS côté back).
// Les 8 premières + amount sont obligatoires ; les autres sont optionnelles.
const REQUIRED_DIMS = [
  'phase',
  'entity',
  'entry_period',
  'period',
  'account',
  'flow',
  'currency',
  'nature',
] as const;

const OPTIONAL_DIMS = ['partner', 'share', 'analysis', 'analysis2'] as const;

// Toutes les colonnes éditables (dims + amount), ordre de la grille.
const ALL_COLS = [...REQUIRED_DIMS, ...OPTIONAL_DIMS, 'amount'] as const;
type ColName = (typeof ALL_COLS)[number];

// Champs factorisables dans l'en-tête commun (généralement constants sur un
// lot de saisies : on les saisit une fois, ils pré-remplissent chaque ligne).
const COMMON_COLS: readonly ColName[] = [
  'phase',
  'entity',
  'entry_period',
  'period',
  'currency',
  'nature',
];

// Champs variables par ligne (affichés dans la grille allégée par défaut).
const VARIABLE_COLS: readonly ColName[] = [
  'account',
  'flow',
  'partner',
  'share',
  'analysis',
  'analysis2',
  'amount',
];

// Libellés courts pour les en-têtes de colonnes (les libellés complets viennent
// du registre des dimensions côté serveur ; en mode dégradé on retombe sur ces
// libellés courts).
const SHORT_LABELS: Record<ColName, string> = {
  phase: 'Phase',
  entity: 'Entité',
  entry_period: 'Exercice',
  period: 'Période',
  account: 'Compte',
  flow: 'Flux',
  currency: 'Devise',
  nature: 'Nature',
  partner: 'Partenaire',
  share: 'Titre',
  analysis: 'Analyse 1',
  analysis2: 'Analyse 2',
  amount: 'Montant',
};

const isRequired = (col: ColName) =>
  (REQUIRED_DIMS as readonly string[]).includes(col) || col === 'amount';

// Compteur local pour les ids de brouillon (stable par render).
let nextLocalId = 1;

interface DraftRow {
  localId: number;
  values: Record<ColName, string>;
}

function emptyDraftRow(seed: Partial<Record<ColName, string>> = {}): DraftRow {
  const base = Object.fromEntries(ALL_COLS.map((c) => [c, ''])) as Record<
    ColName,
    string
  >;
  return { localId: nextLocalId++, values: { ...base, ...seed } };
}

// Valeurs réutilisables d'une ligne de saisie → EntryInput pour l'API.
function toEntryInput(values: Record<ColName, string>): EntryInput {
  return {
    phase: values.phase ?? '',
    entity: values.entity ?? '',
    entry_period: values.entry_period ?? '',
    period: values.period ?? '',
    account: values.account ?? '',
    flow: values.flow ?? '',
    currency: values.currency ?? '',
    nature: values.nature ?? '',
    partner: values.partner ?? '',
    share: values.share ?? '',
    analysis: values.analysis ?? '',
    analysis2: values.analysis2 ?? '',
    amount: values.amount ?? '',
  };
}

type Notice = { kind: 'success' | 'error'; text: string } | null;

// ─────────────────────────────────────────────────────────────────────────────
//  Cellule de saisie d'une dimension
// ─────────────────────────────────────────────────────────────────────────────

/// Cellule : select si la dimension a une table master data associée (via le
/// graphe de références), input texte libre sinon (analysis, analysis2, custom).
function DimCell({
  dim,
  value,
  onChange,
  required,
  disabled,
}: {
  dim: ColName;
  value: string;
  onChange: (v: string) => void;
  required: boolean;
  disabled?: boolean;
}) {
  const { values, loading } = useDimValues(dim);
  const isEmpty = value.trim() === '';
  const className = `field-inline ${required && isEmpty ? 'field-inline--invalid' : ''}`;
  if (values.length === 0 && !loading) {
    // Pas de table associée → saisie texte libre (analysis, analysis2…).
    return (
      <input
        type={dim === 'amount' ? 'text' : 'text'}
        inputMode={dim === 'amount' ? 'decimal' : undefined}
        className={className}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        placeholder={dim === 'amount' ? '0,00' : ''}
      />
    );
  }
  return (
    <select
      className={className}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled || loading}
    >
      <option value="">{loading ? '…' : '—'}</option>
      {values.map((v) => (
        <option key={v.code} value={v.code}>
          {formatOptionLabel(v.code, v.libelle)}
        </option>
      ))}
    </select>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Section : batch de nouvelles saisies
// ─────────────────────────────────────────────────────────────────────────────

function SaisieBatch({
  common,
  onCommonChange,
  onSaved,
}: {
  // Valeurs courantes de l'en-tête commun (toutes les ColName, mais seules
  // celles de COMMON_COLS sont éditables via onCommonChange).
  common: Record<ColName, string>;
  onCommonChange: (col: ColName, v: string) => void;
  onSaved: () => void;
}) {
  // Les nouvelles lignes héritent des valeurs communes courantes.
  const [rows, setRows] = useState<DraftRow[]>(() => [emptyDraftRow(common)]);
  const [submitting, setSubmitting] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  // Toggle pour afficher les colonnes communes dans la grille (override au
  // cas par cas). Par défaut, la grille est allégée (seulement VARIABLE_COLS).
  const [showCommon, setShowCommon] = useState(false);

  // Colonnes réellement rendues dans la grille.
  const visibleCols: readonly ColName[] = showCommon ? ALL_COLS : VARIABLE_COLS;

  // Met à jour une colonne d'une ligne.
  const updateCell = useCallback(
    (localId: number, col: ColName, v: string) => {
      setRows((prev) =>
        prev.map((r) =>
          r.localId === localId ? { ...r, values: { ...r.values, [col]: v } } : r,
        ),
      );
    },
    [],
  );

  const addRow = useCallback(() => {
    setRows((prev) => [...prev, emptyDraftRow(common)]);
  }, [common]);

  const removeRow = useCallback((localId: number) => {
    setRows((prev) => prev.filter((r) => r.localId !== localId));
  }, []);

  // Applique les 6 valeurs communes à toutes les lignes existantes du batch.
  // Permet de resynchroniser rapidement après modification de l'en-tête.
  const applyCommonToAll = useCallback(() => {
    const commonSubset = Object.fromEntries(
      COMMON_COLS.map((c) => [c, common[c]]),
    ) as Record<ColName, string>;
    setRows((prev) =>
      prev.map((r) => ({ ...r, values: { ...r.values, ...commonSubset } })),
    );
  }, [common]);

  // Validation locale : champs obligatoires renseignés + montants numériques.
  // Le back re-valide et vérifie en plus les FK (références master data).
  const validate = useCallback((): string[] => {
    const errors: string[] = [];
    rows.forEach((r, i) => {
      const lineNo = i + 1;
      const missing: string[] = [];
      (ALL_COLS as readonly ColName[]).forEach((col) => {
        if (isRequired(col) && r.values[col].trim() === '') {
          missing.push(SHORT_LABELS[col]);
        }
      });
      if (missing.length > 0) {
        errors.push(
          `Ligne ${lineNo} : champ(s) obligatoire(s) manquant(s) — ${missing.join(', ')}.`,
        );
      }
      // Montant : parse point/virgule, non-vide.
      const amt = r.values.amount.trim();
      if (amt !== '') {
        const normalized = amt.replace(',', '.');
        if (Number.isNaN(Number(normalized))) {
          errors.push(`Ligne ${lineNo} : montant invalide (« ${amt} »).`);
        }
      }
    });
    return errors;
  }, [rows]);

  const submit = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      setNotice(null);
      const localErrors = validate();
      if (localErrors.length > 0) {
        setNotice({ kind: 'error', text: localErrors.join(' ') });
        return;
      }
      setSubmitting(true);
      try {
        const payload = rows.map((r) => toEntryInput(r.values));
        const res = await api.entriesMutations.create(payload);
        setNotice({
          kind: 'success',
          text: `${res.inserted} écriture(s) enregistrée(s) (IDs : ${res.ids.join(', ')}). Cible : stg_entry (niveau raw). Relancez le pipeline pour propager.`,
        });
        // Réinitialise avec une seule ligne vide (ré-initialise aussi les
        // valeurs communes courantes).
        setRows([emptyDraftRow(common)]);
        onSaved();
      } catch (err) {
        setNotice({
          kind: 'error',
          text: err instanceof Error ? err.message : 'erreur',
        });
      } finally {
        setSubmitting(false);
      }
    },
    [rows, validate, common, onSaved],
  );

  return (
    <form className="rule-section" onSubmit={submit}>
      <div className="rule-section__header">
        <h2 className="rule-section__title">Nouvelles saisies</h2>
        <p className="rule-section__hint">
          Une fois enregistrées, ces lignes vont dans <code>stg_entry</code>{' '}
          (niveau <code>raw</code>) avec <code>source&nbsp;=&nbsp;MANUAL</code>.
          Le pipeline n'est pas relancé automatiquement.
        </p>
      </div>

      {notice && (
        <div className={`alert ${notice.kind === 'error' ? 'alert--error' : 'alert--success'}`}>
          {notice.text}
        </div>
      )}

      {/* En-tête commun : 6 champs factorisés, pré-remplissent chaque nouvelle
          ligne au clic sur « Ajouter une ligne ». Bouton « Appliquer partout »
          pour resynchroniser les lignes existantes après modification. */}
      <div className="saisie-common">
        <div className="saisie-common__header">
          <span className="saisie-common__title">En-tête commun</span>
          <span className="saisie-common__hint">
            Pré-remplit chaque nouvelle ligne. Utiliser « Appliquer partout »
            pour propager aux lignes déjà saisies.
          </span>
        </div>
        <div className="form-grid">
          {COMMON_COLS.map((col) => (
            <label className="field" key={col}>
              <span>
                {SHORT_LABELS[col]}
                <span className="required">*</span>
              </span>
              <DimCell
                dim={col}
                value={common[col]}
                onChange={(v) => onCommonChange(col, v)}
                required
                disabled={submitting}
              />
            </label>
          ))}
        </div>
        <div className="saisie-common__actions">
          <button
            type="button"
            className="btn btn--sm"
            onClick={applyCommonToAll}
            disabled={submitting || rows.length === 0}
            title="Remplace les 6 champs communs de toutes les lignes par les valeurs de cet en-tête"
          >
            ↧ Appliquer partout
          </button>
          <label className="field field--check">
            <input
              type="checkbox"
              checked={showCommon}
              onChange={(e) => setShowCommon(e.target.checked)}
              disabled={submitting}
            />
            <span>Afficher les colonnes communes dans la grille</span>
          </label>
        </div>
      </div>

      <div className="table-wrap">
        <table className="grid grid--dense">
          <thead>
            <tr>
              {visibleCols.map((col) => (
                <th key={col}>
                  {SHORT_LABELS[col]}
                  {isRequired(col) && <span className="required">*</span>}
                </th>
              ))}
              <th aria-label="actions" />
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={visibleCols.length + 1}>
                  Aucune ligne. Cliquez « Ajouter une ligne ».
                </td>
              </tr>
            )}
            {rows.map((r) => (
              <tr key={r.localId}>
                {visibleCols.map((col) => (
                  <td key={col}>
                    <DimCell
                      dim={col}
                      value={r.values[col]}
                      onChange={(v) => updateCell(r.localId, col, v)}
                      required={isRequired(col)}
                      disabled={submitting}
                    />
                  </td>
                ))}
                <td>
                  <button
                    type="button"
                    className="btn btn--sm btn--danger"
                    onClick={() => removeRow(r.localId)}
                    disabled={submitting}
                    title="Retirer cette ligne"
                  >
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="form-actions">
        <button
          type="button"
          className="btn"
          onClick={addRow}
          disabled={submitting}
        >
          + Ajouter une ligne
        </button>
        <span className="form-actions__spacer" />
        <span className="form-actions__info">
          {rows.length} ligne(s) — ligne(s) {Math.min(1, rows.length)} à{' '}
          {rows.length}
        </span>
        <button
          type="submit"
          className="btn btn--primary"
          disabled={submitting || rows.length === 0}
        >
          {submitting ? 'Enregistrement…' : 'Enregistrer tout'}
        </button>
      </div>
    </form>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Section : saisies manuelles existantes (liste + edit/delete)
// ─────────────────────────────────────────────────────────────────────────────

type ManualRow = Record<string, unknown>;

function manualRowToInput(row: ManualRow): EntryInput {
  const s = (k: string) => (row[k] == null ? '' : String(row[k]));
  return {
    phase: s('phase'),
    entity: s('entity'),
    entry_period: s('entry_period'),
    period: s('period'),
    account: s('account'),
    flow: s('flow'),
    currency: s('currency'),
    nature: s('nature'),
    partner: s('partner'),
    share: s('share'),
    analysis: s('analysis'),
    analysis2: s('analysis2'),
    amount: s('amount'),
  };
}

function ManualList({
  refreshKey,
  onChanged,
}: {
  refreshKey: number;
  onChanged: () => void;
}) {
  const [rows, setRows] = useState<ManualRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: number; input: EntryInput } | null>(null);
  const [busy, setBusy] = useState<number | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.entries({
        level: 'raw',
        limit: 10_000,
        offset: 0,
        source: 'MANUAL',
      });
      setRows(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setRows([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, refreshKey]);

  const startEdit = (row: ManualRow) => {
    const id = Number(row.id);
    if (!Number.isFinite(id)) return;
    setEditing({ id, input: manualRowToInput(row) });
  };

  const doDelete = async (id: number) => {
    if (!window.confirm(`Supprimer la saisie manuelle #${id} ?`)) return;
    setBusy(id);
    try {
      await api.entriesMutations.remove(id);
      onChanged();
    } catch (err) {
      window.alert(err instanceof Error ? err.message : 'erreur');
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="rule-section">
      <div className="rule-section__header">
        <h2 className="rule-section__title">Saisies manuelles enregistrées</h2>
        <p className="rule-section__hint">
          Lecture de <code>stg_entry</code> où <code>source = MANUAL</code>.
          Édition / suppression réservées à ces lignes (protection des imports CSV).
        </p>
      </div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}

      <div className="table-wrap">
        <table className="grid grid--dense">
          <thead>
            <tr>
              <th>ID</th>
              {(ALL_COLS as readonly ColName[]).map((col) => (
                <th key={col}>{SHORT_LABELS[col]}</th>
              ))}
              <th aria-label="actions" />
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={ALL_COLS.length + 2}>
                  {loading ? 'Chargement…' : 'Aucune saisie manuelle.'}
                </td>
              </tr>
            )}
            {rows.map((row) => (
              <tr key={String(row.id)}>
                <td>
                  <code>{String(row.id)}</code>
                </td>
                {(ALL_COLS as readonly ColName[]).map((col) => (
                  <td key={col}>
                    {col === 'amount' ? (
                      <span className="num">
                        {formatAmount(Number(row[col] ?? 0))}
                      </span>
                    ) : (
                      String(row[col] ?? '')
                    )}
                  </td>
                ))}
                <td>
                  <button
                    type="button"
                    className="btn btn--sm"
                    onClick={() => startEdit(row)}
                    disabled={busy === Number(row.id)}
                    title="Éditer cette ligne"
                  >
                    ✎
                  </button>
                  <button
                    type="button"
                    className="btn btn--sm btn--danger"
                    onClick={() => doDelete(Number(row.id))}
                    disabled={busy === Number(row.id)}
                    title="Supprimer cette ligne"
                  >
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="page__meta">
        {formatInt(rows.length)} saisie(s) manuelle(s).
      </div>

      {editing && (
        <EditModal
          state={editing}
          onCancel={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            onChanged();
          }}
        />
      )}
    </section>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Modal d'édition d'une saisie manuelle existante
// ─────────────────────────────────────────────────────────────────────────────

function EditModal({
  state,
  onCancel,
  onSaved,
}: {
  state: { id: number; input: EntryInput };
  onCancel: () => void;
  onSaved: () => void;
}) {
  const [values, setValues] = useState<Record<ColName, string>>({
    ...(Object.fromEntries(ALL_COLS.map((c) => [c, ''])) as Record<ColName, string>),
    ...state.input,
  });
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      await api.entriesMutations.update(state.id, toEntryInput(values));
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          Éditer la saisie #{state.id}
        </div>
        <form className="modal__body" onSubmit={submit}>
          {error && <div className="alert alert--error">{error}</div>}
          <div className="form-grid">
            {(ALL_COLS as readonly ColName[]).map((col) => (
              <label className="field" key={col}>
                <span>
                  {SHORT_LABELS[col]}
                  {isRequired(col) && <span className="required">*</span>}
                </span>
                <DimCell
                  dim={col}
                  value={values[col]}
                  onChange={(v) => setValues((prev) => ({ ...prev, [col]: v }))}
                  required={isRequired(col)}
                  disabled={submitting}
                />
              </label>
            ))}
          </div>
          <div className="form-actions">
            <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
              Annuler
            </button>
            <button type="submit" className="btn btn--primary" disabled={submitting}>
              {submitting ? 'Enregistrement…' : 'Enregistrer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Page Saisie (racine)
// ─────────────────────────────────────────────────────────────────────────────

export function SaisiePage() {
  // Pré-remplissage de l'en-tête commun : on réutilise les filtres partagés
  // avec la page Écritures (clés identiques) pour que phase/entity/période
  // soient cohérents entre les deux pages. Currency et nature sont propres à
  // la saisie (la page Écritures ne filtre pas sur ces dimensions).
  const [phase, setPhase] = usePersistentState('ecritures.phase', '');
  const [entity, setEntity] = usePersistentState('ecritures.entity', '');
  const [entryPeriod, setEntryPeriod] = usePersistentState('ecritures.entryPeriod', '');
  const [period, setPeriod] = usePersistentState('ecritures.period', '');
  const [currency, setCurrency] = usePersistentState('saisie.currency', '');
  const [nature, setNature] = usePersistentState('saisie.nature', '');

  // L'objet `common` expose toutes les ColName pour qu'il puisse servir de
  // seed direct à emptyDraftRow ; seules les 6 colonnes communes sont non-vides.
  const common: Record<ColName, string> = useMemo(() => {
    const base = Object.fromEntries(ALL_COLS.map((c) => [c, ''])) as Record<
      ColName,
      string
    >;
    base.phase = phase;
    base.entity = entity;
    base.entry_period = entryPeriod;
    base.period = period;
    base.currency = currency;
    base.nature = nature;
    return base;
  }, [phase, entity, entryPeriod, period, currency, nature]);

  // Setter unique pour l'en-tête commun (préserve la persistence de chaque
  // champ via son usePersistentState dédié).
  const onCommonChange = useCallback(
    (col: ColName, v: string) => {
      switch (col) {
        case 'phase':
          setPhase(v);
          break;
        case 'entity':
          setEntity(v);
          break;
        case 'entry_period':
          setEntryPeriod(v);
          break;
        case 'period':
          setPeriod(v);
          break;
        case 'currency':
          setCurrency(v);
          break;
        case 'nature':
          setNature(v);
          break;
        default:
          // Les autres colonnes ne sont pas éditables via l'en-tête commun.
          break;
      }
    },
    [setPhase, setEntity, setEntryPeriod, setPeriod, setCurrency, setNature],
  );

  // Chargement du graphe de références pour alimenter les listes déroulantes
  // via DimRefProvider. Si l'API est injoignable, fallback côté hook.
  const [references, setReferences] = useState<ReferenceInfo[] | null>(null);
  const [dimensions, setDimensions] = useState<DimensionInfo[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const refs = await api.references();
        if (!cancelled) setReferences(refs);
      } catch {
        if (!cancelled) setReferences(null);
      }
    })();
    void (async () => {
      try {
        const dims = await api.dimensions.list();
        if (!cancelled) setDimensions(dims);
      } catch {
        if (!cancelled) setDimensions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const triggerRefresh = useCallback(() => setRefreshKey((k) => k + 1), []);

  return (
    <DimRefProvider references={references}>
      <section className="page">
        <div className="page__header">
          <h1 className="page__title">Saisie d'écritures</h1>
          <div className="page__actions">
            <span className="page__meta">
              {dimensions.length} dimension(s) chargée(s).
            </span>
          </div>
        </div>

        <SaisieBatch
          common={common}
          onCommonChange={onCommonChange}
          onSaved={triggerRefresh}
        />
        <ManualList refreshKey={refreshKey} onChanged={triggerRefresh} />
      </section>
    </DimRefProvider>
  );
}
