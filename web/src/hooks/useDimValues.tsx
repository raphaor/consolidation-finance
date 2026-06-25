// Hook partagé `useDimValues` + contexte du mapping dimension → table master
// data, extraits de `RulesPage` pour être réutilisés par la page Saisie.
//
// Charge et cache les valeurs possibles d'une dimension depuis la master data
// pour alimenter les listes déroulantes d'un formulaire. Le graphe de
// références vient de l'API (`GET /api/meta/references`) ; en mode dégradé, on
// bascule sur le fallback codé en dur.

import {
  createContext,
  useContext,
  useEffect,
  useState,
} from 'react';
import { api } from '../api';
import { compareText, formatOptionLabel } from '../utils/format';
import type {
  CustomReference,
  MasterTable,
  NativeEnum,
  ReferenceInfo,
  SelectionCond,
} from '../types';

// Tri alphabétique des valeurs d'une dimension par libellé affiché (« code -
// libellé »), pour que tous les menus déroulants les présentent dans cet ordre
// plutôt que dans l'ordre de la table source.
function sortDimValues(values: DimValue[]): DimValue[] {
  return [...values].sort((a, b) =>
    compareText(formatOptionLabel(a.code, a.libelle), formatOptionLabel(b.code, b.libelle)),
  );
}

// Mapping dimension → table master data pour les listes déroulantes contextuelles.
export type DimToTable = Record<string, { table: MasterTable; pkCol: string }>;

