// Page « Caractéristiques & listes de valeurs ».
//
// Un seul assistant guidé pour enrichir une dimension à l'exécution, selon
// l'arbre :
//
//   Caractéristique sur <dimension de base>, nommée <code>
//   └─ ses valeurs viennent de…
//      ├─ (A) une liste que je saisis        → +optionnel : des champs
//      │      └─ chaque champ tire ses valeurs d'une liste de valeurs ou d'une dimension
//      └─ (B) une dimension existante (emprunt)
//
// Sous le capot, deux mécaniques moteur déjà en place :
//   - (A) = caractéristique N1 (`car_<code>`) + attributs N2  (characteristics.rs)
//   - (B) = référence directe (patron B)                      (custom_references.rs)
// La brique « champ = liste de valeurs » s'appuie sur les listes réutilisables
// (`lst_<code>`, value_lists.rs), gérées dans la 2ᵉ section de la page.

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

// Dimension classable / ciblable : dérivée du graphe de références (toute
// dimension d'écriture ayant une master data, c.-à-d. une référence depuis
// `stg_entry`). `table` est le nom d'API master data, `key` sa colonne clé.
interface DimInfo {
  dim: string;
  table: string;
  key: string;
}

// Sélection courante : soit une caractéristique (branche A), soit une référence
// directe (branche B, identifiée par "host.column").
type Selection = { kind: 'char'; code: string } | { kind: 'ref'; id: string } | null;

export function CaracteristiquesPage() {
  const [chars, setChars] = useState<Characteristic[]>([]);
  const [refs, setRefs] = useState<CustomReference[]>([]);
  const [lists, setLists] = useState<ValueList[]>([]);
  const [dims, setDims] = useState<DimInfo[]>([]);
  const [selected, setSelected] = useState<Selection>(null);
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
      setChars(cs);
      setRefs(rs);
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

  const selectedChar = useMemo(
    () => (selected?.kind === 'char' ? chars.find((c) => c.code === selected.code) ?? null : null),
    [selected, chars],
  );
  const selectedRef = useMemo(
    () =>
      selected?.kind === 'ref'
        ? refs.find((r) => `${r.host_dimension}.${r.column}` === selected.id) ?? null
        : null,
    [selected, refs],
  );

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Caractéristiques &amp; listes de valeurs</h1>
        <div className="page__actions">
          <button type="button" className="btn" onClick={() => void loadAll()} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      <div className="page__meta muted">
        Une <strong>caractéristique</strong> enrichit une dimension de base. Ses valeurs viennent
        soit d'une <strong>liste que vous saisissez</strong> (avec, en option, des champs tirés
        d'une liste de valeurs ou d'une dimension), soit d'une <strong>dimension existante</strong>
        {' '}(emprunt). Les <strong>listes de valeurs</strong> (bas de page) sont des nomenclatures
        réutilisables — pas des dimensions.
      </div>

      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        {/* Colonne gauche : liste unifiée + assistant de création */}
        <div style={{ flex: '1 1 320px', minWidth: 320 }}>
          <UnifiedList
            chars={chars}
            refs={refs}
            selected={selected}
            onSelect={setSelected}
            onDeleteChar={async (code) => {
              if (!window.confirm(`Supprimer la caractéristique « ${code} » et ses valeurs ?`)) return;
              try {
                await api.characteristics.remove(code);
                if (selected?.kind === 'char' && selected.code === code) setSelected(null);
                setNotice({ kind: 'success', text: 'Caractéristique supprimée.' });
                await loadAll();
              } catch (err) {
                notifyErr(err);
              }
            }}
            onDeleteRef={async (r) => {
              if (!window.confirm(`Supprimer l'emprunt « ${r.host_dimension}.${r.column} » ?`)) return;
              try {
                await api.customReferences.remove(r.host_dimension, r.column);
                if (selected?.kind === 'ref' && selected.id === `${r.host_dimension}.${r.column}`)
                  setSelected(null);
                setNotice({ kind: 'success', text: 'Emprunt supprimé.' });
                await loadAll();
              } catch (err) {
                notifyErr(err);
              }
            }}
          />

          <CreateWizard
            dims={dims}
            onNotice={setNotice}
            notifyErr={notifyErr}
            onCreated={async (sel) => {
              await loadAll();
              setSelected(sel);
            }}
          />
        </div>

        {/* Colonne droite : détail de la sélection */}
        <div style={{ flex: '2 1 480px', minWidth: 480 }}>
          {selectedChar ? (
            <CharacteristicDetail
              char={selectedChar}
              dims={dims}
              lists={lists}
              loadOptions={loadOptions}
              onNotice={setNotice}
              notifyErr={notifyErr}
              onChanged={loadAll}
            />
          ) : selectedRef ? (
            <ReferenceDetail
              reference={selectedRef}
              loadOptions={loadOptions}
              onNotice={setNotice}
              notifyErr={notifyErr}
            />
          ) : (
            <p className="muted">Sélectionnez une caractéristique pour la détailler.</p>
          )}
        </div>
      </div>

      <hr style={{ margin: '32px 0', border: 0, borderTop: '1px solid var(--border, #e5e7eb)' }} />

      <ValueListsSection lists={lists} onNotice={setNotice} notifyErr={notifyErr} onChanged={loadAll} />
    </section>
  );
}

