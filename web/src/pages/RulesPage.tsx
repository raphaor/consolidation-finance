// Page « Règles » : gère la bibliothèque de règles de consolidation, les
// jeux de règles (composition ordonnée + exécution) et les dimensions custom.
//
// Trois sous-onglets :
//   - Bibliothèque : CRUD sur les règles (définition JSON éditoriale)
//   - Jeux de règles : CRUD sur les rulesets + exécution → rapport
//   - Dimensions : gestion des dimensions custom (catégorie Analytical)

import {
  type FormEvent,
  createContext,
  useCallback,
  useContext,
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
  MasterTable,
  Operation,
  ReferenceInfo,
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
const LEVELS_LIST = ['corporate', 'reclassified', 'converted', 'consolidated'];
const OPS = ['=', '!=', '>', '<', '>=', '<=', 'IN', 'IS NULL', 'IS NOT NULL'];
const COEFF_TYPES = ['pct_integration', 'pct_interet', 'constant'];

const NULL_OPS = new Set(['IS NULL', 'IS NOT NULL']);

// Fallback des 12 dimensions built-in si l'API /api/meta/dimensions est
// injoignable (serveur obsolète, réseau en panne). Miroir de
// `engine/src/dimensions.rs::builtin_dims()`. Les dimensions custom (ajoutées
// par l'utilisateur) ne seront pas présentes, mais l'éditeur reste utilisable.
const BUILTIN_DIMS_FALLBACK: DimensionInfo[] = [
  { name: 'scenario',     category: 'Fixed',      custom: false, label: 'Définition de consolidation', pilotable: false },
  { name: 'entity',       category: 'Active',     custom: false, label: 'Entité',     pilotable: true  },
  { name: 'entry_period', category: 'Fixed',      custom: false, label: 'Exercice',   pilotable: false },
  { name: 'period',       category: 'Fixed',      custom: false, label: 'Période',    pilotable: false },
  { name: 'account',      category: 'Active',     custom: false, label: 'Compte',     pilotable: true  },
  { name: 'flow',         category: 'Active',     custom: false, label: 'Flux',       pilotable: true  },
  { name: 'currency',     category: 'Fixed',      custom: false, label: 'Devise',     pilotable: false },
  { name: 'nature',       category: 'Active',     custom: false, label: 'Nature',     pilotable: true  },
  { name: 'partner',      category: 'Analytical', custom: false, label: 'Partenaire', pilotable: true  },
  { name: 'share',        category: 'Analytical', custom: false, label: 'Quote-part', pilotable: true  },
  { name: 'analysis',     category: 'Analytical', custom: false, label: 'Analyse 1',  pilotable: true  },
  { name: 'analysis2',    category: 'Analytical', custom: false, label: 'Analyse 2',  pilotable: true  },
];

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

/// Parse la valeur d'une condition selon l'opérateur :
/// - `IN` : la string est éclatée par virgule → tableau (le moteur attend un
///   tableau JSON pour `push_condition`, cf. `rules.rs:427`).
/// - autres : retourne la string brute.
function parseCondVal(op: string, raw: string): unknown {
  if (op === 'IN') {
    return raw
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }
  return raw;
}

/// Formate la valeur d'une condition pour l'affichage dans un input texte :
/// - tableau → join par ', ' (réciproque de `parseCondVal`).
/// - autres → toString.
function formatCondVal(val: unknown): string {
  if (Array.isArray(val)) return val.join(', ');
  return String(val ?? '');
}

/// Parse un multiplicateur : accepte la virgule ou le point comme séparateur
/// décimal (la locale fr utilise la virgule, mais JS Number() exige le point).
/// Retourne NaN si la valeur n'est pas un nombre valide.
function parseMultiplicateur(raw: string): number {
  const cleaned = raw.trim().replace(',', '.');
  if (cleaned === '' || cleaned === '-') return NaN;
  return Number(cleaned);
}

// Mapping dimension → table master data pour les listes déroulantes contextuelles.
type DimToTable = Record<string, { table: MasterTable; pkCol: string }>;

// Fallback mode-dégradé si `GET /api/meta/references` est injoignable (serveur
// obsolète, réseau en panne) : miroir codé en dur du graphe de références
// serveur (`engine/src/references.rs`). En fonctionnement normal, ce mapping est
// **dérivé de l'API** (cf. RulesPage / DimRefContext) et non de cette constante.
// Les dimensions libres (analysis, analysis2, custom) sont absentes → saisie
// texte. `partner` / `share` sont des rôles sur la liste des entités.
const DIM_TO_TABLE_FALLBACK: DimToTable = {
  scenario: { table: 'scenarios', pkCol: 'code' },
  entity: { table: 'entities', pkCol: 'code' },
  entry_period: { table: 'periods', pkCol: 'code' },
  period: { table: 'periods', pkCol: 'code' },
  account: { table: 'accounts', pkCol: 'code' },
  flow: { table: 'flows', pkCol: 'code' },
  currency: { table: 'currencies', pkCol: 'code_iso' },
  nature: { table: 'natures', pkCol: 'code' },
  partner: { table: 'entities', pkCol: 'code' },
  share: { table: 'entities', pkCol: 'code' },
  methode: { table: 'methods', pkCol: 'code' },
};

// Construit le mapping dimension → table depuis le graphe de références exposé
// par l'API. On ne garde que les sources `stg_entry` (dimensions d'écriture) et
// `perimeter` (scope des règles, dont `methode`) : ce sont les colonnes
// pilotables dans l'éditeur. `target_table` est déjà un nom de table master data.
function buildDimToTable(refs: ReferenceInfo[]): DimToTable {
  const out: DimToTable = {};
  for (const r of refs) {
    if (r.table === 'stg_entry' || r.table === 'perimeter') {
      out[r.column] = {
        table: r.target_table as MasterTable,
        pkCol: r.target_column,
      };
    }
  }
  return out;
}

// Contexte fournissant le mapping dimension → table aux champs de saisie
// (`useDimValues`), évitant de threader la prop à travers toute la hiérarchie.
// Défaut = fallback, pour rester fonctionnel avant le chargement / en échec.
const DimRefContext = createContext<DimToTable>(DIM_TO_TABLE_FALLBACK);

// Cache module-level des valeurs de dimensions (évite les refetchs à chaque
// ouverture de modale). Clé = nom de dimension.
const dimValuesCache = new Map<string, string[]>();

/// Hook : charge les valeurs possibles d'une dimension depuis la master data.
/// Retourne `values = []` si la dimension n'a pas de table associée (saisie libre).
function useDimValues(dim: string): { values: string[]; loading: boolean } {
  const dimToTable = useContext(DimRefContext);
  const [values, setValues] = useState<string[]>(
    dimValuesCache.get(dim) ?? [],
  );
  const [loading, setLoading] = useState(!dimValuesCache.has(dim));

  useEffect(() => {
    const mapping = dimToTable[dim];
    if (!mapping) {
      setValues([]);
      setLoading(false);
      return;
    }
    if (dimValuesCache.has(dim)) {
      setValues(dimValuesCache.get(dim)!);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void (async () => {
      try {
        const rows = await api.masterData.list(mapping.table);
        const codes = rows
          .map((r) => {
            const v = (r as Record<string, unknown>)[mapping.pkCol];
            return v == null ? '' : String(v);
          })
          .filter((s) => s.length > 0);
        if (cancelled) return;
        dimValuesCache.set(dim, codes);
        setValues(codes);
      } catch {
        if (cancelled) return;
        dimValuesCache.set(dim, []);
        setValues([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [dim, dimToTable]);

  return { values, loading };
}

/// Champ de saisie pour l'opérateur `IN` (valeurs multiples séparées par des
/// virgules). Maintient un **buffer texte local** : on ne reformate PAS depuis
/// la valeur parsée à chaque frappe, car `parseCondVal` filtre les segments
/// vides → la virgule en cours de saisie (segment vide qui la suit) serait
/// supprimée immédiatement, rendant impossible la saisie de « a, b ». Le texte
/// brut tapé est conservé ; le parent reçoit le tableau parsé via `onRawChange`.
function InValueField({
  value,
  onRawChange,
}: {
  value: unknown;
  onRawChange: (raw: string) => void;
}) {
  const [text, setText] = useState(() => formatCondVal(value));
  return (
    <input
      type="text"
      value={text}
      placeholder="valeurs séparées par virgules"
      onChange={(e) => {
        setText(e.target.value);
        onRawChange(e.target.value);
      }}
    />
  );
}

/// Champ de saisie de valeur de condition, adaptatif :
/// - si la dimension a une liste de valeurs connue (master data) et l'opérateur
///   n'est pas IN → `<select>` avec les valeurs + option vide.
/// - si op = IN → `<input type="text">` (saisie multi-valeurs séparées par virgules).
/// - sinon → `<input type="text">` (saisie libre).
function ValueField({
  dim,
  op,
  value,
  onRawChange,
}: {
  dim: string;
  op: string;
  value: unknown;
  onRawChange: (raw: string) => void;
}) {
  const { values } = useDimValues(dim);

  // IN : toujours en texte libre (saisie multi-valeurs), avec buffer local pour
  // préserver les virgules pendant la frappe (cf. InValueField).
  if (op === 'IN') {
    return <InValueField value={value} onRawChange={onRawChange} />;
  }

  // Liste déroulante si des valeurs sont disponibles.
  if (values.length > 0) {
    const current = formatCondVal(value);
    return (
      <select
        value={values.includes(current) ? current : ''}
        onChange={(e) => onRawChange(e.target.value)}
      >
        <option value="" disabled>
          — choisir —
        </option>
        {values.map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
        {/* Si la valeur actuelle n'est pas dans la liste (ex: règle existante
            avec une valeur supprimée de la master data), on l'affiche quand même. */}
        {current && !values.includes(current) && (
          <option value={current}>{current} (hors liste)</option>
        )}
      </select>
    );
  }

  // Saisie libre par défaut.
  return (
    <input
      type="text"
      value={formatCondVal(value)}
      onChange={(e) => onRawChange(e.target.value)}
    />
  );
}

/// Champ de valeur d'une destination `override`, adaptatif : si la dimension a
/// une master data connue (cf. `DIM_TO_TABLE`, ex. `account` → liste des
/// comptes), on propose un `<select>` ; sinon saisie libre. Même logique que
/// `ValueField` mais pour une valeur unique (pas d'opérateur).
function OverrideValueField({
  dim,
  value,
  onChange,
}: {
  dim: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const { values } = useDimValues(dim);
  if (values.length > 0) {
    return (
      <select
        className="rule-dest-input"
        value={values.includes(value) ? value : ''}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="" disabled>
          — choisir —
        </option>
        {values.map((v) => (
          <option key={v} value={v}>
            {v}
          </option>
        ))}
        {value && !values.includes(value) && (
          <option value={value}>{value} (hors liste)</option>
        )}
      </select>
    );
  }
  return (
    <input
      type="text"
      className="rule-dest-input"
      placeholder={`valeur ${dim}`}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
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
  const [draft, setDraft] = useState<RuleDraft>(initial);
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

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
              <code>IN</code> (ex : <code>methode IN globale, proportionnelle</code>).
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

                {/* Sélection */}
                <div className="rule-section" style={{ marginTop: 0 }}>
                  <h4 className="rule-section__title">Sélection</h4>
                  <p className="rule-section__hint">
                    Conditions combinées par <strong>ET</strong> (toutes doivent être vraies).
                  </p>
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
                          <ValueField
                            dim={s.dim}
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
                          <OverrideValueField
                            dim={dim}
                            value={dest.value ?? ''}
                            onChange={(v) =>
                              updateDestination(opIdx, dim, { value: v })
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
    | { mode: 'duplicate'; draft: RuleDraft }
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
              className="btn btn--sm btn--danger"
              onClick={() => void handleDelete(info.row.original.code)}
            >
              Supprimer
            </button>
          </div>
        ),
      },
    ],
    [openEdit, openDuplicate, handleDelete],
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
            </div>
          );
        },
      },
    ],
    [openEdit, handleDelete],
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
  const [dimsError, setDimsError] = useState<string | null>(null);
  // Mapping dimension → table master data, dérivé du graphe de références
  // serveur (`GET /api/meta/references`). Fallback codé en dur si injoignable.
  const [dimToTable, setDimToTable] = useState<DimToTable>(DIM_TO_TABLE_FALLBACK);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const refs = await api.references();
        if (!cancelled) setDimToTable(buildDimToTable(refs));
      } catch {
        // Mode dégradé : on conserve le fallback (les dropdowns restent
        // fonctionnels sur les dimensions built-in).
        if (!cancelled) setDimToTable(DIM_TO_TABLE_FALLBACK);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Charge les dimensions une fois pour toutes (Bibliothèque en a besoin pour
  // construire les listes dynamiques pilotableDims / selectionDims).
  // En cas d'échec (serveur obsolète, /api/meta/dimensions absent), on bascule
  // sur le fallback builtin pour que l'éditeur reste utilisable, et on signale
  // le mode dégradé via dimsError.
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
    </DimRefContext.Provider>
  );
}
