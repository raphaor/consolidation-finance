// Page « Caractéristiques » : définition des regroupements N1 (sur une dimension
// de base), de leurs attributs N2 (typés vers une autre dimension), saisie des
// valeurs, et affectation des membres.
//
// La consommation par les règles de consolidation viendra dans un incrément
// ultérieur (cf. pending-improvements #11).

import { type FormEvent, useCallback, useEffect, useMemo, useState } from 'react';
import { api } from '../api';
import type { Characteristic, CustomReference, MasterTable, ReferenceInfo } from '../types';

type Row = Record<string, unknown>;
type Notice = { kind: 'success' | 'error'; text: string } | null;

// Dimension classable / ciblable : dérivée du graphe de références (toute
// dimension d'écriture ayant une master data, c.-à-d. une référence depuis
// `stg_entry`). `table` est le nom d'API master data, `key` sa colonne clé.
interface DimInfo {
  dim: string;
  table: string;
  key: string;
}

export function CaracteristiquesPage() {
  const [chars, setChars] = useState<Characteristic[]>([]);
  const [dims, setDims] = useState<DimInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [values, setValues] = useState<Row[]>([]);
  const [memberOptions, setMemberOptions] = useState<Record<string, Row[]>>({});
  const [loading, setLoading] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);

  const [newChar, setNewChar] = useState({ code: '', libelle: '', base_dimension: '' });
  const [newAttr, setNewAttr] = useState({ name: '', libelle: '', target_dimension: '' });
  const [newValue, setNewValue] = useState<Record<string, string>>({});
  const [assignForm, setAssignForm] = useState({ member: '', value: '' });

  const selectedChar = useMemo(
    () => chars.find((c) => c.code === selected) ?? null,
    [chars, selected],
  );

  const notifyErr = useCallback(
    (err: unknown) =>
      setNotice({ kind: 'error', text: err instanceof Error ? err.message : 'erreur' }),
    [],
  );

  const dimKey = useCallback(
    (dim: string) => dims.find((d) => d.dim === dim)?.key ?? 'code',
    [dims],
  );

  const loadChars = useCallback(async () => {
    setLoading(true);
    try {
      const [cs, refs] = await Promise.all([api.characteristics.list(), api.references()]);
      setChars(cs);
      const seen = new Set<string>();
      const ds: DimInfo[] = (refs as ReferenceInfo[])
        .filter((r) => r.table === 'stg_entry')
        .filter((r) => (seen.has(r.column) ? false : (seen.add(r.column), true)))
        .map((r) => ({ dim: r.column, table: r.target_table, key: r.target_column }));
      setDims(ds);
    } catch (err) {
      notifyErr(err);
    } finally {
      setLoading(false);
    }
  }, [notifyErr]);

  useEffect(() => {
    void loadChars();
  }, [loadChars]);

  const loadValues = useCallback(
    async (char: Characteristic) => {
      try {
        const vals = await api.characteristics.listValues(char.code);
        setValues(vals as Row[]);
        const needed = new Set<string>([
          char.base_dimension,
          ...char.attributes.map((a) => a.target_dimension),
        ]);
        const opts: Record<string, Row[]> = {};
        await Promise.all(
          [...needed].map(async (dim) => {
            const di = dims.find((d) => d.dim === dim);
            if (!di) return;
            try {
              opts[dim] = (await api.masterData.list(di.table as MasterTable)) as Row[];
            } catch {
              opts[dim] = [];
            }
          }),
        );
        setMemberOptions(opts);
      } catch (err) {
        notifyErr(err);
      }
    },
    [dims, notifyErr],
  );

  useEffect(() => {
    if (selectedChar) {
      setNewValue({});
      setAssignForm({ member: '', value: '' });
      void loadValues(selectedChar);
    } else {
      setValues([]);
      setMemberOptions({});
    }
  }, [selectedChar, loadValues]);

  // ── Mutations ──────────────────────────────────────────────────────────────

  async function submitNewChar(e: FormEvent) {
    e.preventDefault();
    try {
      await api.characteristics.create(newChar);
      setNotice({ kind: 'success', text: `Caractéristique « ${newChar.code} » créée.` });
      setNewChar({ code: '', libelle: '', base_dimension: '' });
      await loadChars();
      setSelected(newChar.code);
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteChar(code: string) {
    if (!window.confirm(`Supprimer la caractéristique « ${code} » et toutes ses valeurs ?`)) return;
    try {
      await api.characteristics.remove(code);
      if (selected === code) setSelected(null);
      setNotice({ kind: 'success', text: 'Caractéristique supprimée.' });
      await loadChars();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitNewAttr(e: FormEvent) {
    e.preventDefault();
    if (!selected) return;
    try {
      await api.characteristics.addAttribute(selected, newAttr);
      setNotice({ kind: 'success', text: `Attribut « ${newAttr.name} » ajouté.` });
      setNewAttr({ name: '', libelle: '', target_dimension: '' });
      await loadChars();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteAttr(name: string) {
    if (!selected) return;
    if (!window.confirm(`Supprimer l'attribut « ${name} » ?`)) return;
    try {
      await api.characteristics.removeAttribute(selected, name);
      setNotice({ kind: 'success', text: 'Attribut supprimé.' });
      await loadChars();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitNewValue(e: FormEvent) {
    e.preventDefault();
    if (!selectedChar) return;
    const payload: Row = { code: newValue['code'] ?? '' };
    if ((newValue['libelle'] ?? '') !== '') payload['libelle'] = newValue['libelle'];
    for (const a of selectedChar.attributes) {
      const v = newValue[a.name] ?? '';
      if (v !== '') payload[a.name] = v;
    }
    try {
      await api.characteristics.createValue(selectedChar.code, payload);
      setNotice({ kind: 'success', text: 'Valeur créée.' });
      setNewValue({});
      await loadValues(selectedChar);
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteValue(valueCode: string) {
    if (!selectedChar) return;
    if (!window.confirm(`Supprimer la valeur « ${valueCode} » ?`)) return;
    try {
      await api.characteristics.removeValue(selectedChar.code, valueCode);
      setNotice({ kind: 'success', text: 'Valeur supprimée.' });
      await loadValues(selectedChar);
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitAssign(e: FormEvent) {
    e.preventDefault();
    if (!selectedChar) return;
    try {
      await api.characteristics.assign(selectedChar.code, {
        member: assignForm.member,
        value: assignForm.value === '' ? null : assignForm.value,
      });
      setNotice({
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

  // ── Helpers de rendu ─────────────────────────────────────────────────────────

  function optionList(dim: string) {
    const key = dimKey(dim);
    const rows = memberOptions[dim] ?? [];
    return rows.map((r) => {
      const v = String(r[key] ?? '');
      const lbl = String(r['libelle'] ?? v);
      return { v, lbl };
    });
  }

  // ── Rendu ──────────────────────────────────────────────────────────────────

  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">Attributs de dimension</h1>
        <div className="page__actions">
          <button type="button" className="btn" onClick={() => void loadChars()} disabled={loading}>
            {loading ? 'Chargement…' : 'Rafraîchir'}
          </button>
        </div>
      </div>

      <div className="page__meta muted">
        Deux façons d'enrichir une dimension à l'exécution : une <strong>caractéristique</strong>
        {' '}(regroupement N1 avec table de valeurs à part et attributs N2 vers d'autres
        dimensions), ou une <strong>référence directe</strong> (une colonne pointant
        directement vers une dimension, y compris elle-même : hiérarchie de type compte parent).
      </div>

      {notice && <div className={`alert alert--${notice.kind}`}>{notice.text}</div>}

      <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        {/* Colonne gauche : liste + création N1 */}
        <div style={{ flex: '1 1 280px', minWidth: 280 }}>
          <h2 className="page__title" style={{ fontSize: '1rem' }}>
            Caractéristiques
          </h2>
          <div className="table-wrap">
            <table className="grid">
              <thead>
                <tr>
                  <th>Code</th>
                  <th>Dimension de base</th>
                  <th>N2</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {chars.length === 0 && (
                  <tr>
                    <td className="grid__empty" colSpan={4}>
                      Aucune caractéristique.
                    </td>
                  </tr>
                )}
                {chars.map((c) => (
                  <tr
                    key={c.code}
                    style={{
                      cursor: 'pointer',
                      background: c.code === selected ? 'rgba(99, 102, 241, 0.12)' : undefined,
                    }}
                    onClick={() => setSelected(c.code)}
                  >
                    <td>
                      <strong>{c.code}</strong>
                      <div className="muted">{c.libelle}</div>
                    </td>
                    <td>{c.base_dimension}</td>
                    <td>{c.attributes.length}</td>
                    <td>
                      <button
                        type="button"
                        className="btn btn--sm btn--danger"
                        onClick={(e) => {
                          e.stopPropagation();
                          void deleteChar(c.code);
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

          <form className="form-grid" onSubmit={submitNewChar} style={{ marginTop: 16 }}>
            <label className="field">
              <span>Code •</span>
              <input
                value={newChar.code}
                onChange={(e) => setNewChar({ ...newChar, code: e.target.value })}
                placeholder="comportement"
                required
              />
            </label>
            <label className="field">
              <span>Libellé</span>
              <input
                value={newChar.libelle}
                onChange={(e) => setNewChar({ ...newChar, libelle: e.target.value })}
              />
            </label>
            <label className="field">
              <span>Dimension de base •</span>
              <select
                value={newChar.base_dimension}
                onChange={(e) => setNewChar({ ...newChar, base_dimension: e.target.value })}
                required
              >
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
            <div className="form-actions">
              <button type="submit" className="btn btn--primary">
                Créer
              </button>
            </div>
          </form>
        </div>

        {/* Colonne droite : détail de la N1 sélectionnée */}
        <div style={{ flex: '2 1 460px', minWidth: 460 }}>
          {!selectedChar ? (
            <p className="muted">Sélectionnez une caractéristique pour la détailler.</p>
          ) : (
            <>
              <h2 className="page__title" style={{ fontSize: '1rem' }}>
                {selectedChar.code} — attributs (N2)
              </h2>
              <div className="table-wrap">
                <table className="grid">
                  <thead>
                    <tr>
                      <th>Nom</th>
                      <th>Libellé</th>
                      <th>Dimension cible</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {selectedChar.attributes.length === 0 && (
                      <tr>
                        <td className="grid__empty" colSpan={4}>
                          Aucun attribut.
                        </td>
                      </tr>
                    )}
                    {selectedChar.attributes.map((a) => (
                      <tr key={a.name}>
                        <td>
                          <strong>{a.name}</strong>
                        </td>
                        <td>{a.libelle}</td>
                        <td>{a.target_dimension}</td>
                        <td>
                          <button
                            type="button"
                            className="btn btn--sm btn--danger"
                            onClick={() => void deleteAttr(a.name)}
                          >
                            Suppr.
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              <form className="form-grid" onSubmit={submitNewAttr} style={{ marginTop: 12 }}>
                <label className="field">
                  <span>Nom de l'attribut •</span>
                  <input
                    value={newAttr.name}
                    onChange={(e) => setNewAttr({ ...newAttr, name: e.target.value })}
                    placeholder="compte_destination"
                    required
                  />
                </label>
                <label className="field">
                  <span>Libellé</span>
                  <input
                    value={newAttr.libelle}
                    onChange={(e) => setNewAttr({ ...newAttr, libelle: e.target.value })}
                  />
                </label>
                <label className="field">
                  <span>Dimension cible •</span>
                  <select
                    value={newAttr.target_dimension}
                    onChange={(e) => setNewAttr({ ...newAttr, target_dimension: e.target.value })}
                    required
                  >
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
                <div className="form-actions">
                  <button type="submit" className="btn btn--primary">
                    Ajouter l'attribut
                  </button>
                </div>
              </form>

              {/* Valeurs N1 */}
              <h2 className="page__title" style={{ fontSize: '1rem', marginTop: 24 }}>
                Valeurs
              </h2>
              <div className="table-wrap">
                <table className="grid">
                  <thead>
                    <tr>
                      <th>Code</th>
                      <th>Libellé</th>
                      {selectedChar.attributes.map((a) => (
                        <th key={a.name}>{a.libelle || a.name}</th>
                      ))}
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {values.length === 0 && (
                      <tr>
                        <td className="grid__empty" colSpan={3 + selectedChar.attributes.length}>
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
                          {selectedChar.attributes.map((a) => (
                            <td key={a.name}>
                              {row[a.name] == null ? (
                                <span className="muted">—</span>
                              ) : (
                                String(row[a.name])
                              )}
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

              <form className="form-grid" onSubmit={submitNewValue} style={{ marginTop: 12 }}>
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
                {selectedChar.attributes.map((a) => (
                  <label key={a.name} className="field">
                    <span>{a.libelle || a.name}</span>
                    <select
                      value={newValue[a.name] ?? ''}
                      onChange={(e) => setNewValue({ ...newValue, [a.name]: e.target.value })}
                    >
                      <option value="">—</option>
                      {optionList(a.target_dimension).map((o) => (
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

              {/* Affectation d'un membre */}
              <h2 className="page__title" style={{ fontSize: '1rem', marginTop: 24 }}>
                Affecter un membre de « {selectedChar.base_dimension} »
              </h2>
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
                    {optionList(selectedChar.base_dimension).map((o) => (
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
          )}
        </div>
      </div>

      <hr style={{ margin: '32px 0', border: 0, borderTop: '1px solid var(--border, #e5e7eb)' }} />

      <DirectReferencesSection dims={dims} onNotice={setNotice} notifyErr={notifyErr} />
    </section>
  );
}

// ─── Références directes (patron B) ───────────────────────────────────────────
// Une colonne sur la master data d'une dimension hôte, pointant directement vers
// une dimension cible (y compris elle-même). Sans table intermédiaire.

function DirectReferencesSection({
  dims,
  onNotice,
  notifyErr,
}: {
  dims: DimInfo[];
  onNotice: (n: Notice) => void;
  notifyErr: (err: unknown) => void;
}) {
  const [refs, setRefs] = useState<CustomReference[]>([]);
  const [newRef, setNewRef] = useState({ host_dimension: '', column: '', target_dimension: '' });
  const [assignForm, setAssignForm] = useState({ ref: '', member: '', value: '' });
  const [memberRows, setMemberRows] = useState<Record<string, unknown>[]>([]);
  const [valueRows, setValueRows] = useState<Record<string, unknown>[]>([]);

  const dimKey = useCallback(
    (dim: string) => dims.find((d) => d.dim === dim)?.key ?? 'code',
    [dims],
  );

  const loadRefs = useCallback(async () => {
    try {
      setRefs(await api.customReferences.list());
    } catch (err) {
      notifyErr(err);
    }
  }, [notifyErr]);

  useEffect(() => {
    void loadRefs();
  }, [loadRefs]);

  // La référence sélectionnée pour l'affectation est identifiée par "host.column".
  const selectedRef = useMemo(
    () => refs.find((r) => `${r.host_dimension}.${r.column}` === assignForm.ref) ?? null,
    [refs, assignForm.ref],
  );

  // Charge les options (membres = dim hôte, valeurs = dim cible) à la sélection.
  useEffect(() => {
    if (!selectedRef) {
      setMemberRows([]);
      setValueRows([]);
      return;
    }
    const hostDi = dims.find((d) => d.dim === selectedRef.host_dimension);
    const targetDi = dims.find((d) => d.dim === selectedRef.target_dimension);
    void (async () => {
      try {
        setMemberRows(
          hostDi ? ((await api.masterData.list(hostDi.table as MasterTable)) as Record<string, unknown>[]) : [],
        );
        setValueRows(
          targetDi
            ? ((await api.masterData.list(targetDi.table as MasterTable)) as Record<string, unknown>[])
            : [],
        );
      } catch (err) {
        notifyErr(err);
      }
    })();
  }, [selectedRef, dims, notifyErr]);

  async function submitNewRef(e: FormEvent) {
    e.preventDefault();
    try {
      await api.customReferences.create(newRef);
      onNotice({ kind: 'success', text: `Référence « ${newRef.host_dimension}.${newRef.column} » créée.` });
      setNewRef({ host_dimension: '', column: '', target_dimension: '' });
      await loadRefs();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function deleteRef(r: CustomReference) {
    if (!window.confirm(`Supprimer la référence « ${r.host_dimension}.${r.column} » ?`)) return;
    try {
      await api.customReferences.remove(r.host_dimension, r.column);
      onNotice({ kind: 'success', text: 'Référence supprimée.' });
      await loadRefs();
    } catch (err) {
      notifyErr(err);
    }
  }

  async function submitAssign(e: FormEvent) {
    e.preventDefault();
    if (!selectedRef) return;
    try {
      await api.customReferences.assign(selectedRef.host_dimension, selectedRef.column, {
        member: assignForm.member,
        value: assignForm.value === '' ? null : assignForm.value,
      });
      onNotice({
        kind: 'success',
        text:
          assignForm.value === ''
            ? `« ${assignForm.member} » : ${selectedRef.column} vidé.`
            : `« ${assignForm.member} » : ${selectedRef.column} = « ${assignForm.value} ».`,
      });
      setAssignForm({ ...assignForm, member: '', value: '' });
    } catch (err) {
      notifyErr(err);
    }
  }

  function rowOptions(rows: Record<string, unknown>[], dim: string) {
    const key = dimKey(dim);
    return rows.map((r) => {
      const v = String(r[key] ?? '');
      return { v, lbl: String(r['libelle'] ?? v) };
    });
  }

  return (
    <div>
      <h2 className="page__title" style={{ fontSize: '1.1rem' }}>
        Références directes
      </h2>
      <div className="page__meta muted">
        Une colonne ajoutée à une dimension, pointant directement vers une autre dimension
        (ou vers elle-même pour une hiérarchie, comme un compte parent). Pas de table de
        valeurs intermédiaire.
      </div>

      <div className="table-wrap" style={{ marginTop: 12 }}>
        <table className="grid">
          <thead>
            <tr>
              <th>Dimension hôte</th>
              <th>Colonne</th>
              <th>Dimension cible</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {refs.length === 0 && (
              <tr>
                <td className="grid__empty" colSpan={4}>
                  Aucune référence directe.
                </td>
              </tr>
            )}
            {refs.map((r) => (
              <tr key={`${r.host_dimension}.${r.column}`}>
                <td>{r.host_dimension}</td>
                <td>
                  <strong>{r.column}</strong>
                </td>
                <td>
                  {r.target_dimension}
                  {r.target_dimension === r.host_dimension && (
                    <span className="muted"> (hiérarchie)</span>
                  )}
                </td>
                <td>
                  <button
                    type="button"
                    className="btn btn--sm btn--danger"
                    onClick={() => void deleteRef(r)}
                  >
                    Suppr.
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div style={{ display: 'flex', gap: 24, alignItems: 'flex-start', flexWrap: 'wrap', marginTop: 16 }}>
        {/* Création */}
        <form className="form-grid" onSubmit={submitNewRef} style={{ flex: '1 1 280px', minWidth: 280 }}>
          <label className="field">
            <span>Dimension hôte •</span>
            <select
              value={newRef.host_dimension}
              onChange={(e) => setNewRef({ ...newRef, host_dimension: e.target.value })}
              required
            >
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
            <span>Nom de la colonne •</span>
            <input
              value={newRef.column}
              onChange={(e) => setNewRef({ ...newRef, column: e.target.value })}
              placeholder="compte_parent"
              required
            />
          </label>
          <label className="field">
            <span>Dimension cible •</span>
            <select
              value={newRef.target_dimension}
              onChange={(e) => setNewRef({ ...newRef, target_dimension: e.target.value })}
              required
            >
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
          <div className="form-actions">
            <button type="submit" className="btn btn--primary">
              Créer la référence
            </button>
          </div>
        </form>

        {/* Affectation */}
        <form className="form-grid" onSubmit={submitAssign} style={{ flex: '1 1 280px', minWidth: 280 }}>
          <label className="field">
            <span>Référence •</span>
            <select
              value={assignForm.ref}
              onChange={(e) => setAssignForm({ ref: e.target.value, member: '', value: '' })}
              required
            >
              <option value="" disabled>
                — choisir —
              </option>
              {refs.map((r) => {
                const id = `${r.host_dimension}.${r.column}`;
                return (
                  <option key={id} value={id}>
                    {id}
                  </option>
                );
              })}
            </select>
          </label>
          <label className="field">
            <span>Membre •</span>
            <select
              value={assignForm.member}
              onChange={(e) => setAssignForm({ ...assignForm, member: e.target.value })}
              disabled={!selectedRef}
              required
            >
              <option value="" disabled>
                — choisir —
              </option>
              {selectedRef &&
                rowOptions(memberRows, selectedRef.host_dimension).map((o) => (
                  <option key={o.v} value={o.v}>
                    {o.lbl}
                  </option>
                ))}
            </select>
          </label>
          <label className="field">
            <span>Valeur (vide = retirer)</span>
            <select
              value={assignForm.value}
              onChange={(e) => setAssignForm({ ...assignForm, value: e.target.value })}
              disabled={!selectedRef}
            >
              <option value="">—</option>
              {selectedRef &&
                rowOptions(valueRows, selectedRef.target_dimension).map((o) => (
                  <option key={o.v} value={o.v}>
                    {o.lbl}
                  </option>
                ))}
            </select>
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn--primary" disabled={!selectedRef}>
              Affecter
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