// Fallback mode-dégradé si `GET /api/meta/references` est injoignable (serveur
// obsolète, réseau en panne) : miroir codé en dur du graphe de références
// serveur (`engine/src/references.rs`). Les dimensions libres (analysis,
// analysis2, custom) sont absentes → saisie texte. `partner` / `share` sont des
// rôles sur la liste des entités.
export const DIM_TO_TABLE_FALLBACK: DimToTable = {
  phase: { table: 'scenario_categories', pkCol: 'code' },
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
// pilotables dans les formulaires. `target_table` est déjà un nom de table
// master data.
export function buildDimToTable(refs: ReferenceInfo[]): DimToTable {
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
export const DimRefContext = createContext<DimToTable>(DIM_TO_TABLE_FALLBACK);

// Cache module-level des valeurs de dimensions (évite les refetchs à chaque
// ouverture de modale). Clé = nom de dimension. Partagé entre toutes les pages
// qui consomment `useDimValues`.
//
// Stocke désormais le couple (code, libelle) pour pouvoir afficher
// « code - libellé » dans les dropdowns.
export interface DimValue {
  code: string;
  libelle: string;
}

const dimValuesCache = new Map<string, DimValue[]>();

/// Hook : charge les valeurs possibles d'une dimension depuis la master data.
/// Renvoie `{ values: DimValue[] }` où chaque item porte le code (clé technique
/// de la dimension) ET le libellé (master data). Renvoie `values = []` si la
/// dimension n'a pas de table associée (saisie libre).
export function useDimValues(dim: string): { values: DimValue[]; loading: boolean } {
  const dimToTable = useContext(DimRefContext);
  const [values, setValues] = useState<DimValue[]>(dimValuesCache.get(dim) ?? []);
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
        const codes: DimValue[] = rows
          .map((r) => {
            const row = r as Record<string, unknown>;
            const code = row[mapping.pkCol];
            const libelle = row['libelle'];
            if (code == null) return null;
            return {
              code: String(code),
              libelle: libelle == null ? '' : String(libelle),
            };
          })
          .filter((v): v is DimValue => v !== null && v.code.length > 0);
        const sorted = sortDimValues(codes);
        if (cancelled) return;
        dimValuesCache.set(dim, sorted);
        setValues(sorted);
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

// ─────────────────────────────────────────────────────────────────────────────
//  Hooks dédiés à la sélection étendue (via N1 / ref patron B).
//  `useCharacteristicValues` est l'équivalent de `useDimValues` pour les
//  valeurs N1 d'une caractéristique (table `car_<code>`). `useSelectionValues`
//  unifie la résolution selon le mode de traversée d'une SelectionCond.
// ─────────────────────────────────────────────────────────────────────────────

// Cache module-level des valeurs N1 d'une caractéristique (clé = code N1).
// Évite les refetchs à chaque ouverture de modale, comme `dimValuesCache`.
const characteristicValuesCache = new Map<string, DimValue[]>();

/// Hook : charge les valeurs N1 d'une caractéristique (lignes de `car_<code>`).
/// Renvoie `{values: DimValue[]}` où chaque item porte le code + le libellé.
/// `charCode = ''` (pas de caractéristique) → `values = []` sans fetch.
export function useCharacteristicValues(
  charCode: string,
): { values: DimValue[]; loading: boolean } {
  const [values, setValues] = useState<DimValue[]>(
    charCode === '' ? [] : characteristicValuesCache.get(charCode) ?? [],
  );
  const [loading, setLoading] = useState(
    charCode !== '' && !characteristicValuesCache.has(charCode),
  );

  useEffect(() => {
    if (charCode === '') {
      setValues([]);
      setLoading(false);
      return;
    }
    if (characteristicValuesCache.has(charCode)) {
      setValues(characteristicValuesCache.get(charCode)!);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void (async () => {
      try {
        const rows = await api.characteristics.listValues(charCode);
        const codes: DimValue[] = rows
          .map((r) => {
            const row = r as Record<string, unknown>;
            const code = row['code'];
            const libelle = row['libelle'];
            if (code == null) return null;
            return {
              code: String(code),
              libelle: libelle == null ? '' : String(libelle),
            };
          })
          .filter((v): v is DimValue => v !== null && v.code.length > 0);
        const sorted = sortDimValues(codes);
        if (cancelled) return;
        characteristicValuesCache.set(charCode, sorted);
        setValues(sorted);
      } catch {
        if (cancelled) return;
        characteristicValuesCache.set(charCode, []);
        setValues([]);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [charCode]);

  return { values, loading };
}

/// Hook unifié : résout les valeurs possibles pour la valeur d'une SelectionCond
/// selon son mode de traversée :
///   - `sel.via`              → valeurs N1 de la caractéristique (`car_<via>`).
///   - `sel.ref`              → master data de la dimension cible de la réf.
///   - `sel.attr`             → valeurs statiques de l'enum natif (ex :
///                              `account.classe` ∈ {bilan, resultat, flux}).
///   - sinon (direct)         → master data de `sel.dim`.
///
/// Appelle `useDimValues` et `useCharacteristicValues` inconditionnellement
/// (Rust React rules of hooks) ; la clé vide `''` désactive le fetch côté hooks
/// sous-jacents. `customReferences` sert à résoudre la dimension cible de `ref`.
/// `nativeEnums` sert à résoudre les valeurs d'un `attr` (catalogue statique).
export function useSelectionValues(
  sel: SelectionCond,
  customReferences: CustomReference[],
  nativeEnums?: NativeEnum[],
): { values: DimValue[]; loading: boolean } {
  const charCode = sel.via ?? '';
  // Pour `ref` : la cible est la dimension cible de la référence (le plus souvent
  // = sel.dim, ex : compte_parent → account). Pour `attr` ou `direct` : sel.dim.
  let effectiveDim = sel.dim;
  if (!sel.via && !sel.attr && sel.ref) {
    const refDef = customReferences.find(
      (r) => r.host_dimension === sel.dim && r.column === sel.ref,
    );
    effectiveDim = refDef?.target_dimension ?? sel.dim;
  }
  // Un seul des deux hooks fetch réellement (l'autre a clé vide ou dim sans
  // mapping). Pour `attr`, les deux hooks sont à clé vide : on lit les valeurs
  // depuis le catalogue `nativeEnums` après les hooks (rules of hooks).
  const dimValues = useDimValues(charCode || sel.attr ? '' : effectiveDim);
  const charValues = useCharacteristicValues(charCode);

  if (sel.attr) {
    const enumDef = nativeEnums?.find(
      (e) => e.host_dimension === sel.dim && e.column === sel.attr,
    );
    return {
      values: enumDef
        ? sortDimValues(enumDef.values.map((v) => ({ code: v, libelle: v })))
        : [],
      loading: false,
    };
  }
  return charCode ? charValues : dimValues;
}
