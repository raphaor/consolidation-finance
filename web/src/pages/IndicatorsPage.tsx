// Page « Indicateurs » — volet 2 du moteur de formules (docs/FORMULES.md §4).
// Deux sous-onglets : Postes (briques agrégées sur fact_entry) et Indicateurs
// (formules combinant des postes, calculées à un grain, avec preview live).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import type {
  Aggregate,
  AggregateCond,
  Characteristic,
  ConsolidationSummary,
  CustomReference,
  DimensionInfo,
  Indicator,
  IndicatorOperand,
  IndicatorPreview,
  NativeEnum,
} from '../types';
import { formatOptionLabel, sortForDisplay } from '../utils/format';

const LEVELS = ['corporate', 'converted', 'consolidated'];
const OPS = ['=', '!=', '>', '<', '>=', '<=', 'IN', 'IS NULL', 'IS NOT NULL'];
const NULL_OPS = new Set(['IS NULL', 'IS NOT NULL']);
const FUNCTIONS = ['MIN', 'MAX', 'SAFE_DIV', 'IF', 'ABS', 'ROUND'];
const FORMATS = ['nombre', 'pourcentage', 'ratio'];

type Tab = 'postes' | 'indicateurs';

export function IndicatorsPage() {
  const [tab, setTab] = useState<Tab>('postes');
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [characteristics, setCharacteristics] = useState<Characteristic[]>([]);
  const [customRefs, setCustomRefs] = useState<CustomReference[]>([]);
  const [nativeEnums, setNativeEnums] = useState<NativeEnum[]>([]);
  const [consolidations, setConsolidations] = useState<ConsolidationSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [d, c, r, e, cons] = await Promise.all([
          api.dimensions.list(),
          api.characteristics.list(),
          api.customReferences.list(),
          api.nativeEnums(),
          api.consolidations.list(),
        ]);
        setDims(sortForDisplay(d, (x) => formatOptionLabel(x.name, x.label)));
        setCharacteristics(c);
        setCustomRefs(r);
        setNativeEnums(e);
        setConsolidations(sortForDisplay(cons, (x) => x.libelle));
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    })();
  }, []);

  return (
    <div className="page">
      <div className="page__header">
        <h2>Indicateurs</h2>
        <p className="page__hint">
          Postes = sélections agrégées sur fact_entry. Indicateurs = formules
          combinant des postes, calculées à un grain. Un ratio n'est jamais sommé.
        </p>
      </div>
      {error && <div className="banner banner--error">{error}</div>}

      <div className="subtabs" style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        <button
          type="button"
          className={`tab ${tab === 'postes' ? 'tab--active' : ''}`}
          onClick={() => setTab('postes')}
        >
          Postes
        </button>
        <button
          type="button"
          className={`tab ${tab === 'indicateurs' ? 'tab--active' : ''}`}
          onClick={() => setTab('indicateurs')}
        >
          Indicateurs
        </button>
      </div>

      {tab === 'postes' ? (
        <PostesTab
          dims={dims}
          characteristics={characteristics}
          customRefs={customRefs}
          nativeEnums={nativeEnums}
          onError={setError}
        />
      ) : (
        <IndicateursTab dims={dims} consolidations={consolidations} onError={setError} />
      )}
    </div>
  );
}

// =====================================================================
// Sous-onglet Postes
// =====================================================================

interface CondDraft extends AggregateCond {
  traverse: string; // '' | 'attr:<col>' | 'via:<char>' | 'ref:<col>'
}

interface PosteForm {
  code: string;
  libelle: string;
  level: string;
  conditions: CondDraft[];
}

const EMPTY_POSTE: PosteForm = { code: '', libelle: '', level: 'consolidated', conditions: [] };

