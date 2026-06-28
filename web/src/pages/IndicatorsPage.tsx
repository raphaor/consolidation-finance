// Page « Indicateurs » — volet 2 du moteur de formules (docs/FORMULES.md §4).
// Deux sous-onglets : Postes (briques agrégées sur fact_entry) et Indicateurs
// (formules combinant des postes, calculées à un grain, avec preview live).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { FormulaEditor, type FormulaEditorHandle } from '../components/FormulaEditor';
import { LibraryList } from '../components/LibraryList';
import { OperandPalette } from '../components/OperandPalette';
import {
  OpSelect,
  SelectionValueField,
  TraverseField,
} from '../components/ConditionFields';
import { NULL_OPS, OP_SYMBOL } from '../components/operators';
import { PageHeader } from '../components/PageHeader';
import { useCrudResource } from '../hooks/useCrudResource';
import { useDimensionMetadata } from '../hooks/useDimensionMetadata';
import { parseCondVal } from '../utils/conditionValue';
import { errMsg } from '../utils/errMessage';
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
import {
  FORMULA_FUNCTIONS,
  formatFormulaValue,
  formatOptionLabel,
  sortForDisplay,
} from '../utils/format';

const LEVELS = ['corporate', 'converted', 'consolidated'];
const FORMATS = ['nombre', 'pourcentage', 'ratio'];

// Postes et Indicateurs sont désormais deux pages distinctes (groupe Calculs),
// au même niveau que Coefficients — plus de sous-onglets internes. Chacune
// charge seulement les données dont son volet a besoin.

export function PostesPage() {
  const { dims, characteristics, customRefs, nativeEnums, error: metaError } =
    useDimensionMetadata();
  const [error, setError] = useState<string | null>(null);
  const banner = error ?? metaError;

  return (
    <div className="page">
      <PageHeader
        title="Postes"
        hint="Postes = sélections agrégées sur fact_entry, briques de base des indicateurs."
      />
      {banner && <div className="banner banner--error">{banner}</div>}

      <PostesTab
        dims={dims}
        characteristics={characteristics}
        customRefs={customRefs}
        nativeEnums={nativeEnums}
        onError={setError}
      />
    </div>
  );
}

export function IndicateursPage() {
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [consolidations, setConsolidations] = useState<ConsolidationSummary[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        const [d, cons] = await Promise.all([
          api.dimensions.list(),
          api.consolidations.list(),
        ]);
        setDims(sortForDisplay(d, (x) => formatOptionLabel(x.name, x.label)));
        setConsolidations(sortForDisplay(cons, (x) => x.libelle));
      } catch (err) {
        setError(errMsg(err));
      }
    })();
  }, []);

  return (
    <div className="page">
      <PageHeader
        title="Indicateurs"
        hint="Indicateurs = formules combinant des postes, calculées à un grain. Un ratio n'est jamais sommé."
      />
      {error && <div className="banner banner--error">{error}</div>}

      <IndicateursTab dims={dims} consolidations={consolidations} onError={setError} />
    </div>
  );
}

// =====================================================================
// Sous-onglet Postes
// =====================================================================

// Condition de sélection en cours d'édition. `traverse` condense le mode de
// traversée (`''` | `via:<char>` | `ref:<col>` | `attr:<col>`) ; les champs
// `via`/`ref`/`attr` réellement envoyés au moteur en sont dérivés au save.
interface CondDraft {
  dim: string;
  op: string;
  val: unknown;
  traverse: string;
}

// Décode `traverse` en (via, ref, attr) pour la résolution des valeurs
// (`useSelectionValues`) et pour l'enregistrement.
function decodeTraverse(traverse: string): { via?: string; ref?: string; attr?: string } {
  if (traverse.startsWith('via:')) return { via: traverse.slice(4) };
  if (traverse.startsWith('ref:')) return { ref: traverse.slice(4) };
  if (traverse.startsWith('attr:')) return { attr: traverse.slice(5) };
  return {};
}

