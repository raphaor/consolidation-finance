// Hook `useState` adossé à `localStorage` : la valeur survit au démontage du
// composant (changement d'onglet) et au rechargement de la page. Utilisé pour
// persister les filtres des vues Rapports et Écritures.
//
// `key` doit être unique par usage (préfixé par la page) ; la valeur est
// sérialisée en JSON. En cas de stockage indisponible ou de JSON corrompu, on
// retombe silencieusement sur `initial` (le hook reste utilisable sans
// persistance).

import { useCallback, useEffect, useState } from 'react';

export function usePersistentState<T>(
  key: string,
  initial: T,
): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = window.localStorage.getItem(key);
      return raw === null ? initial : (JSON.parse(raw) as T);
    } catch {
      return initial;
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // stockage plein ou indisponible (mode privé) : on ignore, l'état reste
      // en mémoire pour la session courante.
    }
  }, [key, value]);

  const set = useCallback((v: T) => setValue(v), []);
  return [value, set];
}
