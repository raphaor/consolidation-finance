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
import { compareText, sortForDisplay } from '../utils/format';
import {
  DIM_TO_TABLE_FALLBACK,
  DimRefContext,
  buildDimToTable,
  type DimToTable,
} from '../hooks/useDimValues';
import {
  OpSelect,
  OverrideValueField,
  SelectionValueField,
  TraverseField,
  ValueField,
} from '../components/ConditionFields';
import { NULL_OPS, OP_SYMBOL } from '../components/operators';
import { parseCondVal } from '../utils/conditionValue';
import { RenameModal } from '../components/RenameModal';
import type {
  Characteristic,
  CustomReference,
  DimensionInfo,
  NativeEnum,
  Operation,
  RuleDefinition,
  RuleSummary,
  RulesetDetail,
  RulesetItem,
  RulesetSummary,
  ScopeCond,
  SelectionCond,
} from '../types';

// Constantes qui ne dépendent pas du registre des dimensions.
const SCOPE_DIMS = ['methode', 'pct_interet', 'pct_integration', 'entree', 'sortie'];
const LEVELS_LIST = ['corporate', 'converted', 'consolidated'];
const COEFF_TYPES = [
  'pct_integration',
  'pct_interet',
  // Élimination IC corporate : Min(1, INTEG_PA / INTEG_EN) en N / N-1 et leur
  // variation. N-1 lu via le périmètre du scénario d'à-nouveau (cf. REGLES_CONSO §4.2).
  'elim_ic_corp_n',
  'elim_ic_corp_n1',
  'elim_ic_corp_var',
  'constant',
];

// Dimensions pilotables affichées d'emblée en destination (les plus fréquentes).
// Les autres dimensions pilotables sont proposées à l'ajout via des chips « + ».
// (À l'ouverture d'une règle existante, toute dimension déjà pilotée — mode ≠
// inherit — est aussi révélée automatiquement, cf. `visibleDestDims`.)
const DEFAULT_DEST_DIMS = ['account', 'flow', 'nature'];

// Dimensions de filtre affichées d'emblée en sélection (mêmes que destination).
// Les autres sont ajoutables via des chips « + ». Les lignes seedées laissées
// vides sont filtrées à l'enregistrement (cf. `RuleFormModal.submit`).
const DEFAULT_SEL_DIMS = ['account', 'flow', 'nature'];

/// Une condition de sélection est-elle « porteuse » (à envoyer au moteur) ? Les
/// ops unaires (IS NULL / IS NOT NULL) le sont toujours ; sinon il faut une
/// valeur non vide. Sert à filtrer les lignes de base seedées restées vides.
function selectionIsMeaningful(s: SelectionCond): boolean {
  if (NULL_OPS.has(s.op)) return true;
  if (Array.isArray(s.val)) return s.val.length > 0;
  return String(s.val ?? '').trim() !== '';
}

/// Seed les dimensions de base manquantes (account/flow/nature) en tête de la
/// sélection de chaque opération, sans dupliquer celles déjà présentes ni
/// toucher aux conditions existantes. Appelé à l'ouverture de la modale.
function seedBaseSelection(
  def: RuleDefinition,
  selectionDims: string[],
): RuleDefinition {
  const base = DEFAULT_SEL_DIMS.filter((d) => selectionDims.includes(d));
  return {
    ...def,
    operations: def.operations.map((op) => {
      const present = new Set(op.selection.map((s) => s.dim));
      const seeded: SelectionCond[] = base
        .filter((d) => !present.has(d))
        .map((d) => ({ dim: d, op: '=', val: '' }));
      return { ...op, selection: [...seeded, ...op.selection] };
    }),
  };
}

