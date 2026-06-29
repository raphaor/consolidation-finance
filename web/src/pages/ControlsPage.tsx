// Page « Contrôles de données » — vérifications configurables exécutées à la demande.
// Deux onglets : Bibliothèque (contrôles) et Jeux de contrôles (sets + exécution).
// Spec : docs/CONTROLES_DONNEES.md.

import { Fragment, useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../api';
import {
  OpSelect,
  SelectionValueField,
  TraverseField,
} from '../components/ConditionFields';
import { NULL_OPS } from '../components/operators';
import { PageHeader } from '../components/PageHeader';
import { SubTabs } from '../components/SubTabs';
import {
  DIM_TO_TABLE_FALLBACK,
  DimRefContext,
  buildDimToTable,
  type DimToTable,
} from '../hooks/useDimValues';
import { useDimensionMetadata } from '../hooks/useDimensionMetadata';
import { parseCondVal } from '../utils/conditionValue';
import { errMsg } from '../utils/errMessage';
import { formatOptionLabel, sortForDisplay } from '../utils/format';
import type {
  Characteristic,
  ConsolidationSummary,
  Control,
  ControlDefinition,
  ControlSet,
  ControlSetReport,
  ControlStatus,
  CustomReference,
  DimensionInfo,
  NativeEnum,
  SelectionCond,
} from '../types';

const ALL_LEVELS = ['raw', 'corporate', 'converted', 'consolidated'] as const;
const ASSERTION_TYPES = [
  { value: 'range', label: 'Seuil (range)' },
  { value: 'nonzero', label: 'Non nul' },
  { value: 'existence', label: 'Existence' },
  { value: 'equals', label: 'Égal à' },
] as const;
const METRICS = [
  { value: 'variation_pct', label: 'Variation %' },
  { value: 'variation_abs', label: 'Variation absolue' },
  { value: 'variation', label: 'Variation signée' },
] as const;
type Tab = 'biblio' | 'jeux';

// ── Helpers ──

function statusBadge(s: ControlStatus): string {
  switch (s) {
    case 'pass': return '✅';
    case 'warn': return '⚠️';
    case 'error': return '❌';
    case 'no_data': return '⬜';
  }
}

function worstStatus(statuses: ControlStatus[]): ControlStatus {
  if (statuses.includes('error')) return 'error';
  if (statuses.includes('warn')) return 'warn';
  if (statuses.includes('no_data')) return 'no_data';
  return 'pass';
}

// ── Types locaux pour le formulaire ──

interface AssertionForm {
  type: 'range' | 'nonzero' | 'existence' | 'equals';
  warn: string;
  error: string;
  target: string;
}

interface CompareForm {
  metric: string;
  baseline_consolidation_id: string;
  warn: string;
  error: string;
}

interface ControlForm {
  code: string;
  libelle: string;
  levels: string[];
  grain: string[];
  conditions: { dim: string; traverse: string; op: string; val: string }[];
  expression: string;
  assertions: AssertionForm[];
  compareEnabled: boolean;
  compare: CompareForm;
}

interface ControlSetDraft {
  code: string;
  libelle: string;
  controls: { code: string; selected: boolean }[];
}

function decodeTraverse(traverse: string): { via?: string; ref?: string; attr?: string } {
  if (!traverse) return {};
  const [kind, name] = traverse.split(':');
  if (kind === 'via') return { via: name };
  if (kind === 'ref') return { ref: name };
  if (kind === 'attr') return { attr: name };
  return {};
}

function encodeTraverse(sel: SelectionCond): string {
  if (sel.via) return `via:${sel.via}`;
  if (sel.ref) return `ref:${sel.ref}`;
  if (sel.attr) return `attr:${sel.attr}`;
  return '';
}

function condToForm(sel: SelectionCond) {
  const v = sel.val;
  let valStr: string;
  if (v == null) valStr = '';
  else if (typeof v === 'string') valStr = v;
  else if (Array.isArray(v)) valStr = v.join(',');
  else valStr = String(v);
  return {
    dim: sel.dim,
    traverse: encodeTraverse(sel),
    op: sel.op,
    val: valStr,
  };
}

function formToCond(f: { dim: string; traverse: string; op: string; val: string }): SelectionCond {
  const { via, ref, attr } = decodeTraverse(f.traverse);
  return {
    dim: f.dim,
    op: f.op,
    val: parseCondVal(f.op, f.val),
    via,
    ref,
    attr,
  };
}

function formToDef(f: ControlForm): ControlDefinition {
  return {
    levels: f.levels as ControlDefinition['levels'],
    grain: f.grain,
    selection: f.conditions.map(formToCond),
    expression: f.expression || null,
    assertions: f.assertions.map((a) => {
      if (a.type === 'range') return { type: 'range' as const, warn: Number(a.warn), error: Number(a.error) };
      if (a.type === 'equals') return { type: 'equals' as const, target: Number(a.target) };
      return { type: a.type };
    }),
    compare: f.compareEnabled
      ? {
          metric: f.compare.metric as 'variation_abs' | 'variation_pct' | 'variation',
          baseline_consolidation_id: f.compare.baseline_consolidation_id
            ? Number(f.compare.baseline_consolidation_id)
            : null,
          warn: Number(f.compare.warn),
          error: Number(f.compare.error),
        }
      : null,
  };
}

const EMPTY_ASSERTION: AssertionForm = { type: 'range', warn: '100', error: '1000', target: '' };
const EMPTY_COMPARE: CompareForm = { metric: 'variation_pct', baseline_consolidation_id: '', warn: '10', error: '50' };

const EMPTY_FORM: ControlForm = {
  code: '',
  libelle: '',
  levels: ['consolidated'],
  grain: [],
  conditions: [],
  expression: '',
  assertions: [{ ...EMPTY_ASSERTION }],
  compareEnabled: false,
  compare: { ...EMPTY_COMPARE },
};

// =====================================================================
// Page principale
// =====================================================================

export function ControlsPage() {
  const { dims, characteristics, customRefs, nativeEnums, error: metaError } =
    useDimensionMetadata();
  const [dimToTable, setDimToTable] = useState<DimToTable>(DIM_TO_TABLE_FALLBACK);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>('biblio');

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const refs = await api.references();
        if (!cancelled) setDimToTable(buildDimToTable(refs));
      } catch {
        if (!cancelled) setDimToTable(DIM_TO_TABLE_FALLBACK);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const banner = error ?? metaError;

  return (
    <DimRefContext.Provider value={dimToTable}>
    <div className="page">
      <PageHeader
        title="Contrôles de données"
        hint="Vérifications configurables : complétude, cohérence, variations. Exécutées à la demande."
      />
      {banner && <div className="banner banner--error">{banner}</div>}

      <SubTabs
        items={[
          { id: 'biblio', label: 'Bibliothèque' },
          { id: 'jeux', label: 'Jeux de contrôles' },
        ]}
        active={tab}
        onChange={setTab}
      />

      {tab === 'biblio' ? (
        <BiblioTab
          dims={dims}
          characteristics={characteristics}
          customRefs={customRefs}
          nativeEnums={nativeEnums}
          onError={setError}
        />
      ) : (
        <JeuxTab
          dims={dims}
          onError={setError}
        />
      )}
    </div>
    </DimRefContext.Provider>
  );
}

// =====================================================================
// Onglet Bibliothèque : liste + éditeur de contrôles
// =====================================================================

function BiblioTab({
  dims,
  characteristics,
  customRefs,
  nativeEnums,
  onError,
}: {
  dims: DimensionInfo[];
  characteristics: Characteristic[];
  customRefs: CustomReference[];
  nativeEnums: NativeEnum[];
  onError: (e: string | null) => void;
}) {
  const [controls, setControls] = useState<Control[]>([]);
  const [selected, setSelected] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState<ControlForm>({ ...EMPTY_FORM, assertions: [{ ...EMPTY_ASSERTION }] });
  const [saving, setSaving] = useState(false);
  const exprRef = useRef<HTMLInputElement>(null);

  const insertExprToken = useCallback((token: string) => {
    const input = exprRef.current;
    if (!input) return;
    const start = input.selectionStart ?? form.expression.length;
    const end = input.selectionEnd ?? start;
    const before = form.expression.slice(0, start);
    const after = form.expression.slice(end);
    const cursorOffset = token.indexOf('%');
    const clean = cursorOffset >= 0 ? token.replace('%', '') : token;
    const newExpr = before + clean + after;
    setForm((f) => ({ ...f, expression: newExpr }));
    requestAnimationFrame(() => {
      input.focus();
      const pos = cursorOffset >= 0 ? start + cursorOffset : start + clean.length;
      input.setSelectionRange(pos, pos);
    });
  }, [form.expression]);

  const reload = useCallback(async () => {
    try {
      setControls(await api.controls.list());
    } catch (e) {
      onError(errMsg(e));
    }
  }, [onError]);

  useEffect(() => { void reload(); }, [reload]);

  const open = useCallback(
    (ctrl: Control | 'new') => {
      onError(null);
      if (ctrl === 'new') {
        setSelected('new');
        setForm({ ...EMPTY_FORM, assertions: [{ ...EMPTY_ASSERTION }] });
      } else {
        setSelected(ctrl.code);
        const def = ctrl.definition;
        setForm({
          code: ctrl.code,
          libelle: ctrl.libelle ?? '',
          levels: [...def.levels],
          grain: [...def.grain],
          conditions: def.selection.map(condToForm),
          expression: def.expression ?? '',
          assertions: def.assertions.map((a) => {
            if (a.type === 'range') return { type: 'range' as const, warn: String(a.warn ?? ''), error: String(a.error ?? ''), target: '' };
            if (a.type === 'equals') return { type: 'equals' as const, warn: '', error: '', target: String(a.target ?? '') };
            return { type: a.type, warn: '', error: '', target: '' };
          }),
          compareEnabled: def.compare !== null,
          compare: def.compare
            ? {
                metric: def.compare.metric,
                baseline_consolidation_id: def.compare.baseline_consolidation_id?.toString() ?? '',
                warn: String(def.compare.warn),
                error: String(def.compare.error),
              }
            : { ...EMPTY_COMPARE },
        });
      }
    },
    [onError],
  );

  const save = useCallback(async () => {
    if (!form.code) return;
    setSaving(true);
    try {
      const def = formToDef(form);
      if (selected === 'new') {
        await api.controls.create({ code: form.code, libelle: form.libelle || undefined, definition: def });
      } else {
        await api.controls.update(form.code, { libelle: form.libelle || undefined, definition: def });
      }
      await reload();
      setSelected(null);
    } catch (e) {
      onError(errMsg(e));
    } finally {
      setSaving(false);
    }
  }, [form, selected, reload, onError]);

  const remove = useCallback(
    async (code: string) => {
      if (!confirm(`Supprimer le contrôle ${code} ?`)) return;
      try {
        await api.controls.remove(code);
        if (selected === code) setSelected(null);
        await reload();
      } catch (e) {
        onError(errMsg(e));
      }
    },
    [selected, reload, onError],
  );

  // Helpers formulaire
  const toggleLevel = (level: string) => {
    setForm((f) => ({
      ...f,
      levels: f.levels.includes(level) ? f.levels.filter((l) => l !== level) : [...f.levels, level],
    }));
  };
  const addCondForDim = (dim: string) => {
    setForm((f) => ({ ...f, conditions: [...f.conditions, { dim, traverse: '', op: '=', val: '' }] }));
  };
  const updateCond = (i: number, patch: Partial<{ dim: string; traverse: string; op: string; val: string }>) => {
    setForm((f) => ({
      ...f,
      conditions: f.conditions.map((c, idx) => (idx === i ? { ...c, ...patch } : c)),
    }));
  };
  const addAssertion = () => setForm((f) => ({ ...f, assertions: [...f.assertions, { ...EMPTY_ASSERTION }] }));
  const updateAssertion = (i: number, patch: Partial<AssertionForm>) => {
    setForm((f) => ({
      ...f,
      assertions: f.assertions.map((a, idx) => (idx === i ? { ...a, ...patch } : a)),
    }));
  };
  const removeAssertion = (i: number) => setForm((f) => ({ ...f, assertions: f.assertions.filter((_, idx) => idx !== i) }));
  const addGrainDim = (dim: string) => setForm((f) => ({ ...f, grain: [...f.grain, dim] }));
  const removeGrainDim = (dim: string) => setForm((f) => ({ ...f, grain: f.grain.filter((g) => g !== dim) }));

  return (
    <div style={{ display: 'flex', gap: 16, minHeight: 500 }}>
      {/* Liste */}
      <div style={{ width: 280, flexShrink: 0 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
          <h3 style={{ margin: 0 }}>Contrôles</h3>
          <button type="button" className="btn btn--primary btn--sm" onClick={() => open('new')}>
            + Nouveau
          </button>
        </div>
        <div className="table-wrap" style={{ maxHeight: 400, overflow: 'auto' }}>
          <table className="table table--compact">
            <tbody>
              {controls.map((c) => (
                <tr
                  key={c.code}
                  className={`row-selectable ${selected === c.code ? 'row-selected' : ''}`}
                  onClick={() => open(c)}
                >
                  <td>
                    <strong>{c.code}</strong>
                    {c.libelle && <span className="muted"> — {c.libelle}</span>}
                  </td>
                  <td style={{ width: 30, textAlign: 'right' }}>
                    <button
                      type="button"
                      className="btn btn--ghost btn--xs"
                      onClick={(e) => { e.stopPropagation(); void remove(c.code); }}
                      title="Supprimer"
                    >
                      ✕
                    </button>
                  </td>
                </tr>
              ))}
              {controls.length === 0 && (
                <tr><td className="muted">Aucun contrôle</td></tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Éditeur */}
      <div className="editor-pane">
        {selected !== null ? (
          <div>
            <h3>{selected === 'new' ? 'Nouveau contrôle' : `Modifier ${form.code}`}</h3>

            {/* Identité */}
            <div style={{ display: 'flex', gap: 12, marginBottom: 12 }}>
              <label className="field" style={{ flex: '0 0 180px' }}>
                <span>Code</span>
                <input type="text" value={form.code} disabled={selected !== 'new'}
                  onChange={(e) => setForm((f) => ({ ...f, code: e.target.value }))}
                  placeholder="ex. CTRL_IC_SOLD" />
              </label>
              <label className="field" style={{ flex: 1 }}>
                <span>Libellé</span>
                <input type="text" value={form.libelle}
                  onChange={(e) => setForm((f) => ({ ...f, libelle: e.target.value }))}
                  placeholder="ex. Élimination IC soldée" />
              </label>
            </div>

            {/* Niveaux */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Niveaux cibles</h4>
              <div style={{ display: 'flex', gap: 8 }}>
                {ALL_LEVELS.map((l) => (
                  <button key={l} type="button"
                    className={`rule-chip ${form.levels.includes(l) ? 'rule-chip--active' : ''}`}
                    onClick={() => toggleLevel(l)}>
                    {l}
                  </button>
                ))}
              </div>
            </div>

            {/* Grain */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Grain (dimensions de regroupement)</h4>
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 6 }}>
                {form.grain.map((g) => {
                  const libelle = dims.find((d) => d.name === g)?.label ?? '';
                  return (
                    <span key={g} className="rule-chip rule-chip--active" onClick={() => removeGrainDim(g)} title="Retirer">
                      {formatOptionLabel(g, libelle)} ✕
                    </span>
                  );
                })}
                {form.grain.length === 0 && <span className="muted">total (pas de grain)</span>}
              </div>
              <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                {dims.filter((d) => !form.grain.includes(d.name)).map((d) => (
                  <button key={d.name} type="button" className="rule-chip" onClick={() => addGrainDim(d.name)}>
                    + {formatOptionLabel(d.name, d.label)}
                  </button>
                ))}
              </div>
            </div>

            {/* Sélection */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Sélection (filtres)</h4>
              <p className="rule-section__hint">
                Les filtres réduisent les lignes agrégées. <strong>Pour un calcul entre valeurs</strong> (ex : actif − passif), utilisez <code>IF(colonne = 'valeur', ...)</code> dans l'expression.
              </p>
              {form.conditions.map((c, i) => {
                const viaOptions = characteristics.filter((ch) => ch.base_dimension === c.dim);
                const refOptions = customRefs.filter((r) => r.host_dimension === c.dim);
                const attrOptions = nativeEnums.filter((e) => e.host_dimension === c.dim);
                const { via, ref, attr } = decodeTraverse(c.traverse);
                const libelle = dims.find((d) => d.name === c.dim)?.label ?? '';
                return (
                  <div key={i} className="rule-condition rule-condition--compact">
                    <span className="rule-sel-dim" title={formatOptionLabel(c.dim, libelle)}>
                      {formatOptionLabel(c.dim, libelle)}
                    </span>
                    <TraverseField value={c.traverse} disabled={!c.dim}
                      viaOptions={viaOptions} refOptions={refOptions} attrOptions={attrOptions}
                      onSelect={(v) => updateCond(i, { traverse: v, val: '' })} />
                    <OpSelect value={c.op} onChange={(op) => updateCond(i, { op })} />
                    {!NULL_OPS.has(c.op) && (
                      <label className="field field--grow">
                        <span>Valeur</span>
                        <SelectionValueField
                          sel={{ dim: c.dim, op: c.op, val: c.val, via, ref, attr }}
                          customReferences={customRefs} nativeEnums={nativeEnums}
                          op={c.op} value={c.val}
                          onRawChange={(raw) => updateCond(i, { val: String(parseCondVal(c.op, raw) ?? '') })} />
                      </label>
                    )}
                    <button type="button" className="btn btn--ghost" title="Retirer"
                      onClick={() => setForm((f) => ({ ...f, conditions: f.conditions.filter((_, idx) => idx !== i) }))}>
                      ✕
                    </button>
                  </div>
                );
              })}
              {(() => {
                const present = new Set(form.conditions.map((c) => c.dim));
                const addable = sortForDisplay(dims.filter((d) => !present.has(d.name)), (d) => formatOptionLabel(d.name, d.label));
                return (
                  <div className="rule-dest-add">
                    <span className="rule-dest-add__label">Ajouter un filtre</span>
                    {addable.map((d) => (
                      <button key={d.name} type="button" className="rule-chip" onClick={() => addCondForDim(d.name)}>
                        + {formatOptionLabel(d.name, d.label)}
                      </button>
                    ))}
                  </div>
                );
              })()}
            </div>

            {/* Expression */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Expression d'agrégation</h4>
              <p className="rule-section__hint">
                SQL calculé sur les lignes filtrées, groupées par le grain. Par défaut <code>SUM(amount)</code>. Cliquez pour insérer au curseur.
              </p>
              {(() => {
                const selDims = form.conditions.map((c) => c.dim);
                const allCols = [...new Set(['amount', ...form.grain, ...selDims])];
                const funcs = [
                  { label: 'SUM()', insert: 'SUM(%)' },
                  { label: 'ABS()', insert: 'ABS(%)' },
                  { label: 'IF()', insert: "IF(% = '', , )" },
                  { label: 'MIN()', insert: 'MIN(%)' },
                  { label: 'MAX()', insert: 'MAX(%)' },
                  { label: 'ROUND()', insert: 'ROUND(%, 0)' },
                  { label: 'SAFE_DIV()', insert: 'SAFE_DIV(%, )' },
                ];
                return (
                  <>
                    <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 6 }}>
                      <span className="muted" style={{ fontSize: 11, lineHeight: '22px', marginRight: 4 }}>Fonctions :</span>
                      {funcs.map((f) => (
                        <button key={f.label} type="button" className="rule-chip" title={`Insérer ${f.insert}`}
                          onClick={() => insertExprToken(f.insert)}>
                          {f.label}
                        </button>
                      ))}
                    </div>
                    <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 6 }}>
                      <span className="muted" style={{ fontSize: 11, lineHeight: '22px', marginRight: 4 }}>Colonnes :</span>
                      {allCols.map((col) => (
                        <button key={col} type="button" className="rule-chip" onClick={() => insertExprToken(col)}>
                          {col}
                        </button>
                      ))}
                    </div>
                    <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 8 }}>
                      <span className="muted" style={{ fontSize: 11, lineHeight: '22px', marginRight: 4 }}>Opérateurs :</span>
                      {['+', '-', '*', '/', '=', '!=', ' AND ', ' OR '].map((op) => (
                        <button key={op} type="button" className="rule-chip" onClick={() => insertExprToken(op.trim())}>
                          {op.trim()}
                        </button>
                      ))}
                    </div>
                  </>
                );
              })()}
              <input ref={exprRef} type="text" value={form.expression}
                onChange={(e) => setForm((f) => ({ ...f, expression: e.target.value }))}
                placeholder="SUM(amount) — vide = somme brute"
                style={{ width: '100%', fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace', fontSize: 13 }} />
            </div>

            {/* Assertions */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Assertions</h4>
              {form.assertions.map((a, i) => (
                <div key={i} style={{ display: 'flex', gap: 8, alignItems: 'flex-end', marginBottom: 6 }}>
                  <label className="field" style={{ flex: '0 0 150px' }}>
                    <span>Type</span>
                    <select value={a.type} onChange={(e) => updateAssertion(i, { type: e.target.value as AssertionForm['type'] })}>
                      {ASSERTION_TYPES.map((t) => (<option key={t.value} value={t.value}>{t.label}</option>))}
                    </select>
                  </label>
                  {a.type === 'range' && (
                    <>
                      <label className="field" style={{ flex: '0 0 100px' }}><span>Warn</span><input type="number" value={a.warn} onChange={(e) => updateAssertion(i, { warn: e.target.value })} /></label>
                      <label className="field" style={{ flex: '0 0 100px' }}><span>Error</span><input type="number" value={a.error} onChange={(e) => updateAssertion(i, { error: e.target.value })} /></label>
                    </>
                  )}
                  {a.type === 'equals' && (
                    <label className="field" style={{ flex: '0 0 120px' }}><span>Cible</span><input type="number" value={a.target} onChange={(e) => updateAssertion(i, { target: e.target.value })} /></label>
                  )}
                  <button type="button" className="btn btn--ghost" onClick={() => removeAssertion(i)} title="Retirer">✕</button>
                </div>
              ))}
              <button type="button" className="rule-chip" onClick={addAssertion}>+ Ajouter une assertion</button>
            </div>

            {/* Comparaison */}
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">
                <label>
                  <input type="checkbox" checked={form.compareEnabled}
                    onChange={(e) => setForm((f) => ({ ...f, compareEnabled: e.target.checked }))} />{' '}
                  Comparaison inter-périodes
                </label>
              </h4>
              {form.compareEnabled && (
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <label className="field" style={{ flex: '0 0 160px' }}>
                    <span>Métrique</span>
                    <select value={form.compare.metric} onChange={(e) => setForm((f) => ({ ...f, compare: { ...f.compare, metric: e.target.value } }))}>
                      {METRICS.map((m) => (<option key={m.value} value={m.value}>{m.label}</option>))}
                    </select>
                  </label>
                  <label className="field" style={{ flex: '0 0 140px' }}>
                    <span>Baseline ID</span>
                    <input type="number" value={form.compare.baseline_consolidation_id}
                      onChange={(e) => setForm((f) => ({ ...f, compare: { ...f.compare, baseline_consolidation_id: e.target.value } }))}
                      placeholder="auto N-1" />
                  </label>
                  <label className="field" style={{ flex: '0 0 100px' }}><span>Warn %</span><input type="number" value={form.compare.warn} onChange={(e) => setForm((f) => ({ ...f, compare: { ...f.compare, warn: e.target.value } }))} /></label>
                  <label className="field" style={{ flex: '0 0 100px' }}><span>Error %</span><input type="number" value={form.compare.error} onChange={(e) => setForm((f) => ({ ...f, compare: { ...f.compare, error: e.target.value } }))} /></label>
                </div>
              )}
            </div>

            {/* Actions */}
            <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
              <button type="button" className="btn btn--primary" disabled={saving || !form.code} onClick={() => void save()}>
                {saving ? 'Enregistrement…' : 'Enregistrer'}
              </button>
              <button type="button" className="btn btn--ghost" onClick={() => setSelected(null)}>Fermer</button>
            </div>
          </div>
        ) : (
          <div className="muted" style={{ padding: 24, textAlign: 'center' }}>
            Sélectionnez un contrôle dans la liste ou créez-en un nouveau.
          </div>
        )}
      </div>
    </div>
  );
}

// =====================================================================
// Onglet Jeux : liste + éditeur + exécution + rapport
// =====================================================================

function JeuxTab({
  dims: _dims,
  onError,
}: {
  dims: DimensionInfo[];
  onError: (e: string | null) => void;
}) {
  const [controls, setControls] = useState<Control[]>([]);
  const [controlSets, setControlSets] = useState<ControlSet[]>([]);
  const [setEditing, setSetEditing] = useState<string | 'new' | null>(null);
  const [setDraft, setSetDraft] = useState<ControlSetDraft>({ code: '', libelle: '', controls: [] });
  const [csSaving, setCsSaving] = useState(false);
  const [report, setReport] = useState<ControlSetReport | null>(null);
  const [runPhase, setRunPhase] = useState('REEL');
  const [runEntryPeriod, setRunEntryPeriod] = useState('2026-12');
  const [runConsolidationId, setRunConsolidationId] = useState<string>('');
  const [running, setRunning] = useState(false);
  const [phases, setPhases] = useState<{ code: string; libelle: string }[]>([]);
  const [periods, setPeriods] = useState<{ code: string; libelle: string; type?: string }[]>([]);
  const [consolidations, setConsolidations] = useState<ConsolidationSummary[]>([]);

  const reload = useCallback(async () => {
    try {
      const [cList, sList, scList, pList, conList] = await Promise.all([
        api.controls.list(),
        api.controlSets.list(),
        api.masterData.list('scenario_categories'),
        api.masterData.list('periods'),
        api.consolidations.list(),
      ]);
      setControls(cList);
      setControlSets(sList);
      setPhases(scList as { code: string; libelle: string }[]);
      setPeriods(pList as { code: string; libelle: string; type?: string }[]);
      setConsolidations(conList);
    } catch (e) {
      onError(errMsg(e));
    }
  }, [onError]);

  useEffect(() => { void reload(); }, [reload]);

  const openSet = useCallback(
    (set: ControlSet | 'new') => {
      onError(null);
      setReport(null);
      if (set === 'new') {
        setSetEditing('new');
        setSetDraft({ code: '', libelle: '', controls: controls.map((c) => ({ code: c.code, selected: false })) });
      } else {
        setSetEditing(set.code);
        setSetDraft({
          code: set.code,
          libelle: set.libelle ?? '',
          controls: controls.map((c) => ({ code: c.code, selected: set.controls.some((sc) => sc.code === c.code) })),
        });
      }
    },
    [controls, onError],
  );

  const saveSet = useCallback(async () => {
    if (!setDraft.code) return;
    setCsSaving(true);
    try {
      const body = {
        libelle: setDraft.libelle || undefined,
        controls: setDraft.controls.filter((c) => c.selected).map((c, i) => ({ code: c.code, ord: i + 1 })),
      };
      if (setEditing === 'new') {
        await api.controlSets.create({ code: setDraft.code, ...body });
      } else {
        await api.controlSets.update(setDraft.code, body);
      }
      await reload();
      setSetEditing(null);
    } catch (e) {
      onError(errMsg(e));
    } finally {
      setCsSaving(false);
    }
  }, [setDraft, setEditing, reload, onError]);

  const removeSet = useCallback(
    async (code: string) => {
      if (!confirm(`Supprimer le jeu ${code} ?`)) return;
      try {
        await api.controlSets.remove(code);
        if (setEditing === code) setSetEditing(null);
        await reload();
      } catch (e) {
        onError(errMsg(e));
      }
    },
    [setEditing, reload, onError],
  );

  const runControlSet = useCallback(
    async (setCode: string) => {
      setRunning(true);
      setReport(null);
      try {
        const r = await api.controlSets.run(setCode, {
          phase: runPhase,
          entry_period: runEntryPeriod,
          consolidation_id: runConsolidationId ? Number(runConsolidationId) : undefined,
        });
        setReport(r);
      } catch (e) {
        onError(errMsg(e));
      } finally {
        setRunning(false);
      }
    },
    [runPhase, runEntryPeriod, runConsolidationId, onError],
  );

  const toggleSetControl = (code: string) => {
    setSetDraft((f) => ({
      ...f,
      controls: f.controls.map((c) => (c.code === code ? { ...c, selected: !c.selected } : c)),
    }));
  };
  const moveSetUp = (code: string) => {
    setSetDraft((f) => {
      const idx = f.controls.findIndex((c) => c.code === code);
      if (idx <= 0) return f;
      const arr = [...f.controls];
      [arr[idx - 1], arr[idx]] = [arr[idx], arr[idx - 1]];
      return { ...f, controls: arr };
    });
  };
  const moveSetDown = (code: string) => {
    setSetDraft((f) => {
      const idx = f.controls.findIndex((c) => c.code === code);
      if (idx < 0 || idx >= f.controls.length - 1) return f;
      const arr = [...f.controls];
      [arr[idx], arr[idx + 1]] = [arr[idx + 1], arr[idx]];
      return { ...f, controls: arr };
    });
  };

  return (
    <div style={{ display: 'flex', gap: 16, minHeight: 500 }}>
      {/* Liste */}
      <div style={{ width: 320, flexShrink: 0 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
          <h3 style={{ margin: 0 }}>Jeux</h3>
          <button type="button" className="btn btn--primary btn--sm" onClick={() => openSet('new')}>
            + Nouveau
          </button>
        </div>
        <div className="table-wrap" style={{ maxHeight: 300, overflow: 'auto' }}>
          <table className="table table--compact">
            <tbody>
              {controlSets.map((s) => (
                <tr
                  key={s.code}
                  className={`row-selectable ${setEditing === s.code ? 'row-selected' : ''}`}
                  onClick={() => openSet(s)}
                >
                  <td>
                    <strong>{s.code}</strong>
                    {s.libelle && <span className="muted"> — {s.libelle}</span>}
                    <br />
                    <span className="muted" style={{ fontSize: '0.8em' }}>
                      {s.controls.map((c) => c.code).join(', ')}
                    </span>
                  </td>
                  <td style={{ width: 60, textAlign: 'right' }}>
                    <button
                      type="button"
                      className="btn btn--sm btn--primary"
                      disabled={running}
                      onClick={(e) => { e.stopPropagation(); void runControlSet(s.code); }}
                      title="Exécuter"
                    >
                      ▶
                    </button>
                  </td>
                </tr>
              ))}
              {controlSets.length === 0 && (
                <tr><td className="muted">Aucun jeu</td></tr>
              )}
            </tbody>
          </table>
        </div>
        <div style={{ marginTop: 12 }}>
          <h4 style={{ margin: '0 0 6px', fontSize: 13 }}>Paramètres d'exécution</h4>
          <p className="rule-section__hint" style={{ marginBottom: 6 }}>
            Choisir une consolidation auto-remplit Phase et Période. Les 4 niveaux s'exécutent sur la même période.
          </p>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            <label className="field" style={{ flex: '1 1 180px' }}>
              <span>Consolidation</span>
              <select value={runConsolidationId} onChange={(e) => {
                const id = e.target.value;
                setRunConsolidationId(id);
                // Auto-remplir phase et période depuis la consolidation sélectionnée
                if (id) {
                  const conso = consolidations.find((c) => String(c.id) === id);
                  if (conso) {
                    setRunPhase(conso.phase);
                    setRunEntryPeriod(conso.exercice);
                  }
                }
              }}>
                <option value="">— (raw uniquement) —</option>
                {sortForDisplay(consolidations, (c) => c.libelle).map((c) => (
                  <option key={c.id} value={c.id}>{c.libelle}</option>
                ))}
              </select>
            </label>
            <label className="field" style={{ flex: '1 1 140px' }}>
              <span>Phase (raw)</span>
              <select value={runPhase} onChange={(e) => setRunPhase(e.target.value)} disabled={!!runConsolidationId}>
                {phases.map((p) => (
                  <option key={p.code} value={p.code}>{formatOptionLabel(p.code, p.libelle)}</option>
                ))}
                {phases.length === 0 && <option value={runPhase}>{runPhase}</option>}
              </select>
            </label>
            <label className="field" style={{ flex: '1 1 140px' }}>
              <span>Période (raw)</span>
              <select value={runEntryPeriod} onChange={(e) => setRunEntryPeriod(e.target.value)} disabled={!!runConsolidationId}>
                {sortForDisplay(
                  periods.filter((p) => !p.type || p.type === 'exercice'),
                  (p) => formatOptionLabel(p.code, p.libelle),
                ).map((p) => (
                  <option key={p.code} value={p.code}>{formatOptionLabel(p.code, p.libelle)}</option>
                ))}
                {periods.length === 0 && <option value={runEntryPeriod}>{runEntryPeriod}</option>}
              </select>
            </label>
          </div>
        </div>
      </div>

      {/* Éditeur / Rapport */}
      <div className="editor-pane">
        {setEditing !== null ? (
          <div>
            <h3>{setEditing === 'new' ? 'Nouveau jeu de contrôles' : `Modifier ${setDraft.code}`}</h3>
            <div style={{ display: 'flex', gap: 12, marginBottom: 12 }}>
              <label className="field" style={{ flex: '0 0 200px' }}>
                <span>Code</span>
                <input type="text" value={setDraft.code} disabled={setEditing !== 'new'}
                  onChange={(e) => setSetDraft((f) => ({ ...f, code: e.target.value }))}
                  placeholder="ex. CS_BILAN" />
              </label>
              <label className="field" style={{ flex: 1 }}>
                <span>Libellé</span>
                <input type="text" value={setDraft.libelle}
                  onChange={(e) => setSetDraft((f) => ({ ...f, libelle: e.target.value }))}
                  placeholder="ex. Contrôles bilan" />
              </label>
            </div>
            <div className="rule-section" style={{ marginBottom: 12 }}>
              <h4 className="rule-section__title">Contrôles inclus (checkbox + flèches pour ordonner)</h4>
              <div className="table-wrap">
                <table className="table table--compact">
                  <tbody>
                    {setDraft.controls.map((c) => {
                      const ctrl = controls.find((x) => x.code === c.code);
                      const selectedControls = setDraft.controls.filter((x) => x.selected);
                      const idx = selectedControls.indexOf(c);
                      return (
                        <tr key={c.code} className={c.selected ? 'row-selected' : ''} style={{ opacity: c.selected ? 1 : 0.5 }}>
                          <td style={{ width: 30 }}>
                            <input type="checkbox" checked={c.selected} onChange={() => toggleSetControl(c.code)} />
                          </td>
                          <td>
                            <strong>{c.code}</strong>
                            {ctrl?.libelle && <span className="muted"> — {ctrl.libelle}</span>}
                          </td>
                          <td style={{ width: 60, textAlign: 'right' }}>
                            {c.selected && (
                              <>
                                <button type="button" className="btn btn--ghost btn--xs" disabled={idx === 0} onClick={() => moveSetUp(c.code)} title="Monter">↑</button>
                                <button type="button" className="btn btn--ghost btn--xs" disabled={idx === selectedControls.length - 1} onClick={() => moveSetDown(c.code)} title="Descendre">↓</button>
                              </>
                            )}
                          </td>
                        </tr>
                      );
                    })}
                    {setDraft.controls.length === 0 && (
                      <tr><td className="muted">Aucun contrôle disponible — créez-en d'abord dans la Bibliothèque.</td></tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
            <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
              <button type="button" className="btn btn--primary" disabled={csSaving || !setDraft.code} onClick={() => void saveSet()}>
                {csSaving ? 'Enregistrement…' : 'Enregistrer'}
              </button>
              {setEditing !== 'new' && (
                <button type="button" className="btn btn--danger" onClick={() => void removeSet(setDraft.code)}>Supprimer</button>
              )}
              <button type="button" className="btn btn--ghost" onClick={() => setSetEditing(null)}>Fermer</button>
            </div>
          </div>
        ) : report ? (
          <ReportView report={report} onClose={() => setReport(null)} />
        ) : (
          <div className="muted" style={{ padding: 24, textAlign: 'center' }}>
            Sélectionnez un jeu pour l'éditer, ou exécutez-en un avec ▶.
          </div>
        )}
      </div>
    </div>
  );
}

// =====================================================================
// Vue rapport
// =====================================================================

function ReportView({ report, onClose }: { report: ControlSetReport; onClose: () => void }) {
  const [expanded, setExpanded] = useState<string | null>(null);

  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 12 }}>
        <h3 style={{ margin: 0 }}>
          Rapport — {report.set_code}
          <span className="muted" style={{ fontSize: '0.7em', marginLeft: 8 }}>
            {report.executed_at}
          </span>
        </h3>
        <button type="button" className="btn btn--ghost" onClick={onClose}>Fermer</button>
      </div>

      {/* Résumé */}
      <div style={{ display: 'flex', gap: 16, marginBottom: 16 }}>
        {Object.entries(report.summary.by_level).map(([level, s]) => (
          <div key={level} className="card" style={{ padding: '8px 12px' }}>
            <strong>{level}</strong>
            <div style={{ display: 'flex', gap: 8, fontSize: '0.9em' }}>
              <span>✅ {s.pass}</span>
              <span>⚠️ {s.warn}</span>
              <span>❌ {s.error}</span>
              <span>⬜ {s.no_data}</span>
            </div>
          </div>
        ))}
      </div>

      {/* Détails */}
      <table className="table table--compact">
        <thead>
          <tr>
            <th>Contrôle</th>
            <th>Niveaux</th>
            <th>Statut</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {report.details.map((d) => {
            const levelStatuses = Object.values(d.levels).map((l) => l.status);
            const worst = worstStatus(levelStatuses);
            const isExpanded = expanded === d.control_code;
            // Lignes détaillées avec grain non vide
            const detailRows = Object.entries(d.levels).flatMap(([lvl, lr]) =>
              lr.rows
                .filter((r) => Object.keys(r.grain).length > 0 || r.value !== null)
                .map((r, ri) => ({ lvl, r, ri })),
            );
            const hasDetails = detailRows.length > 0;
            return (
              <Fragment key={d.control_code}>
                <tr
                  className={hasDetails ? 'row-selectable' : ''}
                  onClick={hasDetails ? () => setExpanded(isExpanded ? null : d.control_code) : undefined}
                >
                  <td>
                    <strong>{d.control_code}</strong>
                    {d.control_libelle && <span className="muted"> — {d.control_libelle}</span>}
                  </td>
                  <td>
                    {Object.entries(d.levels).map(([lvl, lr]) => (
                      <span key={lvl} style={{ marginRight: 6 }}>
                        {statusBadge(lr.status)} {lvl}
                        {lr.error && <span className="muted" style={{ fontSize: '0.8em' }}> ({lr.error})</span>}
                      </span>
                    ))}
                  </td>
                  <td>{statusBadge(worst)}</td>
                  <td style={{ width: 30 }}>
                    {hasDetails && (isExpanded ? '▼' : '▶')}
                  </td>
                </tr>
                {isExpanded && detailRows.map(({ lvl, r, ri }) => (
                  <tr key={`${lvl}-${ri}`} className="row-detail">
                    <td style={{ paddingLeft: 24 }}>
                      {ri === 0 && <span className="muted">{lvl}</span>}
                    </td>
                    <td colSpan={2}>
                      {Object.entries(r.grain).map(([k, v]) => `${k}=${v ?? '—'}`).join(', ')}
                      {r.value !== null && (
                        <span className="muted" style={{ marginLeft: 8 }}>= {r.value.toLocaleString('fr-FR')}</span>
                      )}
                      {r.baseline !== null && (
                        <span className="muted" style={{ marginLeft: 8 }}>(base: {r.baseline.toLocaleString('fr-FR')})</span>
                      )}
                      {r.variation !== null && (
                        <span className="muted" style={{ marginLeft: 8 }}>Δ {r.variation.toFixed(2)}</span>
                      )}
                    </td>
                    <td>{statusBadge(r.status)}</td>
                  </tr>
                ))}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
