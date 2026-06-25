// Page « Coefficients » — bibliothèque de formules nommées (volet 1 du moteur de
// formules, cf. docs/FORMULES.md). À gauche : la bibliothèque (natifs verrouillés
// + utilisateur). À droite : l'éditeur ergonomique — panneau d'opérandes
// insérables, barre de formule, et preview live (valeur évaluée + SQL compilé).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { FormulaEditor, type FormulaEditorHandle } from '../components/FormulaEditor';
import { OperandPalette } from '../components/OperandPalette';
import type { Coefficient, CoefficientOperand, CoefficientPreview } from '../types';

const FUNCTIONS = ['MIN', 'MAX', 'SAFE_DIV', 'IF', 'ABS', 'ROUND'];

interface FormState {
  code: string;
  libelle: string;
  expression: string;
}

const EMPTY_FORM: FormState = { code: '', libelle: '', expression: '' };

export function CoefficientsPage() {
  const [coefficients, setCoefficients] = useState<Coefficient[]>([]);
  const [operands, setOperands] = useState<CoefficientOperand[]>([]);
  // null = rien d'ouvert ; 'new' = création ; sinon le code édité.
  const [selected, setSelected] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [samples, setSamples] = useState<Record<string, number>>({});
  const [preview, setPreview] = useState<CoefficientPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const exprRef = useRef<FormulaEditorHandle>(null);

  const isBuiltin = useMemo(() => {
    if (selected === 'new' || selected === null) return false;
    return coefficients.find((c) => c.code === selected)?.kind === 'builtin';
  }, [selected, coefficients]);
  const readOnly = isBuiltin;

  const reload = useCallback(async () => {
    try {
      const [list, ops] = await Promise.all([
        api.coefficients.list(),
        api.coefficients.operands(),
      ]);
      setCoefficients(list);
      setOperands(ops);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Ouvre un coefficient en édition (ou la création).
  const open = useCallback((c: Coefficient | 'new') => {
    setError(null);
    if (c === 'new') {
      setSelected('new');
      setForm(EMPTY_FORM);
    } else {
      setSelected(c.code);
      setForm({ code: c.code, libelle: c.libelle ?? '', expression: c.expression });
    }
  }, []);

  // Preview live (débouncée) : valide + évalue la formule contre des valeurs
  // d'exemple. Les opérandes référencés non saisis prennent 1 (résultat parlant).
  useEffect(() => {
    if (!form.expression.trim()) {
      setPreview(null);
      return;
    }
    const handle = setTimeout(async () => {
      try {
        const res = await api.coefficients.preview({
          expression: form.expression,
          samples,
        });
        setPreview(res);
      } catch (e) {
        setPreview({
          ok: false,
          error: e instanceof Error ? e.message : String(e),
          operands: [],
        });
      }
    }, 300);
    return () => clearTimeout(handle);
  }, [form.expression, samples]);

  // Quand les opérandes référencés changent, initialise les valeurs d'exemple
  // manquantes à 1 (pour une preview non triviale).
  useEffect(() => {
    if (!preview?.operands?.length) return;
    setSamples((prev) => {
      let changed = false;
      const next = { ...prev };
      for (const op of preview.operands) {
        if (!(op in next)) {
          next[op] = 1;
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [preview?.operands]);

  // Insère un fragment à la position du curseur dans la barre de formule.
  const insert = useCallback((fragment: string) => {
    exprRef.current?.insert(fragment);
  }, []);

  const save = useCallback(async () => {
    setError(null);
    setSaving(true);
    try {
      const body = { libelle: form.libelle || undefined, expression: form.expression };
      if (selected === 'new') {
        await api.coefficients.create({ code: form.code, ...body });
      } else if (selected) {
        await api.coefficients.update(selected, body);
      }
      await reload();
      setSelected(form.code);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }, [form, selected, reload]);

  const remove = useCallback(
    async (code: string) => {
      if (!confirm(`Supprimer le coefficient « ${code} » ?`)) return;
      setError(null);
      try {
        await api.coefficients.remove(code);
        await reload();
        if (selected === code) setSelected(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [reload, selected],
  );

  const duplicate = useCallback((c: Coefficient) => {
    setError(null);
    setSelected('new');
    setForm({ code: `${c.code}_COPIE`, libelle: c.libelle ?? '', expression: c.expression });
  }, []);

  return (
    <div className="page">
      <div className="page__header">
        <h2>Coefficients</h2>
        <p className="page__hint">
          Formules type Excel évaluées au grain d'une écriture de règle. Opérandes
          de périmètre aux 4 perspectives ; fonctions <code>MIN MAX SAFE_DIV IF ABS ROUND</code>.
        </p>
      </div>

      {error && <div className="banner banner--error">{error}</div>}

      <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start' }}>
        {/* ── Bibliothèque ── */}
        <div style={{ flex: '0 0 320px' }}>
          <button type="button" className="btn btn--primary" onClick={() => open('new')}>
            + Nouveau coefficient
          </button>
          <table className="table" style={{ marginTop: 12 }}>
            <thead>
              <tr>
                <th>Code</th>
                <th>Type</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {coefficients.map((c) => (
                <tr
                  key={c.code}
                  className={selected === c.code ? 'row--selected' : ''}
                  style={{ cursor: 'pointer' }}
                >
                  <td onClick={() => open(c)} title={c.libelle ?? ''}>
                    {c.code}
                  </td>
                  <td onClick={() => open(c)}>
                    <span className={`rule-badge ${c.kind === 'builtin' ? '' : 'rule-badge--user'}`}>
                      {c.kind === 'builtin' ? 'natif' : 'utilisateur'}
                    </span>
                  </td>
                  <td>
                    <button type="button" className="btn btn--ghost" onClick={() => duplicate(c)} title="Dupliquer">
                      ⧉
                    </button>
                    {c.kind === 'user' && (
                      <button type="button" className="btn btn--ghost" onClick={() => remove(c.code)} title="Supprimer">
                        ✕
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* ── Éditeur ── */}
        {selected !== null && (
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: 'flex', gap: 12 }}>
              <label className="field" style={{ flex: '0 0 200px' }}>
                <span>Code</span>
                <input
                  type="text"
                  value={form.code}
                  disabled={selected !== 'new'}
                  onChange={(e) => setForm((f) => ({ ...f, code: e.target.value }))}
                  placeholder="ex. minoritaire"
                />
              </label>
              <label className="field" style={{ flex: 1 }}>
                <span>Libellé</span>
                <input
                  type="text"
                  value={form.libelle}
                  disabled={readOnly}
                  onChange={(e) => setForm((f) => ({ ...f, libelle: e.target.value }))}
                  placeholder="ex. Quote-part minoritaire"
                />
              </label>
            </div>

            {/* Fonctions insérables */}
            <div style={{ margin: '12px 0 6px', display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {FUNCTIONS.map((fn) => (
                <button
                  key={fn}
                  type="button"
                  className="chip chip--fn"
                  disabled={readOnly}
                  onClick={() => insert(`${fn}()`)}
                  title="Insérer la fonction"
                >
                  {fn}
                </button>
              ))}
            </div>

            {/* Barre de formule */}
            <label className="field">
              <span>Formule</span>
              <FormulaEditor
                ref={exprRef}
                value={form.expression}
                onChange={(v) => setForm((f) => ({ ...f, expression: v }))}
                operands={operands}
                readOnly={readOnly}
                rows={4}
                placeholder="MIN(1; SAFE_DIV([pct_integration.partner]; [pct_integration.entity]))"
              />
            </label>

            {/* Preview live */}
            <div
              className={`preview ${preview && !preview.ok ? 'preview--err' : 'preview--ok'}`}
            >
              {!preview && <span className="muted">Saisissez une formule pour la prévisualiser.</span>}
              {preview && preview.ok && (
                <div>
                  <strong>Résultat&nbsp;: {formatValue(preview.value)}</strong>
                  {preview.sql && <pre>{preview.sql}</pre>}
                </div>
              )}
              {preview && !preview.ok && (
                <span className="preview__error">⚠ {preview.error}</span>
              )}
            </div>

            {/* Valeurs d'exemple de la preview */}
            {preview?.operands?.length ? (
              <div className="coeff-samples">
                <div className="muted coeff-samples__label">Valeurs d'exemple (preview) :</div>
                <div className="coeff-samples__grid">
                  {preview.operands.map((op) => (
                    <label key={op} className="field field--inline">
                      <span className="coeff-samples__token">{op}</span>
                      <input
                        type="number"
                        step="any"
                        className="coeff-samples__input"
                        value={samples[op] ?? 1}
                        onChange={(e) =>
                          setSamples((s) => ({ ...s, [op]: Number(e.target.value) }))
                        }
                      />
                    </label>
                  ))}
                </div>
              </div>
            ) : null}

            {/* Actions */}
            {!readOnly && (
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
            )}
            {readOnly && (
              <div className="muted" style={{ marginTop: 12 }}>
                Coefficient natif — non modifiable. Utilisez « Dupliquer » pour en
                créer une variante éditable.
              </div>
            )}

            {/* Panneau d'opérandes insérables */}
            <OperandPalette
              title="Opérandes disponibles (périmètre)"
              hint="Cliquez pour insérer dans la formule. Taux absent → 0 (vigilance à votre charge ; protégez les divisions avec SAFE_DIV)."
              operands={operands}
              disabled={readOnly}
              onPick={(token) => insert(`[${token}]`)}
            />
          </div>
        )}
      </div>
    </div>
  );
}

function formatValue(v: number | undefined): string {
  if (v === undefined) return '—';
  // Affiche jusqu'à 6 décimales sans zéros superflus.
  return Number(v.toFixed(6)).toString();
}
