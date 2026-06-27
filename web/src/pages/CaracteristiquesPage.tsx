// Page « Attributs de dimension » — refonte alignée sur Règles / Coefficients.
//
// Un attribut enrichit une dimension de base. Deux formes (un seul mécanisme
// moteur chacune) :
//
//   • Caractéristique (liste propre)  → on définit ses valeurs (+ champs),
//        puis on classe les membres de la dimension de base.
//        Sous le capot : caractéristique N1 (`car_<code>`) + attributs N2.
//        (characteristics.rs)
//   • Emprunt (réutilise une dimension) → les valeurs sont celles d'une autre
//        dimension (y compris elle-même : hiérarchie). Pas de champs.
//        Sous le capot : référence directe (patron B). (custom_references.rs)
//
// Deux sous-onglets (comme la page Règles) :
//   - Attributs de dimension : liste unifiée + détail de la sélection.
//   - Listes de valeurs       : nomenclatures réutilisables (`lst_<code>`),
//        qu'un champ de caractéristique peut viser. (value_lists.rs)
//
// Chaque ligne et chaque détail portent une phrase de relecture « en clair »
// (cf. `rule-op-summary` de Règles) pour ne pas avoir à déchiffrer les champs.

import { type FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import { formatOptionLabel, sortForDisplay } from '../utils/format';
import type {
  Characteristic,
  CustomReference,
  MasterTable,
  ReferenceInfo,
  ValueList,
} from '../types';

type Row = Record<string, unknown>;
type Notice = { kind: 'success' | 'error'; text: string } | null;
type Opt = { v: string; lbl: string };
type Subtab = 'attributs' | 'listes';

// Dimension classable / ciblable : dérivée du graphe de références (toute
// dimension d'écriture ayant une master data, c.-à-d. une référence depuis
// `stg_entry`). `table` est le nom d'API master data, `key` sa colonne clé.
interface DimInfo {
  dim: string;
  table: string;
  key: string;
}

// Sélection courante : soit une caractéristique, soit un emprunt (identifié par
// "host.column").
type Selection = { kind: 'char'; code: string } | { kind: 'ref'; id: string } | null;

const refId = (r: CustomReference) => `${r.host_dimension}.${r.column}`;

export function CaracteristiquesPage() {
  const [subtab, setSubtab] = useState<Subtab>('attributs');
  const [chars, setChars] = useState<Characteristic[]>([]);
  const [refs, setRefs] = useState<CustomReference[]>([]);
  const [lists, setLists] = useState<ValueList[]>([]);
  const [dims, setDims] = useState<DimInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);

  const notifyErr = useCallback(
    (err: unknown) =>
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' }),
    [],
  );

  // Une cible (de champ N2) est une liste si son code figure dans `lists`,
  // sinon c'est une dimension. Sert à savoir où puiser ses options.
  const isList = useCallback((target: string) => lists.some((l) => l.code === target), [lists]);

  // Charge les options (valeurs) d'une cible, qu'elle soit une dimension ou une
  // liste de valeurs.
  const loadOptions = useCallback(
    async (target: string): Promise<Opt[]> => {
      try {
        if (isList(target)) {
          const rows = (await api.valueLists.listValues(target)) as Row[];
          return sortForDisplay(
            rows.map((r) => ({
              v: String(r['code'] ?? ''),
              lbl: String(r['libelle'] ?? r['code'] ?? ''),
            })),
            (o) => o.lbl,
          );
        }
        const di = dims.find((d) => d.dim === target);
        if (!di) return [];
        const rows = (await api.masterData.list(di.table as MasterTable)) as Row[];
        return sortForDisplay(
          rows.map((r) => ({
            v: String(r[di.key] ?? ''),
            lbl: String(r['libelle'] ?? r[di.key] ?? ''),
          })),
          (o) => o.lbl,
        );
      } catch {
        return [];
      }
    },
    [dims, isList],
  );

  const loadAll = useCallback(async () => {
    setLoading(true);
    try {
      const [cs, rs, ls, references] = await Promise.all([
        api.characteristics.list(),
        api.customReferences.list(),
        api.valueLists.list(),
        api.references(),
      ]);
      setChars(sortForDisplay(cs, (c) => c.code));
      setRefs(sortForDisplay(rs, (r) => refId(r)));
      setLists(sortForDisplay(ls, (l) => formatOptionLabel(l.code, l.libelle)));
      const seen = new Set<string>();
      const ds: DimInfo[] = (references as ReferenceInfo[])
        .filter((r) => r.table === 'stg_entry')
        .filter((r) => (seen.has(r.column) ? false : (seen.add(r.column), true)))
        .map((r) => ({ dim: r.column, table: r.target_table, key: r.target_column }));
      setDims(sortForDisplay(ds, (d) => d.dim));
    } catch (err) {
      notifyErr(err);
    } finally {
      setLoading(false);
    }
  }, [notifyErr]);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Attributs de dimension</h1>
        <div className="page__actions">
          <button type="button" className="btn" onClick={() => void loadAll()} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      <p className="page__meta">
        Ajoutez une information de classement à une dimension : une{' '}
        <strong>caractéristique</strong> (liste propre que vous définissez, avec en option des
        champs) ou un <strong>emprunt</strong> des valeurs d'une autre dimension. Les{' '}
        <strong>listes de valeurs</strong> sont des nomenclatures réutilisables — pas des
        dimensions.
      </p>

      <div className="subtabs">
        <button
          type="button"
          className={`subtab ${subtab === 'attributs' ? 'subtab--active' : ''}`}
          onClick={() => setSubtab('attributs')}
        >
          Attributs de dimension
        </button>
        <button
          type="button"
          className={`subtab ${subtab === 'listes' ? 'subtab--active' : ''}`}
          onClick={() => setSubtab('listes')}
        >
          Listes de valeurs
        </button>
      </div>

      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      {subtab === 'attributs' ? (
        <AttributsTab
          chars={chars}
          refs={refs}
          lists={lists}
          dims={dims}
          loadOptions={loadOptions}
          isList={isList}
          onNotice={setNotice}
          notifyErr={notifyErr}
          reloadAll={loadAll}
        />
      ) : (
        <ListesTab lists={lists} onNotice={setNotice} notifyErr={notifyErr} reloadAll={loadAll} />
      )}
    </section>
  );
}