function PostesTab({
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
  const [aggregates, setAggregates] = useState<Aggregate[]>([]);
  const [selected, setSelected] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState<PosteForm>(EMPTY_POSTE);
  const [saving, setSaving] = useState(false);

  const reload = useCallback(async () => {
    try {
      setAggregates(await api.aggregates.list());
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }, [onError]);
  useEffect(() => {
    void reload();
  }, [reload]);

  const open = useCallback((a: Aggregate | 'new') => {
    onError(null);
    if (a === 'new') {
      setSelected('new');
      setForm(EMPTY_POSTE);
      return;
    }
    setSelected(a.code);
    setForm({
      code: a.code,
      libelle: a.libelle ?? '',
      level: a.level,
      conditions: (a.definition?.selection ?? []).map((c) => ({
        ...c,
        traverse: c.attr ? `attr:${c.attr}` : c.via ? `via:${c.via}` : c.ref ? `ref:${c.ref}` : '',
      })),
    });
  }, [onError]);

  // Options de traversée pour une dimension donnée.
  const traverseOptions = useCallback(
    (dim: string) => {
      const opts: { value: string; label: string }[] = [];
      nativeEnums.filter((e) => e.host_dimension === dim).forEach((e) =>
        opts.push({ value: `attr:${e.column}`, label: `attribut · ${e.column}` }),
      );
      characteristics.filter((c) => c.base_dimension === dim).forEach((c) =>
        opts.push({ value: `via:${c.code}`, label: `caractéristique · ${c.code}` }),
      );
      customRefs.filter((r) => r.host_dimension === dim).forEach((r) =>
        opts.push({ value: `ref:${r.column}`, label: `référence · ${r.column}` }),
      );
      // `(direct)` en tête, le reste trié alphabétiquement par libellé.
      return [{ value: '', label: '(direct)' }, ...sortForDisplay(opts, (o) => o.label)];
    },
    [nativeEnums, characteristics, customRefs],
  );

  const updateCond = (i: number, patch: Partial<CondDraft>) =>
    setForm((f) => ({
      ...f,
      conditions: f.conditions.map((c, idx) => (idx === i ? { ...c, ...patch } : c)),
    }));

  const save = useCallback(async () => {
    onError(null);
    setSaving(true);
    try {
      const selection: AggregateCond[] = form.conditions.map((c) => {
        const cond: AggregateCond = { dim: c.dim, op: c.op };
        if (!NULL_OPS.has(c.op)) cond.val = c.val;
        if (c.traverse.startsWith('attr:')) cond.attr = c.traverse.slice(5);
        else if (c.traverse.startsWith('via:')) cond.via = c.traverse.slice(4);
        else if (c.traverse.startsWith('ref:')) cond.ref = c.traverse.slice(4);
        return cond;
      });
      const body = { libelle: form.libelle || undefined, level: form.level, definition: { selection } };
      if (selected === 'new') await api.aggregates.create({ code: form.code, ...body });
      else if (selected) await api.aggregates.update(selected, body);
      await reload();
      setSelected(form.code);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [form, selected, reload, onError]);

  const remove = useCallback(
    async (code: string) => {
      if (!confirm(`Supprimer le poste « ${code} » ?`)) return;
      try {
        await api.aggregates.remove(code);
        await reload();
        if (selected === code) setSelected(null);
      } catch (e) {
        onError(e instanceof Error ? e.message : String(e));
      }
    },
    [reload, selected, onError],
  );

  return (
    <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start' }}>
      <div style={{ flex: '0 0 300px' }}>
        <button type="button" className="btn btn--primary" onClick={() => open('new')}>
          + Nouveau poste
        </button>
        <table className="table" style={{ marginTop: 12 }}>
          <thead>
            <tr>
              <th>Code</th>
              <th>Niveau</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {aggregates.map((a) => (
              <tr key={a.code} className={selected === a.code ? 'row--selected' : ''} style={{ cursor: 'pointer' }}>
                <td onClick={() => open(a)}>{a.code}</td>
                <td onClick={() => open(a)}>{a.level}</td>
                <td>
                  <button type="button" className="btn btn--ghost" onClick={() => remove(a.code)} title="Supprimer">
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {selected !== null && (
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', gap: 12 }}>
            <label className="field" style={{ flex: '0 0 180px' }}>
              <span>Code</span>
              <input
                type="text"
                value={form.code}
                disabled={selected !== 'new'}
                onChange={(e) => setForm((f) => ({ ...f, code: e.target.value }))}
                placeholder="ex. ca"
              />
            </label>
            <label className="field" style={{ flex: 1 }}>
              <span>Libellé</span>
              <input
                type="text"
                value={form.libelle}
                onChange={(e) => setForm((f) => ({ ...f, libelle: e.target.value }))}
                placeholder="ex. Chiffre d'affaires"
              />
            </label>
            <label className="field" style={{ flex: '0 0 160px' }}>
              <span>Niveau</span>
              <select value={form.level} onChange={(e) => setForm((f) => ({ ...f, level: e.target.value }))}>
                {LEVELS.map((l) => (
                  <option key={l} value={l}>
                    {l}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <h3 style={{ margin: '16px 0 6px' }}>Sélection</h3>
          {form.conditions.map((c, i) => (
            <div key={i} style={{ display: 'flex', gap: 6, marginBottom: 6, alignItems: 'flex-end', flexWrap: 'wrap' }}>
              <label className="field">
                <span>Dimension</span>
                <select value={c.dim} onChange={(e) => updateCond(i, { dim: e.target.value, traverse: '' })}>
                  <option value="">— choisir —</option>
                  {dims.map((d) => (
                    <option key={d.name} value={d.name}>
                      {formatOptionLabel(d.name, d.label)}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Traverser</span>
                <select value={c.traverse} onChange={(e) => updateCond(i, { traverse: e.target.value })}>
                  {traverseOptions(c.dim).map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span>Opérateur</span>
                <select value={c.op} onChange={(e) => updateCond(i, { op: e.target.value })}>
                  {OPS.map((o) => (
                    <option key={o} value={o}>
                      {o}
                    </option>
                  ))}
                </select>
              </label>
              {!NULL_OPS.has(c.op) && (
                <label className="field">
                  <span>Valeur{c.op === 'IN' ? ' (séparées par ,)' : ''}</span>
                  <input
                    type="text"
                    value={typeof c.val === 'string' ? c.val : ''}
                    onChange={(e) => updateCond(i, { val: e.target.value })}
                    placeholder={c.op === 'IN' ? '700,705' : 'ex. 700'}
                  />
                </label>
              )}
              <button
                type="button"
                className="btn btn--ghost"
                onClick={() => setForm((f) => ({ ...f, conditions: f.conditions.filter((_, idx) => idx !== i) }))}
              >
                ✕
              </button>
            </div>
          ))}
          <button
            type="button"
            className="btn btn--ghost"
            onClick={() =>
              setForm((f) => ({ ...f, conditions: [...f.conditions, { dim: '', op: '=', val: '', traverse: '' }] }))
            }
          >
            + Ajouter une condition
          </button>

          <div style={{ marginTop: 16, display: 'flex', gap: 8 }}>
            <button
              type="button"
              className="btn btn--primary"
              disabled={saving || !form.code}
              onClick={() => void save()}
            >
              {saving ? 'Enregistrement…' : 'Enregistrer'}
            </button>
            <button type="button" className="btn btn--ghost" onClick={() => setSelected(null)}>
              Fermer
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// =====================================================================
// Sous-onglet Indicateurs
// =====================================================================

interface IndicForm {
  code: string;
  libelle: string;
  expression: string;
  grain: string[];
  format: string;
}

const EMPTY_INDIC: IndicForm = { code: '', libelle: '', expression: '', grain: [], format: 'nombre' };

function IndicateursTab({
  dims,
  consolidations,
  onError,
}: {
  dims: DimensionInfo[];
  consolidations: ConsolidationSummary[];
  onError: (e: string | null) => void;
}) {
  const [indicators, setIndicators] = useState<Indicator[]>([]);
  const [operands, setOperands] = useState<IndicatorOperand[]>([]);
  const [selected, setSelected] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState<IndicForm>(EMPTY_INDIC);
  const [consolidation, setConsolidation] = useState<number | undefined>(undefined);
  const [preview, setPreview] = useState<IndicatorPreview | null>(null);
  const [saving, setSaving] = useState(false);
  const exprRef = useRef<HTMLTextAreaElement>(null);

  const reload = useCallback(async () => {
    try {
      const [list, ops] = await Promise.all([api.indicators.list(), api.indicators.operands()]);
      setIndicators(list);
      setOperands(ops);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    }
  }, [onError]);
  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    if (consolidation === undefined && consolidations.length) setConsolidation(consolidations[0].id);
  }, [consolidations, consolidation]);

  const open = useCallback((ind: Indicator | 'new') => {
    onError(null);
    if (ind === 'new') {
      setSelected('new');
      setForm(EMPTY_INDIC);
    } else {
      setSelected(ind.code);
      setForm({
        code: ind.code,
        libelle: ind.libelle ?? '',
        expression: ind.expression,
        grain: ind.grain ?? [],
        format: ind.format ?? 'nombre',
      });
    }
    setPreview(null);
  }, [onError]);

  // Preview live (débouncée) sur la consolidation sélectionnée.
  useEffect(() => {
    if (!form.expression.trim() || consolidation === undefined) {
      setPreview(null);
      return;
    }
    const handle = setTimeout(async () => {
      try {
        setPreview(
          await api.indicators.preview({
            expression: form.expression,
            grain: form.grain,
            consolidation_id: consolidation,
          }),
        );
      } catch (e) {
        setPreview({ ok: false, error: e instanceof Error ? e.message : String(e), rows: [] });
      }
    }, 350);
    return () => clearTimeout(handle);
  }, [form.expression, form.grain, consolidation]);

  const insert = useCallback((fragment: string) => {
    const ta = exprRef.current;
    setForm((f) => {
      const start = ta?.selectionStart ?? f.expression.length;
      const end = ta?.selectionEnd ?? f.expression.length;
      const next = f.expression.slice(0, start) + fragment + f.expression.slice(end);
      requestAnimationFrame(() => {
        if (ta) {
          const pos = start + fragment.length;
          ta.focus();
          ta.setSelectionRange(pos, pos);
        }
      });
      return { ...f, expression: next };
    });
  }, []);

  const save = useCallback(async () => {
    onError(null);
    setSaving(true);
    try {
      const body = {
        libelle: form.libelle || undefined,
        expression: form.expression,
        grain: form.grain,
        format: form.format,
      };
      if (selected === 'new') await api.indicators.create({ code: form.code, ...body });
      else if (selected) await api.indicators.update(selected, body);
      await reload();
      setSelected(form.code);
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [form, selected, reload, onError]);

  const remove = useCallback(
    async (code: string) => {
      if (!confirm(`Supprimer l'indicateur « ${code} » ?`)) return;
      try {
        await api.indicators.remove(code);
        await reload();
        if (selected === code) setSelected(null);
      } catch (e) {
        onError(e instanceof Error ? e.message : String(e));
      }
    },
    [reload, selected, onError],
  );

  const toggleGrain = (name: string) =>
    setForm((f) => ({
      ...f,
      grain: f.grain.includes(name) ? f.grain.filter((g) => g !== name) : [...f.grain, name],
    }));

  return (
    <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start' }}>
      <div style={{ flex: '0 0 280px' }}>
        <button type="button" className="btn btn--primary" onClick={() => open('new')}>
          + Nouvel indicateur
        </button>
        <table className="table" style={{ marginTop: 12 }}>
          <thead>
            <tr>
              <th>Code</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {indicators.map((ind) => (
              <tr key={ind.code} className={selected === ind.code ? 'row--selected' : ''} style={{ cursor: 'pointer' }}>
                <td onClick={() => open(ind)} title={ind.libelle ?? ''}>
                  {ind.code}
                </td>
                <td>
                  <button type="button" className="btn btn--ghost" onClick={() => remove(ind.code)} title="Supprimer">
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {selected !== null && (
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', gap: 12 }}>
            <label className="field" style={{ flex: '0 0 180px' }}>
              <span>Code</span>
              <input
                type="text"
                value={form.code}
                disabled={selected !== 'new'}
                onChange={(e) => setForm((f) => ({ ...f, code: e.target.value }))}
                placeholder="ex. marge_op"
              />
            </label>
            <label className="field" style={{ flex: 1 }}>
              <span>Libellé</span>
              <input
                type="text"
                value={form.libelle}
                onChange={(e) => setForm((f) => ({ ...f, libelle: e.target.value }))}
                placeholder="ex. Marge opérationnelle"
              />
            </label>
            <label className="field" style={{ flex: '0 0 150px' }}>
              <span>Format</span>
              <select value={form.format} onChange={(e) => setForm((f) => ({ ...f, format: e.target.value }))}>
                {FORMATS.map((fm) => (
                  <option key={fm} value={fm}>
                    {fm}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div style={{ margin: '12px 0 6px', display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {FUNCTIONS.map((fn) => (
              <button key={fn} type="button" className="chip chip--fn" onClick={() => insert(`${fn}()`)}>
                {fn}
              </button>
            ))}
          </div>

          <label className="field">
            <span>Formule</span>
            <textarea
              ref={exprRef}
              rows={3}
              value={form.expression}
              spellCheck={false}
              style={{ fontFamily: 'monospace', fontSize: 14 }}
              onChange={(e) => setForm((f) => ({ ...f, expression: e.target.value }))}
              placeholder="SAFE_DIV([resultat]; [ca])"
            />
          </label>

          {/* Grain */}
          <div style={{ marginTop: 8 }}>
            <div className="muted" style={{ marginBottom: 4 }}>Grain (dimensions de restitution) :</div>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 10 }}>
              {dims.map((d) => (
                <label key={d.name} className="field--inline" style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                  <input type="checkbox" checked={form.grain.includes(d.name)} onChange={() => toggleGrain(d.name)} />
                  <span>{d.name}</span>
                </label>
              ))}
            </div>
            {form.grain.length === 0 && <div className="muted" style={{ marginTop: 4 }}>Aucun grain → un total unique.</div>}
          </div>

          {/* Preview */}
          <div style={{ marginTop: 12 }}>
            <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end', marginBottom: 6 }}>
              <label className="field">
                <span>Consolidation (preview)</span>
                <select
                  value={consolidation ?? ''}
                  onChange={(e) => setConsolidation(e.target.value ? Number(e.target.value) : undefined)}
                >
                  {consolidations.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.libelle} ({c.phase} {c.exercice})
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <PreviewTable preview={preview} grain={form.grain} format={form.format} />
          </div>

          <div style={{ marginTop: 12, display: 'flex', gap: 8 }}>
            <button
              type="button"
              className="btn btn--primary"
              disabled={saving || !form.code || !form.expression}
              onClick={() => void save()}
            >
              {saving ? 'Enregistrement…' : 'Enregistrer'}
            </button>
            <button type="button" className="btn btn--ghost" onClick={() => setSelected(null)}>
              Fermer
            </button>
          </div>

          {/* Postes / indicateurs insérables */}
          <div style={{ marginTop: 20 }}>
            <h3 style={{ margin: '0 0 6px' }}>Postes & indicateurs disponibles</h3>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {operands.length === 0 && (
                <span className="muted">Aucun poste défini — créez-en dans l'onglet « Postes ».</span>
              )}
              {operands.map((op) => (
                <button
                  key={`${op.kind}:${op.token}`}
                  type="button"
                  className="chip"
                  title={`${op.kind} — ${op.label}`}
                  onClick={() => insert(`[${op.token}]`)}
                >
                  {op.token}
                  {op.label ? ` · ${op.label}` : ''}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function PreviewTable({
  preview,
  grain,
  format,
}: {
  preview: IndicatorPreview | null;
  grain: string[];
  format: string;
}) {
  const fmt = useMemo(
    () => (v: number | null) => {
      if (v === null || v === undefined) return '—';
      if (format === 'pourcentage') return `${(v * 100).toFixed(2)} %`;
      return Number(v.toFixed(6)).toString();
    },
    [format],
  );

  if (!preview) return <span className="muted">Saisissez une formule pour la prévisualiser.</span>;
  if (!preview.ok) return <span className="preview__error">⚠ {preview.error}</span>;
  return (
    <table className="table">
      <thead>
        <tr>
          {grain.map((g) => (
            <th key={g}>{g}</th>
          ))}
          <th>valeur</th>
        </tr>
      </thead>
      <tbody>
        {preview.rows.map((row, i) => (
          <tr key={i}>
            {grain.map((g) => (
              <td key={g}>{row.grain[g] ?? '—'}</td>
            ))}
            <td>{fmt(row.value)}</td>
          </tr>
        ))}
        {preview.rows.length === 0 && (
          <tr>
            <td colSpan={grain.length + 1} className="muted">
              Aucune ligne (pipeline non lancé pour cette consolidation ?).
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}