// Phrase de relecture d'un poste, générée depuis les conditions (aide à valider
// sans déchiffrer chaque champ). Même esprit que `summarizeOperation` (règles).
function summarizePoste(level: string, conds: CondDraft[]): string {
  const meaningful = conds.filter(
    (c) =>
      c.dim &&
      (NULL_OPS.has(c.op) ||
        (Array.isArray(c.val) ? c.val.length > 0 : String(c.val ?? '').trim() !== '')),
  );
  const sel =
    meaningful.length === 0
      ? 'toutes les écritures'
      : 'les écritures où ' +
        meaningful
          .map((c) => {
            const sym = OP_SYMBOL[c.op] ?? c.op;
            const { via, ref, attr } = decodeTraverse(c.traverse);
            const trav = via ? `.${via}` : ref ? `.${ref}` : attr ? `.${attr}` : '';
            let v = '';
            if (!NULL_OPS.has(c.op)) {
              v = Array.isArray(c.val) ? `{${c.val.join(', ')}}` : String(c.val ?? '') || '∅';
            }
            return `${c.dim}${trav} ${sym}${v ? ` ${v}` : ''}`.trim();
          })
          .join(' et ');
  return `Sur ${level}, somme des montants de ${sel}.`;
}

interface PosteForm {
  code: string;
  libelle: string;
  level: string;
  conditions: CondDraft[];
}

const EMPTY_POSTE: PosteForm = { code: '', libelle: '', level: 'consolidated', conditions: [] };

