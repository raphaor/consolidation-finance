// Page « Règles » : gère la bibliothèque de règles de consolidation, les
// jeux de règles (composition ordonnée + exécution) et les dimensions custom.
//
// Trois sous-onglets :
//   - Bibliothèque : CRUD sur les règles (définition JSON éditoriale)
//   - Jeux de règles : CRUD sur les rulesets + exécution → rapport
//   - Dimensions : gestion des dimensions custom (catégorie Analytical)

import {
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from 'react';
import {
  type ColumnDef as RTColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from '@tanstack/react-table';
import { api } from '../api';
import type {
  DimensionInfo,
  Operation,
  RuleDefinition,
  RuleResult,
  RuleSummary,
  RulesetDetail,
  RulesetItem,
  RulesetReport,
  RulesetSummary,
  ScopeCond,
  SelectionCond,
} from '../types';

// Constantes qui ne dépendent pas du registre des dimensions.
const SCOPE_DIMS = ['methode', 'pct_interet', 'pct_integration', 'entree', 'sortie'];
const LEVELS_LIST = ['corporate', 'reclassified', 'converted', 'consolidated'];
const OPS = ['=', '!=', '>', '<', '>=', '<=', 'IN', 'IS NULL', 'IS NOT NULL'];
const COEFF_TYPES = ['pct_integration', 'pct_interet', 'constant'];

const NULL_OPS = new Set(['IS NULL', 'IS NOT NULL']);

type Notice = { kind: 'success' | 'error'; text: string } | null;
type Subtab = 'biblio' | 'jeux' | 'dims';
type DestMode = 'inherit' | 'override' | 'null';

interface RuleDraft {
  code: string;
  libelle: string;
  definition: RuleDefinition;
}

interface RulesetDraft {
  code: string;
  libelle: string;
  items: RulesetItem[];
}

/// Calcul des listes dynamiques depuis le registre des dimensions :
/// - `selectionDims` : toutes les dims propagées + `level` (filtre fact_entry).
/// - `pilotableDims`  : dimensions pilotables via `destination`.
function deriveDims(dims: DimensionInfo[]) {
  const selectionDims: string[] = dims.map((d) => d.name);
  selectionDims.push('level');
  const pilotableDims: string[] = dims.filter((d) => d.pilotable).map((d) => d.name);
  return { selectionDims, pilotableDims };
}

function emptyScopeCond(): ScopeCond {
  return { target: 'entity', dim: SCOPE_DIMS[0], op: '=', val: '' };
}

function emptySelectionCond(selectionDims: string[]): SelectionCond {
  const dim = selectionDims[0] ?? 'account';
  return { dim, op: '=', val: '' };
}

function emptyOperation(seq: number, pilotableDims: string[]): Operation {
  const destination: Operation['destination'] = {};
  for (const dim of pilotableDims) {
    destination[dim] = { mode: 'inherit' };
  }
  return {
    seq,
    level: LEVELS_LIST[0],
    selection: [],
    coefficient: { type: COEFF_TYPES[0] },
    multiplicateur: 1,
    destination,
  };
}

function emptyDefinition(): RuleDefinition {
  return { scope: [], operations: [] };
}

function asDefinition(raw: object | undefined): RuleDefinition {
  if (!raw || typeof raw !== 'object') return emptyDefinition();
  const obj = raw as Record<string, unknown>;
  const scope = Array.isArray(obj['scope']) ? (obj['scope'] as ScopeCond[]) : [];
  const operations = Array.isArray(obj['operations'])
    ? (obj['operations'] as Operation[])
    : [];
  return { scope, operations };
}

// =================================================================
// Modal d'édition d'une règle
// =================================================================

interface RuleFormModalProps {
  initial: RuleDraft;
  isEdit: boolean;
  pilotableDims: string[];
  selectionDims: string[];
  onSubmit: (draft: RuleDraft) => Promise<void>;
  onCancel: () => void;
}

function RuleFormModal({
  initial,
  isEdit,
  pilotableDims,
  selectionDims,
  onSubmit,
  onCancel,
}: RuleFormModalProps) {
  const [draft, setDraft] = useState<RuleDraft>(initial);
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  function nextSeq(): number {
    return (
      draft.definition.operations.reduce((m, o) => Math.max(m, o.seq), 0) + 1
    );
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setSubmitting(true);
    setFormError(null);
    try {
      await onSubmit(draft);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : 'erreur');
    } finally {
      setSubmitting(false);
    }
  }

  // ---------- Scope ----------
  function updateScope(idx: number, patch: Partial<ScopeCond>) {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        scope: d.definition.scope.map((c, i) =>
          i === idx ? { ...c, ...patch } : c,
        ),
      },
    }));
  }
  function addScope() {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        scope: [...d.definition.scope, emptyScopeCond()],
      },
    }));
  }
  function removeScope(idx: number) {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        scope: d.definition.scope.filter((_, i) => i !== idx),
      },
    }));
  }

  // ---------- Operations ----------
  function updateOp(opIdx: number, patch: Partial<Operation>) {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        operations: d.definition.operations.map((o, i) =>
          i === opIdx ? { ...o, ...patch } : o,
        ),
      },
    }));
  }
  function addOp() {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        operations: [
          ...d.definition.operations,
          emptyOperation(nextSeq(), pilotableDims),
        ],
      },
    }));
  }
  function removeOp(opIdx: number) {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        operations: d.definition.operations.filter((_, i) => i !== opIdx),
      },
    }));
  }

  // ---------- Selection (dans une opération) ----------
  function updateSelection(
    opIdx: number,
    sIdx: number,
    patch: Partial<SelectionCond>,
  ) {
    setDraft((d) => {
      const operations = d.definition.operations.map((o, i) => {
        if (i !== opIdx) return o;
        return {
          ...o,
          selection: o.selection.map((s, j) =>
            j === sIdx ? { ...s, ...patch } : s,
          ),
        };
      });
      return { ...d, definition: { ...d.definition, operations } };
    });
  }
  function addSelection(opIdx: number) {
    setDraft((d) => {
      const operations = d.definition.operations.map((o, i) =>
        i === opIdx
          ? { ...o, selection: [...o.selection, emptySelectionCond(selectionDims)] }
          : o,
      );
      return { ...d, definition: { ...d.definition, operations } };
    });
  }
  function removeSelection(opIdx: number, sIdx: number) {
    setDraft((d) => {
      const operations = d.definition.operations.map((o, i) =>
        i === opIdx
          ? { ...o, selection: o.selection.filter((_, j) => j !== sIdx) }
          : o,
      );
      return { ...d, definition: { ...d.definition, operations } };
    });
  }

  // ---------- Destination (par dimension pilotable) ----------
  function updateDestination(
    opIdx: number,
    dim: string,
    patch: Partial<{ mode: DestMode; value: string }>,
  ) {
    setDraft((d) => {
      const operations = d.definition.operations.map((o, i) => {
        if (i !== opIdx) return o;
        const current = o.destination[dim] ?? { mode: 'inherit' as DestMode };
        return {
          ...o,
          destination: { ...o.destination, [dim]: { ...current, ...patch } },
        };
      });
      return { ...d, definition: { ...d.definition, operations } };
    });
  }

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal rule-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          {isEdit ? 'Éditer la règle' : 'Nouvelle règle'}
        </div>
        <form className="modal__body" onSubmit={submit}>
          <div className="form-grid">
            <label className="field">
              <span>Code •</span>
              <input
                type="text"
                value={draft.code}
                disabled={isEdit}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, code: e.target.value }))
                }
                required
              />
            </label>
            <label className="field">
              <span>Libellé</span>
              <input
                type="text"
                value={draft.libelle}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, libelle: e.target.value }))
                }
                required
              />
            </label>
          </div>

          {/* ---------- Scope ---------- */}
          <div className="rule-section">
            <h3 className="rule-section__title">Périmètre (scope)</h3>
            {draft.definition.scope.map((c, idx) => (
              <div key={idx} className="rule-condition">
                <label className="field">
                  <span>Cible</span>
                  <select
                    value={c.target}
                    onChange={(e) =>
                      updateScope(idx, {
                        target: e.target.value as ScopeCond['target'],
                      })
                    }
                  >
                    <option value="entity">entity</option>
                    <option value="partner">partner</option>
                  </select>
                </label>
                <label className="field">
                  <span>Dim</span>
                  <select
                    value={c.dim}
                    onChange={(e) => updateScope(idx, { dim: e.target.value })}
                  >
                    {SCOPE_DIMS.map((s) => (
                      <option key={s} value={s}>
                        {s}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>Op</span>
                  <select
                    value={c.op}
                    onChange={(e) => updateScope(idx, { op: e.target.value })}
                  >
                    {OPS.map((o) => (
                      <option key={o} value={o}>
                        {o}
                      </option>
                    ))}
                  </select>
                </label>
                {!NULL_OPS.has(c.op) && (
                  <label className="field">
                    <span>Valeur</span>
                    <input
                      type="text"
                      value={String(c.val ?? '')}
                      onChange={(e) =>
                        updateScope(idx, { val: e.target.value })
                      }
                    />
                  </label>
                )}
                <button
                  type="button"
                  className="btn btn--sm btn--danger"
                  onClick={() => removeScope(idx)}
                >
                  ✕
                </button>
              </div>
            ))}
            <button
              type="button"
              className="rule-add-btn"
              onClick={addScope}
            >
              + Ajouter une condition
            </button>
          </div>

          {/* ---------- Opérations ---------- */}
          <div className="rule-section">
            <h3 className="rule-section__title">Opérations</h3>
            {draft.definition.operations.map((op, opIdx) => (
              <div key={opIdx} className="rule-operation">
                <div className="rule-operation__head">
                  <label className="field">
                    <span>Seq</span>
                    <input
                      type="number"
                      value={op.seq}
                      readOnly
                      style={{ width: 64 }}
                    />
                  </label>
                  <label className="field">
                    <span>Level</span>
                    <select
                      value={op.level}
                      onChange={(e) => updateOp(opIdx, { level: e.target.value })}
                    >
                      {LEVELS_LIST.map((l) => (
                        <option key={l} value={l}>
                          {l}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="field">
                    <span>Coefficient</span>
                    <select
                      value={op.coefficient.type}
                      onChange={(e) =>
                        updateOp(opIdx, {
                          coefficient: { ...op.coefficient, type: e.target.value },
                        })
                      }
                    >
                      {COEFF_TYPES.map((c) => (
                        <option key={c} value={c}>
                          {c}
                        </option>
                      ))}
                    </select>
                  </label>
                  {op.coefficient.type === 'constant' && (
                    <label className="field">
                      <span>Valeur</span>
                      <input
                        type="number"
                        step="any"
                        value={op.coefficient.value ?? 0}
                        onChange={(e) =>
                          updateOp(opIdx, {
                            coefficient: {
                              ...op.coefficient,
                              value: Number(e.target.value),
                            },
                          })
                        }
                      />
                    </label>
                  )}
                  <label className="field">
                    <span>Multiplicateur</span>
                    <input
                      type="number"
                      step="any"
                      value={op.multiplicateur}
                      onChange={(e) =>
                        updateOp(opIdx, {
                          multiplicateur: Number(e.target.value),
                        })
                      }
                    />
                  </label>
                  <button
                    type="button"
                    className="btn btn--sm btn--danger"
                    onClick={() => removeOp(opIdx)}
                  >
                    Supprimer
                  </button>
                </div>

                {/* Sélection */}
                <div className="rule-section" style={{ marginTop: 0 }}>
                  <h4 className="rule-section__title">Sélection</h4>
                  {op.selection.map((s, sIdx) => (
                    <div key={sIdx} className="rule-condition">
                      <label className="field">
                        <span>Dim</span>
                        <select
                          value={s.dim}
                          onChange={(e) =>
                            updateSelection(opIdx, sIdx, { dim: e.target.value })
                          }
                        >
                          {selectionDims.map((d) => (
                            <option key={d} value={d}>
                              {d}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label className="field">
                        <span>Op</span>
                        <select
                          value={s.op}
                          onChange={(e) =>
                            updateSelection(opIdx, sIdx, { op: e.target.value })
                          }
                        >
                          {OPS.map((o) => (
                            <option key={o} value={o}>
                              {o}
                            </option>
                          ))}
                        </select>
                      </label>
                      {!NULL_OPS.has(s.op) && (
                        <label className="field">
                          <span>Valeur</span>
                          <input
                            type="text"
                            value={String(s.val ?? '')}
                            onChange={(e) =>
                              updateSelection(opIdx, sIdx, {
                                val: e.target.value,
                              })
                            }
                          />
                        </label>
                      )}
                      <button
                        type="button"
                        className="btn btn--sm btn--danger"
                        onClick={() => removeSelection(opIdx, sIdx)}
                      >
                        ✕
                      </button>
                    </div>
                  ))}
                  <button
                    type="button"
                    className="rule-add-btn"
                    onClick={() => addSelection(opIdx)}
                  >
                    + Ajouter une condition
                  </button>
                </div>

                {/* Destination */}
                <div className="rule-section" style={{ marginTop: 0 }}>
                  <h4 className="rule-section__title">Destination</h4>
                  {pilotableDims.map((dim) => {
                    const dest = op.destination[dim] ?? { mode: 'inherit' as DestMode };
                    return (
                      <div key={dim} className="rule-dest-row">
                        <span className="rule-dest-label">{dim}</span>
                        <select
                          value={dest.mode}
                          onChange={(e) =>
                            updateDestination(opIdx, dim, {
                              mode: e.target.value as DestMode,
                            })
                          }
                        >
                          <option value="inherit">inherit</option>
                          <option value="override">override</option>
                          <option value="null">null</option>
                        </select>
                        {dest.mode === 'override' && (
                          <input
                            type="text"
                            className="rule-dest-input"
                            placeholder={`valeur ${dim}`}
                            value={dest.value ?? ''}
                            onChange={(e) =>
                              updateDestination(opIdx, dim, {
                                value: e.target.value,
                              })
                            }
                          />
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            ))}
            <button type="button" className="rule-add-btn" onClick={addOp}>
              + Ajouter une opération
            </button>
          </div>

          {formError && (
            <div className="alert alert--error" style={{ marginTop: 12 }}>
              {formError}
            </div>
          )}
          <div className="form-actions">
            <button
              type="button"
              className="btn"
              onClick={onCancel}
              disabled={submitting}
            >
              Annuler
            </button>
            <button
              type="submit"
              className="btn btn--primary"
              disabled={submitting}
            >
              {submitting ? 'Enregistrement…' : 'Enregistrer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

// =================================================================
// Modal d'édition d'un jeu de règles
// =================================================================

interface RulesetFormModalProps {
  initial: RulesetDraft;
  isEdit: boolean;
  ruleOptions: RuleSummary[];
  onSubmit: (draft: RulesetDraft) => Promise<void>;
  onCancel: () => void;
}

function RulesetFormModal({
  initial,
  isEdit,
  ruleOptions,
  onSubmit,
  onCancel,
}: RulesetFormModalProps) {
  const [draft, setDraft] = useState<RulesetDraft>(initial);
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  function nextOrdre(): number {
    return draft.items.reduce((m, it) => Math.max(m, it.ordre), 0) + 1;
  }

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setSubmitting(true);
    setFormError(null);
    try {
      await onSubmit(draft);
    } catch (err) {
      setFormError(err instanceof Error ? err.message : 'erreur');
    } finally {
      setSubmitting(false);
    }
  }

  function addItem() {
    const ruleCode = ruleOptions[0]?.code ?? '';
    setDraft((d) => ({
      ...d,
      items: [...d.items, { ordre: nextOrdre(), rule_code: ruleCode }],
    }));
  }
  function updateItem(idx: number, patch: Partial<RulesetItem>) {
    setDraft((d) => ({
      ...d,
      items: d.items.map((it, i) => (i === idx ? { ...it, ...patch } : it)),
    }));
  }
  function removeItem(idx: number) {
    setDraft((d) => ({
      ...d,
      items: d.items.filter((_, i) => i !== idx),
    }));
  }
  function moveItem(idx: number, dir: -1 | 1) {
    setDraft((d) => {
      const target = idx + dir;
      if (target < 0 || target >= d.items.length) return d;
      const items = [...d.items];
      const [removed] = items.splice(idx, 1);
      items.splice(target, 0, removed);
      return {
        ...d,
        items: items.map((it, i) => ({ ...it, ordre: i + 1 })),
      };
    });
  }

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal rule-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          {isEdit ? 'Éditer le jeu de règles' : 'Nouveau jeu de règles'}
        </div>
        <form className="modal__body" onSubmit={submit}>
          <div className="form-grid">
            <label className="field">
              <span>Code •</span>
              <input
                type="text"
                value={draft.code}
                disabled={isEdit}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, code: e.target.value }))
                }
                required
              />
            </label>
            <label className="field">
              <span>Libellé</span>
              <input
                type="text"
                value={draft.libelle}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, libelle: e.target.value }))
                }
                required
              />
            </label>
          </div>

          <div className="rule-section">
            <h3 className="rule-section__title">Règles du jeu (ordonnées)</h3>
            {draft.items.map((it, idx) => (
              <div key={idx} className="rule-condition">
                <label className="field">
                  <span>Ordre</span>
                  <input
                    type="number"
                    value={it.ordre}
                    readOnly
                    style={{ width: 64 }}
                  />
                </label>
                <label className="field">
                  <span>Règle</span>
                  <select
                    value={it.rule_code}
                    onChange={(e) =>
                      updateItem(idx, { rule_code: e.target.value })
                    }
                  >
                    {ruleOptions.length === 0 && (
                      <option value="">(aucune règle)</option>
                    )}
                    {ruleOptions.map((r) => (
                      <option key={r.code} value={r.code}>
                        {r.code} — {r.libelle}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  className="btn btn--sm"
                  onClick={() => moveItem(idx, -1)}
                  disabled={idx === 0}
                  title="Monter"
                >
                  ↑
                </button>
                <button
                  type="button"
                  className="btn btn--sm"
                  onClick={() => moveItem(idx, 1)}
                  disabled={idx === draft.items.length - 1}
                  title="Descendre"
                >
                  ↓
                </button>
                <button
                  type="button"
                  className="btn btn--sm btn--danger"
                  onClick={() => removeItem(idx)}
                >
                  ✕
                </button>
              </div>
            ))}
            <button
              type="button"
              className="rule-add-btn"
              onClick={addItem}
              disabled={ruleOptions.length === 0}
            >
              + Ajouter une règle au jeu
            </button>
          </div>

          {formError && (
            <div className="alert alert--error" style={{ marginTop: 12 }}>
              {formError}
            </div>
          )}
          <div className="form-actions">
            <button
              type="button"
              className="btn"
              onClick={onCancel}
              disabled={submitting}
            >
              Annuler
            </button>
            <button
              type="submit"
              className="btn btn--primary"
              disabled={submitting}
            >
              {submitting ? 'Enregistrement…' : 'Enregistrer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

// =================================================================
// Sous-onglet « Bibliothèque »
// =================================================================

interface BibliothequeTabProps {
  dims: DimensionInfo[];
}

function BibliothequeTab({ dims }: BibliothequeTabProps) {
  const { pilotableDims, selectionDims } = useMemo(() => deriveDims(dims), [dims]);
  const [rules, setRules] = useState<RuleSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);
  const [form, setForm] = useState<
    | { mode: 'create' }
    | { mode: 'edit'; draft: RuleDraft }
    | null
  >(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await api.rules.list();
      setRules(rows);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setRules([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    setNotice(null);
    void load();
  }, [load]);

  const handleDelete = useCallback(
    async (code: string) => {
      if (!window.confirm(`Supprimer la règle « ${code} » ?`)) return;
      try {
        await api.rules.remove(code);
        setNotice({ kind: 'success', text: 'Règle supprimée.' });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [load],
  );

  const openEdit = useCallback(async (code: string) => {
    try {
      const detail = await api.rules.get(code);
      setForm({
        mode: 'edit',
        draft: {
          code: detail.code,
          libelle: detail.libelle,
          definition: asDefinition(detail.definition),
        },
      });
    } catch (err) {
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
    }
  }, []);

  async function handleSubmit(draft: RuleDraft) {
    if (form?.mode === 'create') {
      await api.rules.create({
        code: draft.code,
        libelle: draft.libelle,
        definition: draft.definition,
      });
      setNotice({ kind: 'success', text: 'Règle créée.' });
    } else {
      await api.rules.update(draft.code, {
        libelle: draft.libelle,
        definition: draft.definition,
      });
      setNotice({ kind: 'success', text: 'Règle mise à jour.' });
    }
    setForm(null);
    await load();
  }

  const columns = useMemo<RTColumnDef<RuleSummary>[]>(
    () => [
      { header: 'Code', accessorKey: 'code' },
      { header: 'Libellé', accessorKey: 'libelle' },
      {
        id: '__actions',
        header: 'Actions',
        enableSorting: false,
        cell: (info) => (
          <div className="row-actions">
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => void openEdit(info.row.original.code)}
            >
              Éditer
            </button>
            <button
              type="button"
              className="btn btn--sm btn--danger"
              onClick={() => void handleDelete(info.row.original.code)}
            >
              Supprimer
            </button>
          </div>
        ),
      },
    ],
    [openEdit, handleDelete],
  );

  const table = useReactTable({
    data: rules,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <>
      <div className="page__actions">
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => setForm({ mode: 'create' })}
          disabled={loading}
        >
          Nouvelle règle
        </button>
        <button
          type="button"
          className="btn"
          onClick={load}
          disabled={loading}
        >
          {loading ? 'Chargement…' : 'Rafraîchir'}
        </button>
      </div>

      <div className="page__meta">{rules.length} règle(s)</div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}
      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((header) => {
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th key={header.id}>
                      {header.isPlaceholder ? null : (
                        <button
                          type="button"
                          className={`th-sort ${canSort ? 'th-sort--sortable' : ''}`}
                          onClick={header.column.getToggleSortingHandler()}
                          disabled={!canSort}
                        >
                          {flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                          <span className="th-sort__mark">
                            {sorted === 'asc'
                              ? '▲'
                              : sorted === 'desc'
                                ? '▼'
                                : canSort
                                  ? '↕'
                                  : ''}
                          </span>
                        </button>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={columns.length}>
                  {loading ? 'Chargement…' : 'Aucune règle.'}
                </td>
              </tr>
            )}
            {table.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {form !== null && (
        <RuleFormModal
          initial={
            form.mode === 'edit'
              ? form.draft
              : { code: '', libelle: '', definition: emptyDefinition() }
          }
          isEdit={form.mode === 'edit'}
          pilotableDims={pilotableDims}
          selectionDims={selectionDims}
          onSubmit={handleSubmit}
          onCancel={() => setForm(null)}
        />
      )}
    </>
  );
}

// =================================================================
// Sous-onglet « Jeux de règles »
// =================================================================

function JeuxTab() {
  const [details, setDetails] = useState<RulesetDetail[]>([]);
  const [ruleOptions, setRuleOptions] = useState<RuleSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);
  const [form, setForm] = useState<
    | { mode: 'create' }
    | { mode: 'edit'; draft: RulesetDraft }
    | null
  >(null);
  const [report, setReport] = useState<RulesetReport | null>(null);
  const [running, setRunning] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [summaries, rules] = await Promise.all([
        api.rulesets.list(),
        api.rules.list(),
      ]);
      const full = await Promise.all(
        summaries.map((s: RulesetSummary) => api.rulesets.get(s.code)),
      );
      setDetails(full);
      setRuleOptions(rules);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setDetails([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    setNotice(null);
    void load();
  }, [load]);

  const openEdit = useCallback(async (code: string) => {
    try {
      const detail = await api.rulesets.get(code);
      setForm({
        mode: 'edit',
        draft: {
          code: detail.code,
          libelle: detail.libelle,
          items: detail.items,
        },
      });
    } catch (err) {
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
    }
  }, []);

  const handleDelete = useCallback(
    async (code: string) => {
      if (!window.confirm(`Supprimer le jeu « ${code} » ?`)) return;
      try {
        await api.rulesets.remove(code);
        setNotice({ kind: 'success', text: 'Jeu supprimé.' });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [load],
  );

  const handleRun = useCallback(async (code: string) => {
    setRunning(code);
    setReport(null);
    try {
      const r = await api.rulesets.run(code);
      setReport(r);
      setNotice({
        kind: 'success',
        text: `Exécution terminée : ${r.total_generated} ligne(s) générée(s).`,
      });
    } catch (err) {
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
    } finally {
      setRunning(null);
    }
  }, []);

  async function handleSubmit(draft: RulesetDraft) {
    const payloadItems = draft.items.map((it) => ({
      ordre: it.ordre,
      rule_code: it.rule_code,
    }));
    if (form?.mode === 'create') {
      await api.rulesets.create({
        code: draft.code,
        libelle: draft.libelle,
        items: payloadItems,
      });
      setNotice({ kind: 'success', text: 'Jeu créé.' });
    } else {
      await api.rulesets.update(draft.code, {
        libelle: draft.libelle,
        items: payloadItems,
      });
      setNotice({ kind: 'success', text: 'Jeu mis à jour.' });
    }
    setForm(null);
    await load();
  }

  const columns = useMemo<RTColumnDef<RulesetDetail>[]>(
    () => [
      { header: 'Code', accessorKey: 'code' },
      { header: 'Libellé', accessorKey: 'libelle' },
      {
        header: 'Nb règles',
        accessorKey: 'items',
        enableSorting: false,
        cell: (info) => (info.getValue() as RulesetItem[]).length,
      },
      {
        id: '__actions',
        header: 'Actions',
        enableSorting: false,
        cell: (info) => {
          const code = info.row.original.code;
          return (
            <div className="row-actions">
              <button
                type="button"
                className="btn btn--sm"
                onClick={() => void openEdit(code)}
              >
                Éditer
              </button>
              <button
                type="button"
                className="btn btn--sm btn--danger"
                onClick={() => void handleDelete(code)}
              >
                Supprimer
              </button>
              <button
                type="button"
                className="btn btn--sm btn--primary"
                onClick={() => void handleRun(code)}
                disabled={running !== null}
              >
                {running === code ? 'Exécution…' : 'Exécuter'}
              </button>
            </div>
          );
        },
      },
    ],
    [openEdit, handleDelete, handleRun, running],
  );

  const table = useReactTable({
    data: details,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <>
      <div className="page__actions">
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => setForm({ mode: 'create' })}
          disabled={loading}
        >
          Nouveau jeu
        </button>
        <button type="button" className="btn" onClick={load} disabled={loading}>
          {loading ? 'Chargement…' : 'Rafraîchir'}
        </button>
      </div>

      <div className="page__meta">{details.length} jeu(x)</div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}
      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((header) => {
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th key={header.id}>
                      {header.isPlaceholder ? null : (
                        <button
                          type="button"
                          className={`th-sort ${canSort ? 'th-sort--sortable' : ''}`}
                          onClick={header.column.getToggleSortingHandler()}
                          disabled={!canSort}
                        >
                          {flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                          <span className="th-sort__mark">
                            {sorted === 'asc'
                              ? '▲'
                              : sorted === 'desc'
                                ? '▼'
                                : canSort
                                  ? '↕'
                                  : ''}
                          </span>
                        </button>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={columns.length}>
                  {loading ? 'Chargement…' : 'Aucun jeu.'}
                </td>
              </tr>
            )}
            {table.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {report !== null && (
        <div className="rule-report">
          <h3 className="rule-report__title">
            Rapport d'exécution — {report.ruleset}
          </h3>
          <table className="grid">
            <thead>
              <tr>
                <th>Règle</th>
                <th>Niveau</th>
                <th>Lignes générées</th>
              </tr>
            </thead>
            <tbody>
              {report.rules.length === 0 && (
                <tr>
                  <td className="grid__empty" colSpan={3}>
                    Aucune ligne générée.
                  </td>
                </tr>
              )}
              {report.rules.map((r: RuleResult, i) => (
                <tr key={`${r.rule_code}-${r.level}-${i}`}>
                  <td className="grid__rowhead">{r.rule_code}</td>
                  <td>{r.level}</td>
                  <td className="num">{r.generated}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="rule-report__total">
            Total : {report.total_generated} ligne(s)
          </p>
        </div>
      )}

      {form !== null && (
        <RulesetFormModal
          initial={
            form.mode === 'edit'
              ? form.draft
              : { code: '', libelle: '', items: [] }
          }
          isEdit={form.mode === 'edit'}
          ruleOptions={ruleOptions}
          onSubmit={handleSubmit}
          onCancel={() => setForm(null)}
        />
      )}
    </>
  );
}

// =================================================================
// Sous-onglet « Dimensions »
// =================================================================

function DimensionsTab() {
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [sorting, setSorting] = useState<{ id: string; desc: boolean }[]>([]);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');
  const [newLabel, setNewLabel] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await api.dimensions.list();
      setDims(rows);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'erreur');
      setDims([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    setNotice(null);
    void load();
  }, [load]);

  const handleCreate = useCallback(
    async (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      setCreating(true);
      try {
        await api.dimensions.create({ name: newName, label: newLabel });
        setNotice({ kind: 'success', text: `Dimension « ${newName} » créée.` });
        setNewName('');
        setNewLabel('');
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      } finally {
        setCreating(false);
      }
    },
    [newName, newLabel, load],
  );

  const handleDelete = useCallback(
    async (name: string) => {
      if (!window.confirm(`Supprimer la dimension « ${name} » ?`)) return;
      try {
        await api.dimensions.remove(name);
        setNotice({ kind: 'success', text: `Dimension « ${name} » supprimée.` });
        await load();
      } catch (err) {
        setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
      }
    },
    [load],
  );

  const columns = useMemo<RTColumnDef<DimensionInfo>[]>(
    () => [
      { header: 'Nom technique', accessorKey: 'name' },
      { header: 'Libellé', accessorKey: 'label' },
      { header: 'Catégorie', accessorKey: 'category' },
      {
        header: 'Perso.',
        accessorKey: 'custom',
        cell: (info) => (info.getValue() ? 'oui' : 'non'),
      },
      {
        header: 'Pilotable',
        accessorKey: 'pilotable',
        cell: (info) => (info.getValue() ? 'oui' : 'non'),
      },
      {
        id: '__actions',
        header: 'Actions',
        enableSorting: false,
        cell: (info) => {
          const dim = info.row.original;
          if (!dim.custom) {
            return <span className="dim-locked" title="Dimension built-in verrouillée">—</span>;
          }
          return (
            <button
              type="button"
              className="btn btn--sm btn--danger"
              onClick={() => void handleDelete(dim.name)}
            >
              Supprimer
            </button>
          );
        },
      },
    ],
    [handleDelete],
  );

  const table = useReactTable({
    data: dims,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <>
      <div className="page__actions">
        <button type="button" className="btn" onClick={load} disabled={loading}>
          {loading ? 'Chargement…' : 'Rafraîchir'}
        </button>
      </div>

      <div className="page__meta">{dims.length} dimension(s)</div>

      {error && <div className="alert alert--error">Erreur : {error}</div>}
      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div className="table-wrap">
        <table className="grid">
          <thead>
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((header) => {
                  const canSort = header.column.getCanSort();
                  const sorted = header.column.getIsSorted();
                  return (
                    <th key={header.id}>
                      {header.isPlaceholder ? null : (
                        <button
                          type="button"
                          className={`th-sort ${canSort ? 'th-sort--sortable' : ''}`}
                          onClick={header.column.getToggleSortingHandler()}
                          disabled={!canSort}
                        >
                          {flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                          <span className="th-sort__mark">
                            {sorted === 'asc'
                              ? '▲'
                              : sorted === 'desc'
                                ? '▼'
                                : canSort
                                  ? '↕'
                                  : ''}
                          </span>
                        </button>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {table.getRowModel().rows.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={columns.length}>
                  {loading ? 'Chargement…' : 'Aucune dimension.'}
                </td>
              </tr>
            )}
            {table.getRowModel().rows.map((row) => (
              <tr key={row.id}>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id}>
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="rule-section" style={{ marginTop: 24 }}>
        <h3 className="rule-section__title">Ajouter une dimension</h3>
        <form className="form-grid" onSubmit={handleCreate}>
          <label className="field">
            <span>Nom technique •</span>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="ex : segment"
              pattern="[A-Za-z_][A-Za-z0-9_]{0,49}"
              title="Lettre ou _ en premier, puis alphanumérique / _ (max 50). Réservés : level, amount, id."
              required
            />
          </label>
          <label className="field">
            <span>Libellé</span>
            <input
              type="text"
              value={newLabel}
              onChange={(e) => setNewLabel(e.target.value)}
              placeholder="ex : Segment produit"
              required
            />
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn--primary" disabled={creating}>
              {creating ? 'Création…' : 'Créer la dimension'}
            </button>
          </div>
        </form>
      </div>
    </>
  );
}

// =================================================================
// Composant racine
// =================================================================

export function RulesPage() {
  const [subtab, setSubtab] = useState<Subtab>('biblio');
  const [dims, setDims] = useState<DimensionInfo[]>([]);

  // Charge les dimensions une fois pour toutes (Bibliothèque en a besoin pour
  // construire les listes dynamiques pilotableDims / selectionDims).
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await api.dimensions.list();
        if (!cancelled) setDims(rows);
      } catch {
        // silently ignore — l'onglet Dimensions affichera l'erreur
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Règles de consolidation</h1>
      </div>

      <div className="subtabs">
        <button
          type="button"
          className={`subtab ${subtab === 'biblio' ? 'subtab--active' : ''}`}
          onClick={() => setSubtab('biblio')}
        >
          Bibliothèque
        </button>
        <button
          type="button"
          className={`subtab ${subtab === 'jeux' ? 'subtab--active' : ''}`}
          onClick={() => setSubtab('jeux')}
        >
          Jeux de règles
        </button>
        <button
          type="button"
          className={`subtab ${subtab === 'dims' ? 'subtab--active' : ''}`}
          onClick={() => setSubtab('dims')}
        >
          Dimensions
        </button>
      </div>

      {subtab === 'biblio' && <BibliothequeTab dims={dims} />}
      {subtab === 'jeux' && <JeuxTab />}
      {subtab === 'dims' && <DimensionsTab />}
    </section>
  );
}