// Fallback des 12 dimensions built-in si l'API /api/meta/dimensions est
// injoignable (serveur obsolète, réseau en panne). Miroir de
// `engine/src/dimensions.rs::builtin_dims()`. Les dimensions custom (ajoutées
// par l'utilisateur) ne seront pas présentes, mais l'éditeur reste utilisable.
const BUILTIN_DIMS_FALLBACK: DimensionInfo[] = [
  { name: 'phase',       category: 'Fixed',      custom: false, label: 'Phase',      pilotable: false },
  { name: 'entity',       category: 'Active',     custom: false, label: 'Entité',     pilotable: true  },
  { name: 'entry_period', category: 'Fixed',      custom: false, label: 'Exercice',   pilotable: false },
  { name: 'period',       category: 'Fixed',      custom: false, label: 'Période',    pilotable: false },
  { name: 'account',      category: 'Active',     custom: false, label: 'Compte',     pilotable: true  },
  { name: 'flow',         category: 'Active',     custom: false, label: 'Flux',       pilotable: true  },
  { name: 'currency',     category: 'Fixed',      custom: false, label: 'Devise',     pilotable: false },
  { name: 'nature',       category: 'Active',     custom: false, label: 'Nature',     pilotable: true  },
  { name: 'partner',      category: 'Analytical', custom: false, label: 'Partenaire', pilotable: true  },
  { name: 'share',        category: 'Analytical', custom: false, label: 'Titre',       pilotable: true  },
  { name: 'analysis',     category: 'Analytical', custom: false, label: 'Analyse 1',  pilotable: true  },
  { name: 'analysis2',    category: 'Analytical', custom: false, label: 'Analyse 2',  pilotable: true  },
];

type Notice = { kind: 'success' | 'error'; text: string } | null;
type DestMode = 'inherit' | 'override' | 'null' | 'map' | 'map_ref';

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
  // Affichage alphabétique des dimensions dans les dropdowns / lignes de destination.
  return {
    selectionDims: sortForDisplay(selectionDims, (d) => d),
    pilotableDims: sortForDisplay(pilotableDims, (d) => d),
  };
}

/// Parse un multiplicateur : accepte la virgule ou le point comme séparateur
/// décimal (la locale fr utilise la virgule, mais JS Number() exige le point).
/// Retourne NaN si la valeur n'est pas un nombre valide.
function parseMultiplicateur(raw: string): number {
  const cleaned = raw.trim().replace(',', '.');
  if (cleaned === '' || cleaned === '-') return NaN;
  return Number(cleaned);
}

/// Phrase de relecture d'une opération, générée depuis le JSON (aide à valider
/// sans déchiffrer chaque champ). Volontairement compacte ; pas de prétention
/// SQL, juste un résumé lisible.
function summarizeOperation(op: Operation): string {
  const fmtVal = (o: string, val: unknown): string => {
    if (NULL_OPS.has(o)) return '';
    if (Array.isArray(val)) return `{${val.join(', ')}}`;
    const s = String(val ?? '');
    return s === '' ? '∅' : s;
  };
  const conds = op.selection.filter(selectionIsMeaningful);
  const sel =
    conds.length === 0
      ? 'toutes les écritures'
      : 'les écritures où ' +
        conds
          .map((s) => {
            const sym = OP_SYMBOL[s.op] ?? s.op;
            const trav = s.via
              ? `.${s.via}`
              : s.ref
                ? `.${s.ref}`
                : s.attr
                  ? `.${s.attr}`
                  : '';
            const v = fmtVal(s.op, s.val);
            return `${s.dim}${trav} ${sym}${v ? ` ${v}` : ''}`.trim();
          })
          .join(' et ');
  const coeffLabel =
    op.coefficient.type === 'constant'
      ? String(op.coefficient.value ?? 0)
      : op.coefficient.type;
  const mult = Number.isNaN(op.multiplicateur) ? '?' : op.multiplicateur;
  const overrides = Object.entries(op.destination)
    .filter(([, d]) => d.mode !== 'inherit')
    .map(([dim, d]) => {
      if (d.mode === 'override') return `${dim} → ${d.value ?? '?'}`;
      if (d.mode === 'null') return `${dim} → ∅`;
      if (d.mode === 'map') return `${dim} ← ${d.via ?? '?'}.${d.attr ?? '?'}`;
      if (d.mode === 'map_ref') return `${dim} ← ${d.ref ?? '?'}`;
      return dim;
    });
  const destPart = overrides.length
    ? `écrit ${overrides.join(', ')}`
    : 'destination héritée';
  return `Sur ${op.level}, prend ${sel}, applique ${coeffLabel} × ${mult}, ${destPart}.`;
}

function emptyScopeCond(): ScopeCond {
  return { target: 'entity', dim: SCOPE_DIMS[0], op: '=', val: '' };
}

function emptyOperation(seq: number, pilotableDims: string[]): Operation {
  const destination: Operation['destination'] = {};
  for (const dim of pilotableDims) {
    destination[dim] = { mode: 'inherit' };
  }
  return {
    seq,
    level: LEVELS_LIST[0],
    // Dimensions de filtre de base affichées d'emblée (vides → ignorées au save).
    selection: DEFAULT_SEL_DIMS.map((d) => ({ dim: d, op: '=', val: '' })),
    coefficient: { type: COEFF_TYPES[0] },
    multiplicateur: 1,
    destination,
  };
}

