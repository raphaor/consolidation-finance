// Page « Coefficients » — bibliothèque de formules nommées (volet 1 du moteur de
// formules, cf. docs/FORMULES.md). À gauche : la bibliothèque (natifs verrouillés
// + utilisateur). À droite : l'éditeur ergonomique — panneau d'opérandes
// insérables, barre de formule, et preview live (valeur évaluée + SQL compilé).

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api';
import { FormulaEditor, type FormulaEditorHandle } from '../components/FormulaEditor';
import { LibraryList } from '../components/LibraryList';
import { OperandPalette } from '../components/OperandPalette';
import { PageHeader } from '../components/PageHeader';
import { useCrudResource } from '../hooks/useCrudResource';
import { FORMULA_FUNCTIONS, formatFormulaValue } from '../utils/format';
import { errMsg } from '../utils/errMessage';
import type { Coefficient, CoefficientOperand, CoefficientPreview } from '../types';

interface FormState {
  code: string;
  libelle: string;
  expression: string;
}

const EMPTY_FORM: FormState = { code: '', libelle: '', expression: '' };

export function CoefficientsPage() {
  const [operands, setOperands] = useState<CoefficientOperand[]>([]);
  const [samples, setSamples] = useState<Record<string, number>>({});
  const [preview, setPreview] = useState<CoefficientPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const exprRef = useRef<FormulaEditorHandle>(null);

  const {
    items: coefficients,
    selected,
    setSelected,
    form,
    setForm,
    saving,
    open,
    startDraft,
    save,
    remove,
  } = useCrudResource<Coefficient, FormState>({
    list: api.coefficients.list,
    keyOf: (c) => c.code,
    emptyForm: EMPTY_FORM,
    toForm: (c) => ({ code: c.code, libelle: c.libelle ?? '', expression: c.expression }),
    codeOf: (f) => f.code,
    create: (f) =>
      api.coefficients.create({
        code: f.code,
        libelle: f.libelle || undefined,
        expression: f.expression,
      }),
    update: (code, f) =>
      api.coefficients.update(code, { libelle: f.libelle || undefined, expression: f.expression }),
    remove: api.coefficients.remove,
    confirmRemove: (code) => `Supprimer le coefficient « ${code} » ?`,
    onError: setError,
  });

  const isBuiltin = useMemo(() => {
    if (selected === 'new' || selected === null) return false;
    return coefficients.find((c) => c.code === selected)?.kind === 'builtin';
  }, [selected, coefficients]);
  const readOnly = isBuiltin;

  // Les opérandes sont statiques vis-à-vis du CRUD des coefficients : un seul
  // chargement au montage suffit.
  useEffect(() => {
    void (async () => {
      try {
        setOperands(await api.coefficients.operands());
      } catch (e) {
        setError(errMsg(e));
      }
    })();
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
          error: errMsg(e),
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

  const duplicate = useCallback(
    (c: Coefficient) =>
      startDraft({ code: `${c.code}_COPIE`, libelle: c.libelle ?? '', expression: c.expression }),
    [startDraft],
  );

  return (
    <div className="page">
      <PageHeader
        title="Coefficients"
        hint={
          <>
            Formules type Excel évaluées au grain d'une écriture de règle. Opérandes
            de périmètre aux 4 perspectives ; fonctions <code>MIN MAX SAFE_DIV IF ABS ROUND</code>.
          </>
        }
      />

      {error && <div className="banner banner--error">{error}</div>}

      <div className="editor-split">
        {/* ── Bibliothèque ── */}
        <LibraryList
          items={coefficients}
          getKey={(c) => c.code}
          selected={selected}
          onSelect={open}
          onNew={() => open('new')}
          newLabel="+ Nouveau coefficient"
          width={320}
          columns={[
            { header: 'Code', cell: (c) => c.code, title: (c) => c.libelle ?? '' },
            {
              header: 'Type',
              cell: (c) => (
                <span className={`rule-badge ${c.kind === 'builtin' ? '' : 'rule-badge--user'}`}>
                  {c.kind === 'builtin' ? 'natif' : 'utilisateur'}
                </span>
              ),
            },
          ]}
          actions={(c) => (
            <>
              <button type="button" className="btn btn--ghost" onClick={() => duplicate(c)} title="Dupliquer">
                ⧉
              </button>
              {c.kind === 'user' && (
                <button type="button" className="btn btn--ghost" onClick={() => remove(c.code)} title="Supprimer">
                  ✕
                </button>
              )}
            </>
          )}
        />

        {/* ── Éditeur ── */}
        {selected !== null && (
          <div className="editor-pane">
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
              {FORMULA_FUNCTIONS.map((fn) => (
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
                  <strong>Résultat&nbsp;: {formatFormulaValue(preview.value)}</strong>
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