// Corps d'API d'un poste (hors code) dérivé du formulaire : conditions → sélection.
function posteToBody(f: PosteForm) {
  const selection: AggregateCond[] = f.conditions.map((c) => {
    const cond: AggregateCond = { dim: c.dim, op: c.op, ...decodeTraverse(c.traverse) };
    if (!NULL_OPS.has(c.op)) cond.val = c.val;
    return cond;
  });
  return { libelle: f.libelle || undefined, level: f.level, definition: { selection } };
}

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
  const {
    items: aggregates,
    selected,
    setSelected,
    form,
    setForm,
    saving,
    open,
    save,
    remove,
  } = useCrudResource<Aggregate, PosteForm>({
    list: api.aggregates.list,
    keyOf: (a) => a.code,
    emptyForm: EMPTY_POSTE,
    toForm: (a) => ({
      code: a.code,
      libelle: a.libelle ?? '',
      level: a.level,
      conditions: (a.definition?.selection ?? []).map((c) => ({
        dim: c.dim,
        op: c.op,
        val: c.val,
        traverse: c.attr ? `attr:${c.attr}` : c.via ? `via:${c.via}` : c.ref ? `ref:${c.ref}` : '',
      })),
    }),
    codeOf: (f) => f.code,
    create: (f) => api.aggregates.create({ code: f.code, ...posteToBody(f) }),
    update: (code, f) => api.aggregates.update(code, posteToBody(f)),
    remove: api.aggregates.remove,
    confirmRemove: (code) => `Supprimer le poste « ${code} » ?`,
    onError,
  });

  // Ajoute une condition pour une dimension donnée (chips « + dim »).
  const addCondForDim = (dim: string) =>
    setForm((f) => ({
      ...f,
      conditions: [...f.conditions, { dim, op: '=', val: '', traverse: '' }],
    }));

  const updateCond = (i: number, patch: Partial<CondDraft>) =>
    setForm((f) => ({
      ...f,
      conditions: f.conditions.map((c, idx) => (idx === i ? { ...c, ...patch } : c)),
    }));

  return (
    <div className="editor-split">
      <LibraryList
        items={aggregates}
        getKey={(a) => a.code}
        selected={selected}
        onSelect={open}
        onNew={() => open('new')}
        newLabel="+ Nouveau poste"
        width={300}
        columns={[
          { header: 'Code', cell: (a) => a.code },
          { header: 'Niveau', cell: (a) => a.level },
        ]}
        actions={(a) => (
          <button type="button" className="btn btn--ghost" onClick={() => remove(a.code)} title="Supprimer">
            ✕
          </button>
        )}
      />

      {selected !== null && (
        <div className="editor-pane">
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

          <div className="rule-section" style={{ marginTop: 12 }}>
            <h4 className="rule-section__title">Sélection</h4>
            {form.conditions.map((c, i) => {
              const viaOptions = characteristics.filter((ch) => ch.base_dimension === c.dim);
              const refOptions = customRefs.filter((r) => r.host_dimension === c.dim);
              const attrOptions = nativeEnums.filter((e) => e.host_dimension === c.dim);
              const { via, ref, attr } = decodeTraverse(c.traverse);
              const libelle = dims.find((d) => d.name === c.dim)?.label ?? '';
              const dimTitle = formatOptionLabel(c.dim, libelle);
              return (
                <div key={i} className="rule-condition rule-condition--compact">
                  <span className="rule-sel-dim" title={dimTitle}>
                    {dimTitle}
                  </span>
                  <TraverseField
                    value={c.traverse}
                    disabled={!c.dim}
                    viaOptions={viaOptions}
                    refOptions={refOptions}
                    attrOptions={attrOptions}
                    onSelect={(v) => updateCond(i, { traverse: v, val: '' })}
                  />
                  <OpSelect value={c.op} onChange={(op) => updateCond(i, { op })} />
                  {!NULL_OPS.has(c.op) && (
                    <label className="field field--grow">
                      <span>Valeur</span>
                      <SelectionValueField
                        sel={{ dim: c.dim, op: c.op, val: c.val, via, ref, attr }}
                        customReferences={customRefs}
                        nativeEnums={nativeEnums}
                        op={c.op}
                        value={c.val}
                        onRawChange={(raw) => updateCond(i, { val: parseCondVal(c.op, raw) })}
                      />
                    </label>
                  )}
                  <button
                    type="button"
                    className="btn btn--ghost"
                    title="Retirer la condition"
                    onClick={() =>
                      setForm((f) => ({ ...f, conditions: f.conditions.filter((_, idx) => idx !== i) }))
                    }
                  >
                    ✕
                  </button>
                </div>
              );
            })}
            {(() => {
              const present = new Set(form.conditions.map((c) => c.dim));
              const addable = sortForDisplay(
                dims.filter((d) => !present.has(d.name)),
                (d) => formatOptionLabel(d.name, d.label),
              );
              return (
                <div className="rule-dest-add">
                  <span className="rule-dest-add__label">Ajouter un filtre</span>
                  {addable.map((d) => (
                    <button key={d.name} type="button" className="rule-chip" onClick={() => addCondForDim(d.name)}>
                      + {formatOptionLabel(d.name, d.label)}
                    </button>
                  ))}
                  {addable.length === 0 && <span className="muted">toutes les dimensions sont posées</span>}
                </div>
              );
            })()}
          </div>

          <div className="rule-op-summary">
            <span className="rule-op-summary__tag">résumé</span>
            <span>{summarizePoste(form.level, form.conditions)}</span>
          </div>

          <div className="editor-actions">
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

// Corps d'API d'un indicateur (hors code) dérivé du formulaire.
function indicToBody(f: IndicForm) {
  return {
    libelle: f.libelle || undefined,
    expression: f.expression,
    grain: f.grain,
    format: f.format,
  };
}

function IndicateursTab({
  dims,
  consolidations,
  onError,
}: {
  dims: DimensionInfo[];
  consolidations: ConsolidationSummary[];
  onError: (e: string | null) => void;
}) {
  const [operands, setOperands] = useState<IndicatorOperand[]>([]);
  const [consolidation, setConsolidation] = useState<number | undefined>(undefined);
  const [preview, setPreview] = useState<IndicatorPreview | null>(null);
  const exprRef = useRef<FormulaEditorHandle>(null);

  const {
    items: indicators,
    selected,
    setSelected,
    form,
    setForm,
    saving,
    open: openResource,
    save,
    remove,
  } = useCrudResource<Indicator, IndicForm>({
    list: api.indicators.list,
    keyOf: (ind) => ind.code,
    emptyForm: EMPTY_INDIC,
    toForm: (ind) => ({
      code: ind.code,
      libelle: ind.libelle ?? '',
      expression: ind.expression,
      grain: ind.grain ?? [],
      format: ind.format ?? 'nombre',
    }),
    codeOf: (f) => f.code,
    create: (f) => api.indicators.create({ code: f.code, ...indicToBody(f) }),
    update: (code, f) => api.indicators.update(code, indicToBody(f)),
    remove: api.indicators.remove,
    confirmRemove: (code) => `Supprimer l'indicateur « ${code} » ?`,
    onError,
  });

  // Ouvrir (ré)initialise la preview, comme avant la factorisation.
  const open = useCallback(
    (ind: Indicator | 'new') => {
      openResource(ind);
      setPreview(null);
    },
    [openResource],
  );

  // Opérandes statiques vis-à-vis du CRUD : un chargement au montage.
  useEffect(() => {
    void (async () => {
      try {
        setOperands(await api.indicators.operands());
      } catch (e) {
        onError(errMsg(e));
      }
    })();
  }, [onError]);

  useEffect(() => {
    if (consolidation === undefined && consolidations.length) setConsolidation(consolidations[0].id);
  }, [consolidations, consolidation]);

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
        setPreview({ ok: false, error: errMsg(e), rows: [] });
      }
    }, 350);
    return () => clearTimeout(handle);
  }, [form.expression, form.grain, consolidation]);

  const insert = useCallback((fragment: string) => {
    exprRef.current?.insert(fragment);
  }, []);

  const toggleGrain = (name: string) =>
    setForm((f) => ({
      ...f,
      grain: f.grain.includes(name) ? f.grain.filter((g) => g !== name) : [...f.grain, name],
    }));

  return (
    <div className="editor-split">
      <LibraryList
        items={indicators}
        getKey={(ind) => ind.code}
        selected={selected}
        onSelect={open}
        onNew={() => open('new')}
        newLabel="+ Nouvel indicateur"
        width={280}
        columns={[{ header: 'Code', cell: (ind) => ind.code, title: (ind) => ind.libelle ?? '' }]}
        actions={(ind) => (
          <button type="button" className="btn btn--ghost" onClick={() => remove(ind.code)} title="Supprimer">
            ✕
          </button>
        )}
      />

      {selected !== null && (
        <div className="editor-pane">
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
            {FORMULA_FUNCTIONS.map((fn) => (
              <button key={fn} type="button" className="chip chip--fn" onClick={() => insert(`${fn}()`)}>
                {fn}
              </button>
            ))}
          </div>

          <label className="field">
            <span>Formule</span>
            <FormulaEditor
              ref={exprRef}
              value={form.expression}
              onChange={(v) => setForm((f) => ({ ...f, expression: v }))}
              operands={operands}
              rows={3}
              placeholder="SAFE_DIV([resultat]; [ca])"
            />
          </label>

          {/* Grain */}
          <div className="grain-block">
            <div className="muted grain-block__label">Grain (dimensions de restitution) :</div>
            <div className="grain-chips">
              {dims.map((d) => {
                const on = form.grain.includes(d.name);
                return (
                  <button
                    key={d.name}
                    type="button"
                    className={`grain-chip${on ? ' is-on' : ''}`}
                    title={d.label}
                    onClick={() => toggleGrain(d.name)}
                  >
                    {d.name}
                  </button>
                );
              })}
            </div>
            {form.grain.length === 0 && (
              <div className="muted grain-block__empty">Aucun grain → un total unique.</div>
            )}
          </div>

          <div className="rule-op-summary">
            <span className="rule-op-summary__tag">résumé</span>
            <span>
              {form.libelle || form.code || 'indicateur'} — format « {form.format} », par{' '}
              {form.grain.length > 0 ? form.grain.join(' × ') : 'total unique'}.
            </span>
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

          <div className="editor-actions">
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
          <OperandPalette
            title="Postes & indicateurs disponibles"
            hint="Cliquez pour insérer dans la formule. Créez les postes dans l’onglet « Postes »."
            operands={operands}
            onPick={(token) => insert(`[${token}]`)}
          />
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
      return formatFormulaValue(v);
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