/// Clone une opération existante pour pré-remplir la suivante (« Ajouter une
/// opération » recopie la dernière). Copie en profondeur les structures
/// imbriquées (`selection`, `coefficient`, `destination`) pour qu'éditer la
/// nouvelle opération n'altère pas celle d'origine. Seul le `seq` est neuf.
function cloneOperation(op: Operation, seq: number): Operation {
  return {
    ...op,
    seq,
    selection: op.selection.map((s) => ({ ...s })),
    coefficient: { ...op.coefficient },
    destination: Object.fromEntries(
      Object.entries(op.destination).map(([dim, dest]) => [dim, { ...dest }]),
    ),
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
  const [draft, setDraft] = useState<RuleDraft>(() => ({
    ...initial,
    definition: seedBaseSelection(initial.definition, selectionDims),
  }));
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  // Dimensions de destination révélées manuellement par opération (clé = seq).
  // Les défauts (account/flow/nature) et toute dimension déjà pilotée sont
  // toujours visibles ; ceci ne couvre que les chips « + » cliqués.
  const [revealedDest, setRevealedDest] = useState<Record<number, string[]>>({});
  // Caractéristiques N1/N2 disponibles pour les destinations `map` et les
  // sélections `via`.
  const [characteristics, setCharacteristics] = useState<Characteristic[]>([]);
  // Références directes (patron B) disponibles pour les destinations `map_ref`
  // et les sélections `ref`. Inclut les FK natives auto-peuplées (`native=true`).
  const [customReferences, setCustomReferences] = useState<CustomReference[]>([]);
  // Enums natifs (CHECK du DDL) disponibles pour les sélections `attr`.
  const [nativeEnums, setNativeEnums] = useState<NativeEnum[]>([]);
  // Codes de coefficient de la bibliothèque (moteur de formules, volet 1) —
  // natifs + utilisateur ; `constant` (littéral inline) est ajouté à la fin.
  // Repli sur COEFF_TYPES si l'API est injoignable (serveur obsolète).
  const [coeffOptions, setCoeffOptions] = useState<{ code: string; label: string }[]>(
    COEFF_TYPES.map((c) => ({ code: c, label: c })),
  );
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [cs, refs, enums, coeffs] = await Promise.all([
          api.characteristics.list(),
          api.customReferences.list(),
          api.nativeEnums(),
          api.coefficients.list(),
        ]);
        if (!cancelled) {
          setCharacteristics(cs);
          setCustomReferences(refs);
          setNativeEnums(enums);
          setCoeffOptions([
            ...sortForDisplay(
              coeffs.map((c) => ({
                code: c.code,
                label: c.libelle ? `${c.code} — ${c.libelle}` : c.code,
              })),
              (c) => c.label,
            ),
            { code: 'constant', label: 'constant (littéral)' },
          ]);
        }
      } catch {
        if (!cancelled) {
          setCharacteristics([]);
          setCustomReferences([]);
          setNativeEnums([]);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  async function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setSubmitting(true);
    setFormError(null);

    // Validation locale avant envoi : tous les multiplicateurs doivent être des
    // nombres valides (la locale fr peut produire NaN via Number("1,5").
    const badMult = draft.definition.operations.filter((o) => isNaN(o.multiplicateur));
    if (badMult.length > 0) {
      const seqs = badMult.map((o) => `op ${o.seq}`).join(', ');
      setFormError(
        `Multiplicateur invalide (NaN) sur : ${seqs}. ` +
          `Utiliser le point ou la virgule comme séparateur décimal (ex: 0,5 ou -1).`,
      );
      setSubmitting(false);
      return;
    }

    // Nettoyage avant envoi : on retire les conditions de sélection vides (lignes
    // de base seedées non renseignées) pour ne pas envoyer de filtre `dim = ''`
    // au moteur.
    const cleaned: RuleDraft = {
      ...draft,
      definition: {
        ...draft.definition,
        operations: draft.definition.operations.map((o) => ({
          ...o,
          selection: o.selection.filter(selectionIsMeaningful),
        })),
      },
    };

    try {
      await onSubmit(cleaned);
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
    setDraft((d) => {
      const ops = d.definition.operations;
      const seq = ops.reduce((m, o) => Math.max(m, o.seq), 0) + 1;
      // Recopie la dernière opération si elle existe (pré-remplissage), sinon
      // crée une opération vide.
      const last = ops[ops.length - 1];
      const newOp = last
        ? cloneOperation(last, seq)
        : emptyOperation(seq, pilotableDims);
      return {
        ...d,
        definition: { ...d.definition, operations: [...ops, newOp] },
      };
    });
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
  // Ajoute une condition de filtre pour une dimension donnée (chips « + »).
  function addSelectionForDim(opIdx: number, dim: string) {
    setDraft((d) => {
      const operations = d.definition.operations.map((o, i) =>
        i === opIdx
          ? { ...o, selection: [...o.selection, { dim, op: '=', val: '' }] }
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
    patch: Partial<{ mode: DestMode; value: string; via: string; attr: string; ref: string }>,
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

  // Dimensions de destination à afficher pour une opération : défauts
  // (account/flow/nature) + toute dimension déjà pilotée (mode ≠ inherit) +
  // chips révélés manuellement. Défauts en tête, le reste trié.
  function visibleDestDims(op: Operation): string[] {
    const defaults = DEFAULT_DEST_DIMS.filter((d) => pilotableDims.includes(d));
    const active = pilotableDims.filter(
      (d) => (op.destination[d]?.mode ?? 'inherit') !== 'inherit',
    );
    const revealed = (revealedDest[op.seq] ?? []).filter((d) =>
      pilotableDims.includes(d),
    );
    const shown = new Set<string>([...defaults, ...active, ...revealed]);
    const rest = sortForDisplay(
      [...shown].filter((d) => !defaults.includes(d)),
      (d) => d,
    );
    return [...defaults, ...rest];
  }

  // Révèle une dimension de destination (chip « + ») sans la piloter encore
  // (reste inherit jusqu'à modification du mode).
  function revealDest(seq: number, dim: string) {
    setRevealedDest((r) => ({ ...r, [seq]: [...(r[seq] ?? []), dim] }));
  }

  // Masque une dimension non-défaut : remet son mode à inherit et la retire des
  // dimensions révélées.
  function hideDest(opIdx: number, seq: number, dim: string) {
    updateDestination(opIdx, dim, { mode: 'inherit' });
    setRevealedDest((r) => ({
      ...r,
      [seq]: (r[seq] ?? []).filter((d) => d !== dim),
    }));
  }

  // ---------- Level global (hérité par toutes les opérations) ----------
  // Le level est porté par chaque opération dans le modèle JSON (rétrocompatible
  // moteur), mais l'UI l'expose comme un attribut global de la règle : toutes
  // les opérations partagent le même niveau. La modifier propage à toutes.
  function updateGlobalLevel(level: string) {
    setDraft((d) => ({
      ...d,
      definition: {
        ...d.definition,
        operations: d.definition.operations.map((o) => ({ ...o, level })),
      },
    }));
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
            <label className="field">
              <span>Niveau d'exécution</span>
              <select
                value={
                  draft.definition.operations[0]?.level ?? LEVELS_LIST[0]
                }
                onChange={(e) => updateGlobalLevel(e.target.value)}
              >
                {LEVELS_LIST.map((l) => (
                  <option key={l} value={l}>
                    {l}
                  </option>
                ))}
              </select>
            </label>
          </div>

          {/* ---------- Scope ---------- */}
          <div className="rule-section">
            <h3 className="rule-section__title">Périmètre (scope)</h3>
            <p className="rule-section__hint">
              Toutes les conditions ci-dessous sont combinées par <strong>ET</strong>.
              Pour exprimer un <strong>OU</strong> sur une même dimension, utilisez l'opérateur{' '}
              <code>IN</code> et cochez plusieurs valeurs (ex :{' '}
              <code>methode IN [globale, proportionnelle]</code>). Pour les dimensions
              sans liste de référence, saisissez les valeurs séparées par des virgules.
            </p>
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
                    <option value="share">share</option>
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
                <OpSelect
                  value={c.op}
                  onChange={(op) => updateScope(idx, { op })}
                />
                {!NULL_OPS.has(c.op) && (
                  <label className="field">
                    <span>Valeur</span>
                    <ValueField
                      dim={c.dim}
                      op={c.op}
                      value={c.val}
                      onRawChange={(raw) =>
                        updateScope(idx, { val: parseCondVal(c.op, raw) })
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
                    <span>Niveau</span>
                    <span className="rule-badge" title="Niveau hérité de la règle (modifiable en haut du formulaire)">
                      {op.level}
                    </span>
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
                      {coeffOptions.map((c) => (
                        <option key={c.code} value={c.code}>
                          {c.label}
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
                      type="text"
                      inputMode="decimal"
                      value={isNaN(op.multiplicateur) ? '' : op.multiplicateur}
                      className={isNaN(op.multiplicateur) ? 'field--invalid' : ''}
                      placeholder="1, -1, 0.5…"
                      onChange={(e) =>
                        updateOp(opIdx, {
                          multiplicateur: parseMultiplicateur(e.target.value),
                        })
                      }
                    />
                    {isNaN(op.multiplicateur) && (
                      <span className="field-hint field-hint--error">
                        Nombre invalide (utiliser . ou , comme séparateur décimal)
                      </span>
                    )}
                  </label>
                  <button
                    type="button"
                    className="btn btn--sm btn--danger"
                    onClick={() => removeOp(opIdx)}
                  >
                    Supprimer
                  </button>
                </div>

                <div className="rule-op-grid">
                {/* Sélection */}
                <div className="rule-section" style={{ marginTop: 0 }}>
                  <h4 className="rule-section__title">Sélection</h4>
                  {(() => {
                    const sortedSelection = [...op.selection].sort((a, b) => {
                      const aIdx = DEFAULT_SEL_DIMS.indexOf(a.dim);
                      const bIdx = DEFAULT_SEL_DIMS.indexOf(b.dim);
                      if (aIdx !== -1 && bIdx !== -1) return aIdx - bIdx;
                      if (aIdx !== -1) return -1;
                      if (bIdx !== -1) return 1;
                      return compareText(a.dim, b.dim);
                    });
                    return sortedSelection.map((s) => {
                      const sIdx = op.selection.indexOf(s);
                      const viaOptions = sortForDisplay(
                        characteristics.filter((c) => c.base_dimension === s.dim),
                        (c) => c.code,
                      );
                      const refOptions = sortForDisplay(
                        customReferences.filter((r) => r.host_dimension === s.dim),
                        (r) => r.column,
                      );
                      const attrOptions = sortForDisplay(
                        nativeEnums.filter((e) => e.host_dimension === s.dim),
                        (e) => e.column,
                      );
                      const traverseVal = s.via
                        ? `via:${s.via}`
                        : s.ref
                          ? `ref:${s.ref}`
                          : s.attr
                            ? `attr:${s.attr}`
                            : '';
                      return (
                        <div key={sIdx} className="rule-condition rule-condition--compact">
                          <span className="rule-sel-dim">{s.dim}</span>
                          <TraverseField
                            value={traverseVal}
                            disabled={s.dim === 'level'}
                            viaOptions={viaOptions}
                            refOptions={refOptions}
                            attrOptions={attrOptions}
                            onSelect={(v) => {
                              if (v === '') {
                                updateSelection(opIdx, sIdx, {
                                  via: undefined,
                                  ref: undefined,
                                  attr: undefined,
                                  val: '',
                                });
                              } else if (v.startsWith('via:')) {
                                updateSelection(opIdx, sIdx, {
                                  via: v.slice(4),
                                  ref: undefined,
                                  attr: undefined,
                                  val: '',
                                });
                              } else if (v.startsWith('ref:')) {
                                updateSelection(opIdx, sIdx, {
                                  via: undefined,
                                  ref: v.slice(4),
                                  attr: undefined,
                                  val: '',
                                });
                              } else if (v.startsWith('attr:')) {
                                updateSelection(opIdx, sIdx, {
                                  via: undefined,
                                  ref: undefined,
                                  attr: v.slice(5),
                                  val: '',
                                });
                              }
                            }}
                          />
                          <OpSelect
                            value={s.op}
                            onChange={(op) => updateSelection(opIdx, sIdx, { op })}
                          />
                          {!NULL_OPS.has(s.op) && (
                            <label className="field field--grow">
                              <span>Valeur</span>
                              <SelectionValueField
                                sel={s}
                                customReferences={customReferences}
                                nativeEnums={nativeEnums}
                                op={s.op}
                                value={s.val}
                                onRawChange={(raw) =>
                                  updateSelection(opIdx, sIdx, {
                                    val: parseCondVal(s.op, raw),
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
                      );
                    });
                  })()}
                  {(() => {
                    const presentSel = new Set(op.selection.map((s) => s.dim));
                    const baseSel = DEFAULT_SEL_DIMS.filter((d) =>
                      selectionDims.includes(d),
                    );
                    const addableSel = [
                      ...baseSel.filter((d) => !presentSel.has(d)),
                      ...sortForDisplay(
                        selectionDims.filter(
                          (d) => !presentSel.has(d) && !baseSel.includes(d),
                        ),
                        (d) => d,
                      ),
                    ];
                    return (
                      <div className="rule-dest-add">
                        <span className="rule-dest-add__label">Ajouter un filtre</span>
                        {addableSel.map((dim) => (
                          <button
                            key={dim}
                            type="button"
                            className="rule-chip"
                            onClick={() => addSelectionForDim(opIdx, dim)}
                          >
                            + {dim}
                          </button>
                        ))}
                      </div>
                    );
                  })()}
                </div>

                {/* Destination */}
                <div className="rule-section rule-section--dest" style={{ marginTop: 0 }}>
                  <h4 className="rule-section__title">Destination</h4>
                  {(() => {
                    const visible = visibleDestDims(op);
                    const addable = sortForDisplay(
                      pilotableDims.filter((d) => !visible.includes(d)),
                      (d) => d,
                    );
                    return (
                      <>
                        {visible.map((dim) => {
                          const dest =
                            op.destination[dim] ?? { mode: 'inherit' as DestMode };
                          // Mode `map` : seules les caractéristiques ayant un
                          // attribut ciblant cette dimension sont proposées
                          // (compatibilité de type imposée par le moteur).
                          const viaOptions = sortForDisplay(
                            characteristics.filter((c) =>
                              c.attributes.some((a) => a.target_dimension === dim),
                            ),
                            (c) => c.code,
                          );
                          const viaChar = characteristics.find(
                            (c) => c.code === dest.via,
                          );
                          const attrOptions = sortForDisplay(
                            (viaChar?.attributes ?? []).filter(
                              (a) => a.target_dimension === dim,
                            ),
                            (a) => a.name,
                          );
                          // Mode `map_ref` : seules les références directes
                          // (patron B) auto-référentielles sur `dim` (host =
                          // target = dim, ex : `compte_parent` sur `account`).
                          const mapRefOptions = sortForDisplay(
                            customReferences.filter(
                              (r) =>
                                r.host_dimension === dim &&
                                r.target_dimension === dim,
                            ),
                            (r) => r.column,
                          );
                          const isDefault = DEFAULT_DEST_DIMS.includes(dim);
                          return (
                            <div key={dim} className="rule-dest-row">
                              <span className="rule-dest-label">{dim}</span>
                              <select
                                className={`rule-dest-mode rule-dest-mode--${dest.mode}`}
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
                                <option value="map" disabled={viaOptions.length === 0}>
                                  map (caractéristique)
                                </option>
                                <option
                                  value="map_ref"
                                  disabled={mapRefOptions.length === 0}
                                >
                                  map_ref (référence)
                                </option>
                              </select>
                              <span className="rule-dest-extra">
                                {dest.mode === 'inherit' && (
                                  <span className="rule-dest-muted">
                                    hérité de la source
                                  </span>
                                )}
                                {dest.mode === 'null' && (
                                  <span className="rule-dest-muted">
                                    forcé à vide (∅)
                                  </span>
                                )}
                                {dest.mode === 'override' && (
                                  <OverrideValueField
                                    dim={dim}
                                    value={dest.value ?? ''}
                                    onChange={(v) =>
                                      updateDestination(opIdx, dim, { value: v })
                                    }
                                  />
                                )}
                                {dest.mode === 'map' && (
                                  <>
                                    <select
                                      className="rule-dest-input"
                                      value={dest.via ?? ''}
                                      onChange={(e) =>
                                        updateDestination(opIdx, dim, {
                                          via: e.target.value,
                                          attr: '',
                                        })
                                      }
                                    >
                                      <option value="" disabled>
                                        — caractéristique —
                                      </option>
                                      {viaOptions.map((c) => (
                                        <option key={c.code} value={c.code}>
                                          {c.code}
                                        </option>
                                      ))}
                                    </select>
                                    <select
                                      className="rule-dest-input"
                                      value={dest.attr ?? ''}
                                      disabled={!dest.via}
                                      onChange={(e) =>
                                        updateDestination(opIdx, dim, {
                                          attr: e.target.value,
                                        })
                                      }
                                    >
                                      <option value="" disabled>
                                        — attribut —
                                      </option>
                                      {attrOptions.map((a) => (
                                        <option key={a.name} value={a.name}>
                                          {a.name}
                                        </option>
                                      ))}
                                    </select>
                                  </>
                                )}
                                {dest.mode === 'map_ref' && (
                                  <select
                                    className="rule-dest-input"
                                    value={dest.ref ?? ''}
                                    onChange={(e) =>
                                      updateDestination(opIdx, dim, {
                                        ref: e.target.value,
                                      })
                                    }
                                  >
                                    <option value="" disabled>
                                      — référence —
                                    </option>
                                    {mapRefOptions.map((r) => (
                                      <option key={r.column} value={r.column}>
                                        {r.column}
                                      </option>
                                    ))}
                                  </select>
                                )}
                              </span>
                              {isDefault ? (
                                <span aria-hidden="true" />
                              ) : (
                                <button
                                  type="button"
                                  className="rule-dest-remove"
                                  aria-label={`Retirer ${dim}`}
                                  title="Retirer cette dimension (repasse en inherit)"
                                  onClick={() => hideDest(opIdx, op.seq, dim)}
                                >
                                  ✕
                                </button>
                              )}
                            </div>
                          );
                        })}
                        {addable.length > 0 && (
                          <div className="rule-dest-add">
                            <span className="rule-dest-add__label">
                              Autres dimensions
                            </span>
                            {addable.map((dim) => (
                              <button
                                key={dim}
                                type="button"
                                className="rule-chip"
                                onClick={() => revealDest(op.seq, dim)}
                              >
                                + {dim}
                              </button>
                            ))}
                          </div>
                        )}
                      </>
                    );
                  })()}
                </div>
                </div>

                <div className="rule-op-summary">
                  <span className="rule-op-summary__tag">résumé</span>
                  <span>{summarizeOperation(op)}</span>
                </div>
              </div>
            ))}
            <button type="button" className="rule-add-btn" onClick={addOp}>
              + Ajouter une opération
            </button>

            {/* Aides consolidées (affichées une seule fois pour toutes les
                opérations afin de garder le formulaire compact). */}
            <div className="rule-help">
              <p className="rule-section__hint rule-op-legend">
                <strong>Opérateurs :</strong> <code>=</code> égal · <code>≠</code>{' '}
                différent · <code>&gt;</code> <code>&lt;</code> <code>≥</code>{' '}
                <code>≤</code> comparaisons · <code>∈</code> dans la liste ·{' '}
                <code>∅</code> est nul · <code>≠∅</code> non nul
              </p>
              <p className="rule-section__hint">
                <strong>Sélection :</strong> conditions combinées par ET (toutes
                doivent être vraies). Par défaut le filtre porte sur la valeur directe
                de la dimension ; l'icône <span className="rule-traverse-glyph">↳</span>{' '}
                « Traverser » permet de filtrer par attribut (caractéristique N1 ou
                référence directe, ex. <code>compte_parent</code>).
              </p>
              <p className="rule-section__hint">
                <strong>Destination :</strong> <code>account</code>, <code>flow</code>{' '}
                et <code>nature</code> sont affichées d'office ; ajoutez les autres
                dimensions via les boutons <strong>+</strong>. Les dimensions laissées
                en <code>inherit</code> conservent la valeur de la source.
              </p>
            </div>
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
// Modal de renommage de code (règle ou jeu de règles)
// =================================================================

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
                    {sortForDisplay(ruleOptions, (r) => `${r.code} — ${r.libelle}`).map((r) => (
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
    | { mode: 'duplicate'; draft: RuleDraft }
    | null
  >(null);
  const [renaming, setRenaming] = useState<string | null>(null);

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

  const handleRename = useCallback(
    async (newCode: string) => {
      if (!renaming) return;
      await api.masterData.rename('rules', renaming, newCode);
      setNotice({ kind: 'success', text: `Règle renommée : ${renaming} → ${newCode}.` });
      setRenaming(null);
      await load();
    },
    [renaming, load],
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

  // Duplication : on récupère la définition source et on ouvre la modale en mode
  // création (code éditable → pas de collision de PK) pré-remplie, avec un code
  // suggéré `{code}_COPIE`. L'utilisateur ajuste code/libellé puis enregistre.
  const openDuplicate = useCallback(async (code: string) => {
    try {
      const detail = await api.rules.get(code);
      setForm({
        mode: 'duplicate',
        draft: {
          code: `${detail.code}_COPIE`,
          libelle: `${detail.libelle} (copie)`,
          definition: asDefinition(detail.definition),
        },
      });
    } catch (err) {
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' });
    }
  }, []);

  async function handleSubmit(draft: RuleDraft) {
    // create et duplicate créent une nouvelle règle ; seul edit met à jour.
    if (form?.mode === 'edit') {
      await api.rules.update(draft.code, {
        libelle: draft.libelle,
        definition: draft.definition,
      });
      setNotice({ kind: 'success', text: 'Règle mise à jour.' });
    } else {
      await api.rules.create({
        code: draft.code,
        libelle: draft.libelle,
        definition: draft.definition,
      });
      setNotice({
        kind: 'success',
        text: form?.mode === 'duplicate' ? 'Règle dupliquée.' : 'Règle créée.',
      });
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
              className="btn btn--sm"
              onClick={() => void openDuplicate(info.row.original.code)}
            >
              Dupliquer
            </button>
            <button
              type="button"
              className="btn btn--sm"
              onClick={() => setRenaming(info.row.original.code)}
            >
              Renommer
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
    [openEdit, openDuplicate, handleRename, handleDelete],
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
            form.mode === 'create'
              ? { code: '', libelle: '', definition: emptyDefinition() }
              : form.draft
          }
          isEdit={form.mode === 'edit'}
          pilotableDims={pilotableDims}
          selectionDims={selectionDims}
          onSubmit={handleSubmit}
          onCancel={() => setForm(null)}
        />
      )}
      {renaming !== null && (
        <RenameModal
          oldCode={renaming}
          entityLabel="la règle"
          onConfirm={handleRename}
          onCancel={() => setRenaming(null)}
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
  const [renaming, setRenaming] = useState<string | null>(null);

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

  const handleRenameRuleset = useCallback(
    async (newCode: string) => {
      if (!renaming) return;
      await api.masterData.rename('rulesets', renaming, newCode);
      setNotice({ kind: 'success', text: `Jeu renommé : ${renaming} → ${newCode}.` });
      setRenaming(null);
      await load();
    },
    [renaming, load],
  );

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
                className="btn btn--sm"
                onClick={() => setRenaming(code)}
              >
                Renommer
              </button>
              <button
                type="button"
                className="btn btn--sm btn--danger"
                onClick={() => void handleDelete(code)}
              >
                Supprimer
              </button>
            </div>
          );
        },
      },
    ],
    [openEdit, handleRenameRuleset, handleDelete],
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
      {renaming !== null && (
        <RenameModal
          oldCode={renaming}
          entityLabel="le jeu de règles"
          onConfirm={handleRenameRuleset}
          onCancel={() => setRenaming(null)}
        />
      )}
    </>
  );
}

// =================================================================
// Composant racine
// =================================================================

export function RulesPage() {
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [dimsError, setDimsError] = useState<string | null>(null);
  const [dimToTable, setDimToTable] = useState<DimToTable>(DIM_TO_TABLE_FALLBACK);

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
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const rows = await api.dimensions.list();
        if (!cancelled) {
          setDims(rows);
          setDimsError(null);
        }
      } catch (err) {
        if (cancelled) return;
        setDims(BUILTIN_DIMS_FALLBACK);
        setDimsError(
          err instanceof Error
            ? `Impossible de charger la liste des dimensions (${err.message}). ` +
              `Utilisation du fallback builtin (12 dims, sans customs).`
            : 'Impossible de charger la liste des dimensions. Fallback builtin activé.',
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <DimRefContext.Provider value={dimToTable}>
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Règles de consolidation</h1>
      </div>

      {dimsError && (
        <div className="alert alert--error" role="alert">
          ⚠ {dimsError}
        </div>
      )}

      <BibliothequeTab dims={dims} />
    </section>
    </DimRefContext.Provider>
  );
}

export function JeuxReglesPage() {
  return (
    <DimRefContext.Provider value={DIM_TO_TABLE_FALLBACK}>
      <section className="page">
        <div className="page__header">
          <h1 className="page__title">Jeux de règles</h1>
        </div>
        <JeuxTab />
      </section>
    </DimRefContext.Provider>
  );
}
