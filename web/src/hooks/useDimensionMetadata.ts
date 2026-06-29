// Chargement (au montage) du quadruplet de métadonnées de dimensions consommé
// par les éditeurs de conditions : dimensions + caractéristiques N1 + références
// directes (patron B) + attributs natifs. Factorise l'effet recopié dans
// Contrôles, Postes, etc. `dims` est trié pour l'affichage (code - libellé).
//
// Le hook expose sa propre `error` ; une page qui agrège d'autres erreurs peut
// l'afficher en complément (ex. `pageError ?? meta.error`).

import { useEffect, useState } from 'react';
import { api } from '../api';
import { errMsg } from '../utils/errMessage';
import { formatOptionLabel, sortForDisplay } from '../utils/format';
import type {
  Characteristic,
  CustomReference,
  DimensionInfo,
  NativeEnum,
} from '../types';

export interface DimensionMetadata {
  dims: DimensionInfo[];
  characteristics: Characteristic[];
  customRefs: CustomReference[];
  nativeEnums: NativeEnum[];
  error: string | null;
}

export function useDimensionMetadata(): DimensionMetadata {
  const [dims, setDims] = useState<DimensionInfo[]>([]);
  const [characteristics, setCharacteristics] = useState<Characteristic[]>([]);
  const [customRefs, setCustomRefs] = useState<CustomReference[]>([]);
  const [nativeEnums, setNativeEnums] = useState<NativeEnum[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [d, c, r, e] = await Promise.all([
          api.dimensions.list(),
          api.characteristics.list(),
          api.customReferences.list(),
          api.nativeEnums(),
        ]);
        if (cancelled) return;
        setDims(sortForDisplay(d, (x) => formatOptionLabel(x.name, x.label)));
        setCharacteristics(c);
        setCustomRefs(r);
        setNativeEnums(e);
      } catch (err) {
        if (!cancelled) setError(errMsg(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return { dims, characteristics, customRefs, nativeEnums, error };
}