// ─── Liste unifiée (caractéristiques branche A + emprunts branche B) ───────────

function UnifiedList({
  chars,
  refs,
  selected,
  onSelect,
  onDeleteChar,
  onDeleteRef,
}: {
  chars: Characteristic[];
  refs: CustomReference[];
  selected: Selection;
  onSelect: (s: Selection) => void;
  onDeleteChar: (code: string) => void;
  onDeleteRef: (r: CustomReference) => void;
}) {
  const hl = (active: boolean) => (active ? 'rgba(99, 102, 241, 0.12)' : undefined);
  const empty = chars.length === 0 && refs.length === 0;

  return (
    <>
      <h2 className="page__title" style={{ fontSize: '1rem' }}>
        Caractéristiques
      </h2>
      <div className="table-wrap">
        <table className="grid">
          <thead>
            <tr>
              <th>Code</th>
              <th>Dimension</th>
              <th>Source des valeurs</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {empty && (
              <tr>
                <td className="grid__empty" colSpan={4}>
                  Aucune caractéristique.
                </td>
              </tr>
            )}
            {chars.map((c) => {
              const active = selected?.kind === 'char' && selected.code === c.code;
              return (
                <tr
                  key={`char-${c.code}`}
                  style={{ cursor: 'pointer', background: hl(active) }}
                  onClick={() => onSelect({ kind: 'char', code: c.code })}
                >
                  <td>
                    <strong>{c.code}</strong>
                    <div className="muted">{c.libelle}</div>
                  </td>
                  <td>{c.base_dimension}</td>
                  <td>
                    liste propre
                    {c.attributes.length > 0 && (
                      <span className="muted"> · {c.attributes.length} champ(s)</span>
                    )}
                  </td>
                  <td>
                    <button
                      type="button"
                      className="btn btn--sm btn--danger"
                      onClick={(e) => {
                        e.stopPropagation();
                        onDeleteChar(c.code);
                      }}
                    >
                      Suppr.
                    </button>
                  </td>
                </tr>
              );
            })}
            {refs.map((r) => {
              const id = `${r.host_dimension}.${r.column}`;
              const active = selected?.kind === 'ref' && selected.id === id;
              return (
                <tr
                  key={`ref-${id}`}
                  style={{ cursor: 'pointer', background: hl(active) }}
                  onClick={() => onSelect({ kind: 'ref', id })}
                >
                  <td>
                    <strong>{r.column}</strong>
                  </td>
                  <td>{r.host_dimension}</td>
                  <td>
                    emprunt : {r.target_dimension}
                    {r.target_dimension === r.host_dimension && (
                      <span className="muted"> (hiérarchie)</span>
                    )}
                  </td>
                  <td>
                    <button
                      type="button"
                      className="btn btn--sm btn--danger"
                      onClick={(e) => {
                        e.stopPropagation();
                        onDeleteRef(r);
                      }}
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
    </>
  );
}

// ─── Assistant de création (branche A ou B) ───────────────────────────────────

function CreateWizard({
  dims,
  onNotice,
  notifyErr,
  onCreated,
}: {
  dims: DimInfo[];
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  onCreated: (sel: Selection) => void;
}) {
  const [base, setBase] = useState('');
  const [code, setCode] = useState('');
  const [libelle, setLibelle] = useState('');
  const [source, setSource] = useState<'list' | 'dim'>('list');
  const [target, setTarget] = useState(''); // dimension empruntée (branche B)

  function reset() {
    setBase('');
    setCode('');
    setLibelle('');
    setSource('list');
    setTarget('');
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    try {
      if (source === 'list') {
        // Branche A : caractéristique N1 (liste propre `car_<code>`).
        await api.characteristics.create({ code, libelle, base_dimension: base });
        onNotice({ kind: 'success', text: `Caractéristique « ${code} » créée.` });
        const created = code;
        reset();
        onCreated({ kind: 'char', code: created });
      } else {
        // Branche B : emprunt d'une dimension existante (référence directe).
        await api.customReferences.create({
          host_dimension: base,
          column: code,
          target_dimension: target,
        });
        onNotice({ kind: 'success', text: `Emprunt « ${base}.${code} » créé.` });
        const id = `${base}.${code}`;
        reset();
        onCreated({ kind: 'ref', id });
      }
    } catch (err) {
      notifyErr(err);
    }
  }

  return (
    <form className="form-grid" onSubmit={submit} style={{ marginTop: 16 }}>
      <h3 className="page__title" style={{ fontSize: '0.95rem' }}>
        Nouvelle caractéristique
      </h3>

      <label className="field">
        <span>1. Dimension de base •</span>
        <select value={base} onChange={(e) => setBase(e.target.value)} required>
          <option value="" disabled>
            — choisir —
          </option>
          {dims.map((d) => (
            <option key={d.dim} value={d.dim}>
              {d.dim}
            </option>
          ))}
        </select>
      </label>

      <label className="field">
        <span>2. Code •</span>
        <input value={code} onChange={(e) => setCode(e.target.value)} placeholder="comportement" required />
      </label>
      <label className="field">
        <span>Libellé</span>
        <input value={libelle} onChange={(e) => setLibelle(e.target.value)} />
      </label>

      <fieldset style={{ border: 0, padding: 0, margin: 0 }}>
        <legend style={{ fontSize: '0.85rem', marginBottom: 4 }}>3. D'où viennent les valeurs ?</legend>
        <label style={{ display: 'block', cursor: 'pointer' }}>
          <input
            type="radio"
            name="source"
            checked={source === 'list'}
            onChange={() => setSource('list')}
          />{' '}
          Une liste que je saisis <span className="muted">(+ champs optionnels)</span>
        </label>
        <label style={{ display: 'block', cursor: 'pointer' }}>
          <input
            type="radio"
            name="source"
            checked={source === 'dim'}
            onChange={() => setSource('dim')}
          />{' '}
          Une dimension existante <span className="muted">(emprunt)</span>
        </label>
      </fieldset>

      {source === 'dim' && (
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

      <div className="form-actions">
        <button type="submit" className="btn btn--primary">
          Créer
        </button>
      </div>
    </form>
  );
}

// ─── Détail d'une caractéristique (branche A) : champs, valeurs, affectation ───

function CharacteristicDetail({
  char,
  dims,
  lists,
  loadOptions,
  onNotice,
  notifyErr,
  onChanged,
}: {
  char: Characteristic;
  dims: DimInfo[];
  lists: ValueList[];
  loadOptions: (target: string) => Promise<Opt[]>;
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  onChanged: () => Promise<void>;
}) {
  const [values, setValues] = useState<Row[]>([]);
  const [optCache, setOptCache] = useState<Record<string, Opt[]>>({});
  const [newField, setNewField] = useState({ name: '', libelle: '', kind: 'list' as 'list' | 'dim', target: '' });
  const [newValue, setNewValue] = useState<Record<string, string>>({});
  const [assignForm, setAssignForm] = useState({ member: '', value: '' });

  const reload = useCallback(async () => {
    try {
      const vals = (await api.characteristics.listValues(char.code)) as Row[];
      setValues(sortForDisplay(vals, (r) => String(r['code'] ?? '')));
      // Options : la dimension de base + la cible de chaque champ (dim ou liste).
      const targets = new Set<string>([char.base_dimension, ...char.attributes.map((a) => a.target_dimension)]);
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

  return (
    <>
      <h2 className="page__title" style={{ fontSize: '1rem' }}>
        {char.code} — liste propre
      </h2>

      {/* Champs (N2) */}
      <h3 className="page__title" style={{ fontSize: '0.95rem', marginTop: 8 }}>
        Champs
      </h3>
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
            {char.attributes.map((a) => {
              const list = lists.find((l) => l.code === a.target_dimension);
              return (
                <tr key={a.name}>
                  <td>
                    <strong>{a.name}</strong>
                  </td>
                  <td>{a.libelle}</td>
                  <td>
                    {a.target_dimension}
                    <span className="muted"> {list ? '(liste)' : '(dimension)'}</span>
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
              );
            })}
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
          <legend style={{ fontSize: '0.85rem', marginBottom: 4 }}>Valeurs depuis</legend>
          <label style={{ marginRight: 12, cursor: 'pointer' }}>
            <input
              type="radio"
              name="fieldKind"
              checked={newField.kind === 'list'}
              onChange={() => setNewField({ ...newField, kind: 'list', target: '' })}
            />{' '}
            une liste de valeurs
          </label>
          <label style={{ cursor: 'pointer' }}>
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
        <p className="muted" style={{ marginTop: 4 }}>
          Aucune liste de valeurs : créez-en une dans la section « Listes de valeurs » en bas de page.
        </p>
      )}

      {/* Valeurs */}
      <h3 className="page__title" style={{ fontSize: '0.95rem', marginTop: 24 }}>
        Valeurs
      </h3>
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

      {/* Affectation */}
      <h3 className="page__title" style={{ fontSize: '0.95rem', marginTop: 24 }}>
        Affecter un membre de « {char.base_dimension} »
      </h3>
      <form className="form-grid" onSubmit={submitAssign}>
        <label className="field">
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
        <label className="field">
          <span>Valeur (vide = déclasser)</span>
          <select
            value={assignForm.value}
            onChange={(e) => setAssignForm({ ...assignForm, value: e.target.value })}
          >
            <option value="">—</option>
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
        <div className="form-actions">
          <button type="submit" className="btn btn--primary">
            Affecter
          </button>
        </div>
      </form>
    </>
  );
}

// ─── Détail d'un emprunt (branche B) : affectation membre → valeur empruntée ───

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

  return (
    <>
      <h2 className="page__title" style={{ fontSize: '1rem' }}>
        {reference.host_dimension}.{reference.column} — emprunt de « {reference.target_dimension} »
      </h2>
      <div className="page__meta muted">
        Les valeurs sont celles de la dimension « {reference.target_dimension} » ; un emprunt n'a
        pas de champs. Affectez ci-dessous une valeur à un membre de «{' '}
        {reference.host_dimension} ».
      </div>
      <form className="form-grid" onSubmit={submit} style={{ marginTop: 12 }}>
        <label className="field">
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
        <label className="field">
          <span>Valeur (vide = retirer)</span>
          <select value={form.value} onChange={(e) => setForm({ ...form, value: e.target.value })}>
            <option value="">—</option>
            {valueOpts.map((o) => (
              <option key={o.v} value={o.v}>
                {o.lbl}
              </option>
            ))}
          </select>
        </label>
        <div className="form-actions">
          <button type="submit" className="btn btn--primary">
            Affecter
          </button>
        </div>
      </form>
    </>
  );
}

// ─── Gestionnaire des listes de valeurs (référentiels réutilisables) ───────────

function ValueListsSection({
  lists,
  onNotice,
  notifyErr,
  onChanged,
}: {
  lists: ValueList[];
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
  onChanged: () => Promise<void>;
}) {
  const [newList, setNewList] = useState({ code: '', libelle: '' });
  const [selected, setSelected] = useState<string | null>(null);
  const [values, setValues] = useState<Row[]>([]);
  const [newValue, setNewValue] = useState({ code: '', libelle: '' });

  const selectedList = useMemo(() => lists.find((l) => l.code === selected) ?? null, [lists, selected]);

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
    if (selectedList) {
      setNewValue({ code: '', libelle: '' });
      void reloadValues(selectedList.code);
    } else {
      setValues([]);
    }
  }, [selectedList, reloadValues]);

  async function submitNewList(e: FormEvent) {
    e.preventDefault();
    try {
      await api.valueLists.create(newList);
      onNotice({ kind: 'success', text: `Liste « ${newList.code} » créée.` });
      const created = newList.code;
      setNewList({ code: '', libelle: '' });
      await onChanged();
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
      await onChanged();
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
      <h2 className="page__title" style={{ fontSize: '1.1rem' }}>
        Listes de valeurs
      </h2>
      <div className="page__meta muted">
        Nomenclatures <strong>code/libellé</strong> autonomes et réutilisables, qu'un champ de
        caractéristique peut viser. Ce ne sont pas des dimensions (aucun axe d'écriture).
      </div>

      <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start', flexWrap: 'wrap', marginTop: 12 }}>
        {/* Listes + création */}
        <div style={{ flex: '1 1 280px', minWidth: 280 }}>
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
                {lists.length === 0 && (
                  <tr>
                    <td className="grid__empty" colSpan={3}>
                      Aucune liste.
                    </td>
                  </tr>
                )}
                {lists.map((l) => (
                  <tr
                    key={l.code}
                    style={{
                      cursor: 'pointer',
                      background: l.code === selected ? 'rgba(99, 102, 241, 0.12)' : undefined,
                    }}
                    onClick={() => setSelected(l.code)}
                  >
                    <td>
                      <strong>{l.code}</strong>
                    </td>
                    <td>{l.libelle}</td>
                    <td>
                      <button
                        type="button"
                        className="btn btn--sm btn--danger"
                        onClick={(e) => {
                          e.stopPropagation();
                          void deleteList(l.code);
                        }}
                      >
                        Suppr.
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
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
        <div style={{ flex: '1 1 280px', minWidth: 280 }}>
          {!selectedList ? (
            <p className="muted">Sélectionnez une liste pour saisir ses valeurs.</p>
          ) : (
            <>
              <h3 className="page__title" style={{ fontSize: '0.95rem' }}>
                {selectedList.code} — valeurs
              </h3>
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
            </>
          )}
        </div>
      </div>
    </div>
  );
}