// ─── Phrases de relecture « en clair » ────────────────────────────────────────

function recapChar(c: Characteristic): string {
  const champs =
    c.attributes.length === 0
      ? 'sans champ'
      : `${c.attributes.length} champ${c.attributes.length > 1 ? 's' : ''}`;
  return `Caractéristique sur « ${c.base_dimension} » · ${champs}`;
}

function recapRef(r: CustomReference): string {
  const hier = r.target_dimension === r.host_dimension ? ' (hiérarchie)' : '';
  const natif = r.native ? ' · natif' : '';
  return `Sur « ${r.host_dimension} », emprunte « ${r.target_dimension} »${hier}${natif}`;
}

// ═══════════════════════════════════════════════════════════════════════════
// Sous-onglet « Attributs de dimension »
// ═══════════════════════════════════════════════════════════════════════════

function AttributsTab({
  chars,
  refs,
  lists,
  dims,
  loadOptions,
  isList,
  onNotice,
  notifyErr,
  reloadAll,
}: {
  chars: Characteristic[];
  refs: CustomReference[];
  lists: ValueList[];
  dims: DimInfo[];
  loadOptions: (target: string) => Promise<Opt[]>;
  isList: (target: string) => boolean;
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  reloadAll: () => Promise<void>;
}) {
  // La page se consulte **par dimension** : on choisit d'abord une dimension de
  // faits, puis on voit (et crée) ses attributs. `dim` étant toujours un axe
  // d'écriture (les `dims` viennent des références `stg_entry`), les emprunts dont
  // l'hôte n'est pas une dimension de faits (ex. FK natives `consolidation.*`)
  // n'apparaissent jamais. Cf. typologie « une table = un seul foyer ».
  const [dim, setDim] = useState('');
  const [selected, setSelected] = useState<Selection>(null);
  const [creating, setCreating] = useState(false);

  // Changer de dimension réinitialise l'attribut sélectionné.
  useEffect(() => {
    setSelected(null);
  }, [dim]);

  const selectedChar = useMemo(
    () => (selected?.kind === 'char' ? chars.find((c) => c.code === selected.code) ?? null : null),
    [selected, chars],
  );
  const selectedRef = useMemo(
    () => (selected?.kind === 'ref' ? refs.find((r) => refId(r) === selected.id) ?? null : null),
    [selected, refs],
  );

  // Attributs de la dimension courante : caractéristiques de base `dim` + emprunts
  // hôtés par `dim`.
  const dimChars = useMemo(() => chars.filter((c) => c.base_dimension === dim), [chars, dim]);
  const dimRefs = useMemo(() => refs.filter((r) => r.host_dimension === dim), [refs, dim]);

  const deleteChar = async (code: string) => {
    if (!window.confirm(`Supprimer la caractéristique « ${code} » et ses valeurs ?`)) return;
    try {
      await api.characteristics.remove(code);
      if (selected?.kind === 'char' && selected.code === code) setSelected(null);
      onNotice({ kind: 'success', text: 'Caractéristique supprimée.' });
      await reloadAll();
    } catch (err) {
      notifyErr(err);
    }
  };

  const deleteRef = async (r: CustomReference) => {
    if (!window.confirm(`Supprimer l'emprunt « ${refId(r)} » ?`)) return;
    try {
      await api.customReferences.remove(r.host_dimension, r.column);
      if (selected?.kind === 'ref' && selected.id === refId(r)) setSelected(null);
      onNotice({ kind: 'success', text: 'Emprunt supprimé.' });
      await reloadAll();
    } catch (err) {
      notifyErr(err);
    }
  };

  const empty = dimChars.length === 0 && dimRefs.length === 0;

  return (
    <>
      {/* Sélecteur de dimension : on consulte les attributs dimension par dimension. */}
      <div className="attr-dimbar">
        <label className="field">
          <span>Dimension</span>
          <select value={dim} onChange={(e) => setDim(e.target.value)}>
            <option value="" disabled>
              — choisir une dimension —
            </option>
            {dims.map((d) => (
              <option key={d.dim} value={d.dim}>
                {d.dim}
              </option>
            ))}
          </select>
        </label>
        {dim !== '' && (
          <button type="button" className="btn btn--primary" onClick={() => setCreating(true)}>
            + Nouvel attribut
          </button>
        )}
      </div>

      {dim === '' ? (
        <div className="rule-section">
          <p className="muted" style={{ margin: 0 }}>
            Choisissez une dimension pour voir et gérer ses attributs.
          </p>
        </div>
      ) : (
        <div className="attr-layout">
          {/* ── Attributs de la dimension ── */}
          <div>
            <div className="attr-list">
              {empty && <div className="attr-empty">Aucun attribut sur « {dim} ».</div>}

              {dimChars.map((c) => {
                const active = selected?.kind === 'char' && selected.code === c.code;
                return (
                  <div
                    key={`char-${c.code}`}
                    className={`attr-item ${active ? 'is-selected' : ''}`}
                    onClick={() => setSelected({ kind: 'char', code: c.code })}
                  >
                    <div className="attr-item__head">
                      <span className="attr-item__code">{c.code}</span>
                      <span className="attr-badge attr-badge--char">caractéristique</span>
                      <button
                        type="button"
                        className="attr-item__del"
                        aria-label={`Supprimer ${c.code}`}
                        title="Supprimer"
                        onClick={(e) => {
                          e.stopPropagation();
                          void deleteChar(c.code);
                        }}
                      >
                        ✕
                      </button>
                    </div>
                    <div className="attr-item__recap">{recapChar(c)}</div>
                  </div>
                );
              })}

              {dimRefs.map((r) => {
                const id = refId(r);
                const active = selected?.kind === 'ref' && selected.id === id;
                return (
                  <div
                    key={`ref-${id}`}
                    className={`attr-item ${active ? 'is-selected' : ''}`}
                    onClick={() => setSelected({ kind: 'ref', id })}
                  >
                    <div className="attr-item__head">
                      <span className="attr-item__code">{r.column}</span>
                      <span
                        className={`attr-badge ${r.native ? 'attr-badge--native' : 'attr-badge--ref'}`}
                      >
                        {r.native ? 'emprunt natif' : 'emprunt'}
                      </span>
                      {!r.native && (
                        <button
                          type="button"
                          className="attr-item__del"
                          aria-label={`Supprimer ${id}`}
                          title="Supprimer"
                          onClick={(e) => {
                            e.stopPropagation();
                            void deleteRef(r);
                          }}
                        >
                          ✕
                        </button>
                      )}
                    </div>
                    <div className="attr-item__recap">{recapRef(r)}</div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* ── Détail de la sélection ── */}
          <div>
            {selectedChar ? (
              <CharacteristicDetail
                char={selectedChar}
                dims={dims}
                lists={lists}
                isList={isList}
                loadOptions={loadOptions}
                onNotice={onNotice}
                notifyErr={notifyErr}
                onChanged={reloadAll}
              />
            ) : selectedRef ? (
              <ReferenceDetail
                reference={selectedRef}
                loadOptions={loadOptions}
                onNotice={onNotice}
                notifyErr={notifyErr}
              />
            ) : (
              <div className="rule-section">
                <p className="muted" style={{ margin: 0 }}>
                  Sélectionnez un attribut à gauche pour le détailler, ou créez-en un nouveau sur
                  «&nbsp;{dim}&nbsp;».
                </p>
              </div>
            )}
          </div>
        </div>
      )}

      {creating && (
        <CreateModal
          dims={dims}
          initialBase={dim}
          notifyErr={notifyErr}
          onCancel={() => setCreating(false)}
          onCreated={async (sel, msg) => {
            setCreating(false);
            onNotice({ kind: 'success', text: msg });
            await reloadAll();
            setSelected(sel);
          }}
        />
      )}
    </>
  );
}

// ─── Modale de création (caractéristique ou emprunt) ──────────────────────────

function CreateModal({
  dims,
  initialBase,
  notifyErr,
  onCancel,
  onCreated,
}: {
  dims: DimInfo[];
  initialBase: string;
  notifyErr: (err: unknown) => void;
  onCancel: () => void;
  onCreated: (sel: Selection, msg: string) => void;
}) {
  const [kind, setKind] = useState<'char' | 'ref'>('char');
  // La dimension de base est imposée par le contexte (dimension sélectionnée) :
  // pré-remplie et verrouillée dans le formulaire.
  const [base] = useState(initialBase);
  const [code, setCode] = useState('');
  const [libelle, setLibelle] = useState('');
  const [target, setTarget] = useState(''); // dimension empruntée (emprunt)
  const [submitting, setSubmitting] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    try {
      if (kind === 'char') {
        await api.characteristics.create({ code, libelle, base_dimension: base });
        onCreated({ kind: 'char', code }, `Caractéristique « ${code} » créée.`);
      } else {
        await api.customReferences.create({
          host_dimension: base,
          column: code,
          target_dimension: target,
        });
        onCreated({ kind: 'ref', id: `${base}.${code}` }, `Emprunt « ${base}.${code} » créé.`);
      }
    } catch (err) {
      notifyErr(err);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="modal__backdrop" onClick={onCancel}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">Nouvel attribut de dimension</div>
        <form className="modal__body" onSubmit={submit}>
          {/* 1. Type */}
          <div className="rule-section" style={{ marginTop: 0 }}>
            <h3 className="rule-section__title">1. Type d'attribut</h3>
            <label style={{ display: 'block', cursor: 'pointer', marginBottom: 4 }}>
              <input
                type="radio"
                name="kind"
                checked={kind === 'char'}
                onChange={() => setKind('char')}
              />{' '}
              <strong>Caractéristique</strong>{' '}
              <span className="muted">— une liste propre que je définis (+ champs optionnels)</span>
            </label>
            <label style={{ display: 'block', cursor: 'pointer' }}>
              <input
                type="radio"
                name="kind"
                checked={kind === 'ref'}
                onChange={() => setKind('ref')}
              />{' '}
              <strong>Emprunt</strong>{' '}
              <span className="muted">— réutilise les valeurs d'une dimension existante</span>
            </label>
          </div>

          {/* 2. Définition (la dimension de base vient du contexte, verrouillée) */}
          <div className="rule-section">
            <h3 className="rule-section__title">2. Définition</h3>
            <div className="form-grid">
              <label className="field">
                <span>Dimension de base</span>
                <select value={base} disabled>
                  <option value={base}>{base}</option>
                </select>
              </label>
              <label className="field">
                <span>Code •</span>
                <input
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  placeholder={kind === 'char' ? 'comportement' : 'compte_parent'}
                  required
                />
              </label>
              {kind === 'char' && (
                <label className="field">
                  <span>Libellé</span>
                  <input value={libelle} onChange={(e) => setLibelle(e.target.value)} />
                </label>
              )}
              {kind === 'ref' && (
                <label className="field">
                  <span>Dimension empruntée •</span>
                  <select value={target} onChange={(e) => setTarget(e.target.value)} required>
                    <option value="" disabled>
                      — choisir —
                    </option>
                    {dims.map((d) => (
                      <option key={d.dim} value={d.dim}>
                        {d.dim}
                        {d.dim === base ? ' (hiérarchie)' : ''}
                      </option>
                    ))}
                  </select>
                </label>
              )}
            </div>
            <p className="rule-section__hint" style={{ marginTop: 8, marginBottom: 0 }}>
              {kind === 'char'
                ? 'Vous saisirez ensuite les valeurs (et les champs) puis classerez les membres.'
                : "L'emprunt n'a pas de champs : ses valeurs sont celles de la dimension empruntée. Empruntez la dimension à elle-même pour une hiérarchie (ex. compte_parent)."}
            </p>
          </div>

          <div className="form-actions">
            <button type="button" className="btn" onClick={onCancel} disabled={submitting}>
              Annuler
            </button>
            <button type="submit" className="btn btn--primary" disabled={submitting}>
              {submitting ? 'Création…' : 'Créer'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

// ─── Détail d'une caractéristique : champs, valeurs, affectation ──────────────

function CharacteristicDetail({
  char,
  dims,
  lists,
  isList,
  loadOptions,
  onNotice,
  notifyErr,
  onChanged,
}: {
  char: Characteristic;
  dims: DimInfo[];
  lists: ValueList[];
  isList: (target: string) => boolean;
  loadOptions: (target: string) => Promise<Opt[]>;
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  onChanged: () => Promise<void>;
}) {
  const [editLibelle, setEditLibelle] = useState<string | null>(null);
  const [values, setValues] = useState<Row[]>([]);
  const [optCache, setOptCache] = useState<Record<string, Opt[]>>({});
  const [newField, setNewField] = useState({
    name: '',
    libelle: '',
    kind: 'list' as 'list' | 'dim',
    target: '',
  });
  const [newValue, setNewValue] = useState<Record<string, string>>({});
  const [assignForm, setAssignForm] = useState({ member: '', value: '' });

  const reload = useCallback(async () => {
    try {
      const vals = (await api.characteristics.listValues(char.code)) as Row[];
      setValues(sortForDisplay(vals, (r) => String(r['code'] ?? '')));
      // Options : la dimension de base + la cible de chaque champ (dim ou liste).
      const targets = new Set<string>([
        char.base_dimension,
        ...char.attributes.map((a) => a.target_dimension),
      ]);
      const cache: Record<string, Opt[]> = {};
      await Promise.all(
        [...targets].map(async (t) => {
          cache[t] = await loadOptions(t);
        }),
      );
      setOptCache(cache);
    } catch (err) {
      notifyErr(err);
    }
  }, [char, loadOptions, notifyErr]);

  useEffect(() => {
    setEditLibelle(null);
    setNewValue({});
    setAssignForm({ member: '', value: '' });
    void reload();
  }, [reload]);

  async function submitField(e: FormEvent) {
    e.preventDefault();
    try {
      await api.characteristics.addAttribute(char.code, {
        name: newField.name,
        libelle: newField.libelle,
        target_dimension: newField.target,
      });
      onNotice({ kind: 'success', text: `Champ « ${newField.name} » ajouté.` });
      setNewField({ name: '', libelle: '', kind: 'list', target: '' });
      await onChanged();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteField(name: string) {
    if (!window.confirm(`Supprimer le champ « ${name} » ?`)) return;
    try {
      await api.characteristics.removeAttribute(char.code, name);
      onNotice({ kind: 'success', text: 'Champ supprimé.' });
      await onChanged();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitValue(e: FormEvent) {
    e.preventDefault();
    const payload: Row = { code: newValue['code'] ?? '' };
    if ((newValue['libelle'] ?? '') !== '') payload['libelle'] = newValue['libelle'];
    for (const a of char.attributes) {
      const v = newValue[a.name] ?? '';
      if (v !== '') payload[a.name] = v;
    }
    try {
      await api.characteristics.createValue(char.code, payload);
      onNotice({ kind: 'success', text: 'Valeur créée.' });
      setNewValue({});
      await reload();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteValue(valueCode: string) {
    if (!window.confirm(`Supprimer la valeur « ${valueCode} » ?`)) return;
    try {
      await api.characteristics.removeValue(char.code, valueCode);
      onNotice({ kind: 'success', text: 'Valeur supprimée.' });
      await reload();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function saveLibelle(e: FormEvent) {
    e.preventDefault();
    if (editLibelle === null) return;
    try {
      await api.characteristics.update(char.code, { libelle: editLibelle });
      onNotice({ kind: 'success', text: 'Libellé mis à jour.' });
      setEditLibelle(null);
      await onChanged();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitAssign(e: FormEvent) {
    e.preventDefault();
    try {
      await api.characteristics.assign(char.code, {
        member: assignForm.member,
        value: assignForm.value === '' ? null : assignForm.value,
      });
      onNotice({
        kind: 'success',
        text:
          assignForm.value === ''
            ? `Membre « ${assignForm.member} » déclassé.`
            : `Membre « ${assignForm.member} » classé en « ${assignForm.value} ».`,
      });
      setAssignForm({ member: '', value: '' });
    } catch (err) {
      notifyErr(err);
    }
  }

  const baseOptions = optCache[char.base_dimension] ?? [];

  // Phrase « en clair » : ce que fait la caractéristique, champs nommés inclus.
  const champsClause =
    char.attributes.length === 0
      ? 'Aucun champ associé.'
      : `Chaque valeur porte ${char.attributes.length > 1 ? 'les champs' : 'le champ'} ${char.attributes
          .map((a) => `« ${a.libelle || a.name} »`)
          .join(', ')}.`;

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
        <h2 className="page__title" style={{ fontSize: '1.05rem' }}>
          {char.code}
        </h2>
        <span className="attr-badge attr-badge--char">caractéristique</span>
      </div>

      {editLibelle === null ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
          <span className="muted" style={{ fontSize: '0.875rem' }}>
            {char.libelle || <em>sans libellé</em>}
          </span>
          <button
            type="button"
            className="btn btn--sm"
            onClick={() => setEditLibelle(char.libelle ?? '')}
          >
            Modifier le libellé
          </button>
        </div>
      ) : (
        <form
          style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}
          onSubmit={saveLibelle}
        >
          <input
            className="field__input"
            value={editLibelle}
            onChange={(e) => setEditLibelle(e.target.value)}
            autoFocus
            style={{ flex: 1 }}
          />
          <button type="submit" className="btn btn--sm btn--primary">
            Enregistrer
          </button>
          <button type="button" className="btn btn--sm" onClick={() => setEditLibelle(null)}>
            Annuler
          </button>
        </form>
      )}

      <div className="rule-op-summary">
        <span className="rule-op-summary__tag">en clair</span>
        <span>
          Classe les membres de « {char.base_dimension} » selon une liste propre. {champsClause}
        </span>
      </div>

      {/* Champs (N2) */}
      <div className="rule-section">
        <h3 className="rule-section__title">Champs</h3>
        <p className="rule-section__hint">
          Colonnes optionnelles portées par chaque valeur (ex. le compte de destination d'une
          élimination). Chaque champ tire ses valeurs d'une liste de valeurs ou d'une dimension.
        </p>
        <div className="table-wrap">
          <table className="grid">
            <thead>
              <tr>
                <th>Nom</th>
                <th>Libellé</th>
                <th>Valeurs depuis</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {char.attributes.length === 0 && (
                <tr>
                  <td className="grid__empty" colSpan={4}>
                    Aucun champ.
                  </td>
                </tr>
              )}
              {char.attributes.map((a) => (
                <tr key={a.name}>
                  <td>
                    <strong>{a.name}</strong>
                  </td>
                  <td>{a.libelle}</td>
                  <td>
                    {a.target_dimension}
                    <span className="attr-src"> {isList(a.target_dimension) ? '(liste)' : '(dimension)'}</span>
                  </td>
                  <td>
                    <button
                      type="button"
                      className="btn btn--sm btn--danger"
                      onClick={() => void deleteField(a.name)}
                    >
                      Suppr.
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <form className="form-grid" onSubmit={submitField} style={{ marginTop: 12 }}>
          <label className="field">
            <span>Nom du champ •</span>
            <input
              value={newField.name}
              onChange={(e) => setNewField({ ...newField, name: e.target.value })}
              placeholder="compte_destination"
              required
            />
          </label>
          <label className="field">
            <span>Libellé</span>
            <input
              value={newField.libelle}
              onChange={(e) => setNewField({ ...newField, libelle: e.target.value })}
            />
          </label>
          <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
            <legend style={{ fontSize: '0.78rem', marginBottom: 4, color: 'var(--text-muted)' }}>
              Valeurs depuis
            </legend>
            <label style={{ marginRight: 12, cursor: 'pointer', fontSize: 13 }}>
              <input
                type="radio"
                name="fieldKind"
                checked={newField.kind === 'list'}
                onChange={() => setNewField({ ...newField, kind: 'list', target: '' })}
              />{' '}
              une liste de valeurs
            </label>
            <label style={{ cursor: 'pointer', fontSize: 13 }}>
              <input
                type="radio"
                name="fieldKind"
                checked={newField.kind === 'dim'}
                onChange={() => setNewField({ ...newField, kind: 'dim', target: '' })}
              />{' '}
              une dimension
            </label>
          </fieldset>
          <label className="field">
            <span>Cible •</span>
            <select
              value={newField.target}
              onChange={(e) => setNewField({ ...newField, target: e.target.value })}
              required
            >
              <option value="" disabled>
                — choisir —
              </option>
              {newField.kind === 'list'
                ? lists.map((l) => (
                    <option key={l.code} value={l.code}>
                      {l.code}
                    </option>
                  ))
                : dims.map((d) => (
                    <option key={d.dim} value={d.dim}>
                      {d.dim}
                    </option>
                  ))}
            </select>
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn--primary">
              Ajouter le champ
            </button>
          </div>
        </form>

        {newField.kind === 'list' && lists.length === 0 && (
          <p className="rule-section__hint" style={{ marginTop: 4 }}>
            Aucune liste de valeurs : créez-en une dans le sous-onglet « Listes de valeurs ».
          </p>
        )}
      </div>

      {/* Valeurs */}
      <div className="rule-section">
        <h3 className="rule-section__title">Valeurs</h3>
        <p className="rule-section__hint">Les classes possibles de cette caractéristique.</p>
        <div className="table-wrap">
          <table className="grid">
            <thead>
              <tr>
                <th>Code</th>
                <th>Libellé</th>
                {char.attributes.map((a) => (
                  <th key={a.name}>{a.libelle || a.name}</th>
                ))}
                <th></th>
              </tr>
            </thead>
            <tbody>
              {values.length === 0 && (
                <tr>
                  <td className="grid__empty" colSpan={3 + char.attributes.length}>
                    Aucune valeur.
                  </td>
                </tr>
              )}
              {values.map((row) => {
                const code = String(row['code'] ?? '');
                return (
                  <tr key={code}>
                    <td>
                      <strong>{code}</strong>
                    </td>
                    <td>{String(row['libelle'] ?? '')}</td>
                    {char.attributes.map((a) => (
                      <td key={a.name}>
                        {row[a.name] == null ? <span className="muted">—</span> : String(row[a.name])}
                      </td>
                    ))}
                    <td>
                      <button
                        type="button"
                        className="btn btn--sm btn--danger"
                        onClick={() => void deleteValue(code)}
                      >
                        Suppr.
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>

        <form className="form-grid" onSubmit={submitValue} style={{ marginTop: 12 }}>
          <label className="field">
            <span>Code •</span>
            <input
              value={newValue['code'] ?? ''}
              onChange={(e) => setNewValue({ ...newValue, code: e.target.value })}
              placeholder="VENTES_IC"
              required
            />
          </label>
          <label className="field">
            <span>Libellé</span>
            <input
              value={newValue['libelle'] ?? ''}
              onChange={(e) => setNewValue({ ...newValue, libelle: e.target.value })}
            />
          </label>
          {char.attributes.map((a) => (
            <label key={a.name} className="field">
              <span>{a.libelle || a.name}</span>
              <select
                value={newValue[a.name] ?? ''}
                onChange={(e) => setNewValue({ ...newValue, [a.name]: e.target.value })}
              >
                <option value="">—</option>
                {(optCache[a.target_dimension] ?? []).map((o) => (
                  <option key={o.v} value={o.v}>
                    {o.lbl}
                  </option>
                ))}
              </select>
            </label>
          ))}
          <div className="form-actions">
            <button type="submit" className="btn btn--primary">
              Créer la valeur
            </button>
          </div>
        </form>
      </div>

      {/* Affectation */}
      <div className="rule-section">
        <h3 className="rule-section__title">Affectation</h3>
        <p className="rule-section__hint">
          Quel membre de « {char.base_dimension} » reçoit quelle classe. Valeur vide = déclasser.
        </p>
        <form className="rule-condition" onSubmit={submitAssign}>
          <label className="field field--grow">
            <span>Membre •</span>
            <select
              value={assignForm.member}
              onChange={(e) => setAssignForm({ ...assignForm, member: e.target.value })}
              required
            >
              <option value="" disabled>
                — choisir —
              </option>
              {baseOptions.map((o) => (
                <option key={o.v} value={o.v}>
                  {o.lbl}
                </option>
              ))}
            </select>
          </label>
          <label className="field field--grow">
            <span>Valeur</span>
            <select
              value={assignForm.value}
              onChange={(e) => setAssignForm({ ...assignForm, value: e.target.value })}
            >
              <option value="">— (déclasser)</option>
              {values.map((row) => {
                const code = String(row['code'] ?? '');
                return (
                  <option key={code} value={code}>
                    {code}
                  </option>
                );
              })}
            </select>
          </label>
          <button type="submit" className="btn btn--primary">
            Affecter
          </button>
        </form>
      </div>
    </>
  );
}

// ─── Détail d'un emprunt : affectation membre → valeur empruntée ───────────────

function ReferenceDetail({
  reference,
  loadOptions,
  onNotice,
  notifyErr,
}: {
  reference: CustomReference;
  loadOptions: (target: string) => Promise<Opt[]>;
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
}) {
  const [memberOpts, setMemberOpts] = useState<Opt[]>([]);
  const [valueOpts, setValueOpts] = useState<Opt[]>([]);
  const [form, setForm] = useState({ member: '', value: '' });

  useEffect(() => {
    setForm({ member: '', value: '' });
    void (async () => {
      setMemberOpts(await loadOptions(reference.host_dimension));
      setValueOpts(await loadOptions(reference.target_dimension));
    })();
  }, [reference, loadOptions]);

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      await api.customReferences.assign(reference.host_dimension, reference.column, {
        member: form.member,
        value: form.value === '' ? null : form.value,
      });
      onNotice({
        kind: 'success',
        text:
          form.value === ''
            ? `« ${form.member} » : ${reference.column} vidé.`
            : `« ${form.member} » : ${reference.column} = « ${form.value} ».`,
      });
      setForm({ member: '', value: '' });
    } catch (err) {
      notifyErr(err);
    }
  }

  const hier = reference.target_dimension === reference.host_dimension;

  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
        <h2 className="page__title" style={{ fontSize: '1.05rem' }}>
          {reference.column}
        </h2>
        <span className={`attr-badge ${reference.native ? 'attr-badge--native' : 'attr-badge--ref'}`}>
          {reference.native ? 'emprunt natif' : 'emprunt'}
        </span>
      </div>

      <div className="rule-op-summary">
        <span className="rule-op-summary__tag">en clair</span>
        <span>
          Sur « {reference.host_dimension} », emprunte les valeurs de «{' '}
          {reference.target_dimension} »{hier ? ' (hiérarchie sur elle-même)' : ''}. Un emprunt n'a
          pas de champs.
        </span>
      </div>

      <div className="rule-section">
        <h3 className="rule-section__title">Affectation</h3>
        {reference.native ? (
          <p className="rule-section__hint" style={{ marginBottom: 0 }}>
            Référence native (auto-alimentée par les données importées) — non modifiable ici.
          </p>
        ) : (
          <>
            <p className="rule-section__hint">
              Affectez une valeur empruntée à un membre de « {reference.host_dimension} ». Valeur
              vide = retirer.
            </p>
            <form className="rule-condition" onSubmit={submit}>
              <label className="field field--grow">
                <span>Membre •</span>
                <select
                  value={form.member}
                  onChange={(e) => setForm({ ...form, member: e.target.value })}
                  required
                >
                  <option value="" disabled>
                    — choisir —
                  </option>
                  {memberOpts.map((o) => (
                    <option key={o.v} value={o.v}>
                      {o.lbl}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field field--grow">
                <span>Valeur</span>
                <select value={form.value} onChange={(e) => setForm({ ...form, value: e.target.value })}>
                  <option value="">— (retirer)</option>
                  {valueOpts.map((o) => (
                    <option key={o.v} value={o.v}>
                      {o.lbl}
                    </option>
                  ))}
                </select>
              </label>
              <button type="submit" className="btn btn--primary">
                Affecter
              </button>
            </form>
          </>
        )}
      </div>
    </>
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// Sous-onglet « Listes de valeurs » (nomenclatures réutilisables)
// ═══════════════════════════════════════════════════════════════════════════

function ListesTab({
  lists,
  onNotice,
  notifyErr,
  reloadAll,
}: {
  lists: ValueList[];
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  reloadAll: () => Promise<void>;
}) {
  const [newList, setNewList] = useState({ code: '', libelle: '' });
  const [selected, setSelected] = useState<string | null>(null);
  const [editLibelle, setEditLibelle] = useState<string | null>(null);
  const [values, setValues] = useState<Row[]>([]);
  const [newValue, setNewValue] = useState({ code: '', libelle: '' });

  const selectedList = useMemo(
    () => lists.find((l) => l.code === selected) ?? null,
    [lists, selected],
  );

  const reloadValues = useCallback(
    async (code: string) => {
      try {
        setValues((await api.valueLists.listValues(code)) as Row[]);
      } catch (err) {
        notifyErr(err);
      }
    },
    [notifyErr],
  );

  useEffect(() => {
    setEditLibelle(null);
    if (selectedList) {
      setNewValue({ code: '', libelle: '' });
      void reloadValues(selectedList.code);
    } else {
      setValues([]);
    }
  }, [selectedList, reloadValues]);

  async function saveListLibelle(e: FormEvent) {
    e.preventDefault();
    if (!selectedList || editLibelle === null) return;
    try {
      await api.valueLists.update(selectedList.code, { libelle: editLibelle });
      onNotice({ kind: 'success', text: 'Libellé mis à jour.' });
      setEditLibelle(null);
      await reloadAll();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitNewList(e: FormEvent) {
    e.preventDefault();
    try {
      await api.valueLists.create(newList);
      onNotice({ kind: 'success', text: `Liste « ${newList.code} » créée.` });
      const created = newList.code;
      setNewList({ code: '', libelle: '' });
      await reloadAll();
      setSelected(created);
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteList(code: string) {
    if (!window.confirm(`Supprimer la liste « ${code} » et ses valeurs ?`)) return;
    try {
      await api.valueLists.remove(code);
      if (selected === code) setSelected(null);
      onNotice({ kind: 'success', text: 'Liste supprimée.' });
      await reloadAll();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitNewValue(e: FormEvent) {
    e.preventDefault();
    if (!selectedList) return;
    try {
      await api.valueLists.createValue(selectedList.code, {
        code: newValue.code,
        libelle: newValue.libelle || undefined,
      });
      onNotice({ kind: 'success', text: 'Valeur ajoutée.' });
      setNewValue({ code: '', libelle: '' });
      await reloadValues(selectedList.code);
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteValue(code: string) {
    if (!selectedList) return;
    try {
      await api.valueLists.removeValue(selectedList.code, code);
      await reloadValues(selectedList.code);
    } catch (err) {
      notifyErr(err);
    }
  }

  return (
    <div>
      <p className="page__meta">
        Nomenclatures <strong>code / libellé</strong> autonomes et réutilisables, qu'un champ de
        caractéristique peut viser. Ce ne sont pas des dimensions (aucun axe d'écriture).
      </p>

      <div className="attr-layout">
        {/* Listes + création */}
        <div>
          <div className="attr-list">
            {lists.length === 0 && <div className="attr-empty">Aucune liste.</div>}
            {lists.map((l) => {
              const active = l.code === selected;
              return (
                <div
                  key={l.code}
                  className={`attr-item ${active ? 'is-selected' : ''}`}
                  onClick={() => setSelected(l.code)}
                >
                  <div className="attr-item__head">
                    <span className="attr-item__code">{l.code}</span>
                    <button
                      type="button"
                      className="attr-item__del"
                      aria-label={`Supprimer ${l.code}`}
                      title="Supprimer"
                      onClick={(e) => {
                        e.stopPropagation();
                        void deleteList(l.code);
                      }}
                    >
                      ✕
                    </button>
                  </div>
                  {l.libelle && <div className="attr-item__recap">{l.libelle}</div>}
                </div>
              );
            })}
          </div>

          <form className="form-grid" onSubmit={submitNewList} style={{ marginTop: 12 }}>
            <label className="field">
              <span>Code •</span>
              <input
                value={newList.code}
                onChange={(e) => setNewList({ ...newList, code: e.target.value })}
                placeholder="incoterm"
                required
              />
            </label>
            <label className="field">
              <span>Libellé</span>
              <input
                value={newList.libelle}
                onChange={(e) => setNewList({ ...newList, libelle: e.target.value })}
              />
            </label>
            <div className="form-actions">
              <button type="submit" className="btn btn--primary">
                Créer la liste
              </button>
            </div>
          </form>
        </div>

        {/* Valeurs de la liste sélectionnée */}
        <div>
          {!selectedList ? (
            <div className="rule-section">
              <p className="muted" style={{ margin: 0 }}>
                Sélectionnez une liste pour saisir ses valeurs.
              </p>
            </div>
          ) : (
            <div className="rule-section">
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
                <h3 className="rule-section__title" style={{ margin: 0 }}>
                  {selectedList.code}
                </h3>
                {editLibelle === null ? (
                  <>
                    <span className="muted" style={{ fontSize: '0.875rem' }}>
                      {selectedList.libelle || <em>sans libellé</em>}
                    </span>
                    <button
                      type="button"
                      className="btn btn--sm"
                      onClick={() => setEditLibelle(selectedList.libelle ?? '')}
                    >
                      Modifier le libellé
                    </button>
                  </>
                ) : (
                  <form
                    style={{ display: 'flex', alignItems: 'center', gap: 8, flex: 1 }}
                    onSubmit={saveListLibelle}
                  >
                    <input
                      className="field__input"
                      value={editLibelle}
                      onChange={(e) => setEditLibelle(e.target.value)}
                      autoFocus
                      style={{ flex: 1 }}
                    />
                    <button type="submit" className="btn btn--sm btn--primary">
                      Enregistrer
                    </button>
                    <button
                      type="button"
                      className="btn btn--sm"
                      onClick={() => setEditLibelle(null)}
                    >
                      Annuler
                    </button>
                  </form>
                )}
              </div>
              <h3 className="rule-section__title">Valeurs</h3>
              <div className="table-wrap">
                <table className="grid">
                  <thead>
                    <tr>
                      <th>Code</th>
                      <th>Libellé</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {values.length === 0 && (
                      <tr>
                        <td className="grid__empty" colSpan={3}>
                          Aucune valeur.
                        </td>
                      </tr>
                    )}
                    {values.map((row) => {
                      const code = String(row['code'] ?? '');
                      return (
                        <tr key={code}>
                          <td>
                            <strong>{code}</strong>
                          </td>
                          <td>{String(row['libelle'] ?? '')}</td>
                          <td>
                            <button
                              type="button"
                              className="btn btn--sm btn--danger"
                              onClick={() => void deleteValue(code)}
                            >
                              Suppr.
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>

              <form className="form-grid" onSubmit={submitNewValue} style={{ marginTop: 12 }}>
                <label className="field">
                  <span>Code •</span>
                  <input
                    value={newValue.code}
                    onChange={(e) => setNewValue({ ...newValue, code: e.target.value })}
                    placeholder="FOB"
                    required
                  />
                </label>
                <label className="field">
                  <span>Libellé</span>
                  <input
                    value={newValue.libelle}
                    onChange={(e) => setNewValue({ ...newValue, libelle: e.target.value })}
                  />
                </label>
                <div className="form-actions">
                  <button type="submit" className="btn btn--primary">
                    Ajouter la valeur
                  </button>
                </div>
              </form>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
