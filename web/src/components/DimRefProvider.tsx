// Provider du contexte `DimRefContext` : expose le mapping dimension -> table
// master data (construit depuis le graphe de references API passe en prop) a
// tous les `useDimValues` descendants. Extrait de `useDimValues` pour satisfaire
// `react-refresh/only-export-components` (un fichier exportant un composant ne
// doit pas exporter hooks/constantes/contexte).

import { type ReactNode } from 'react';
import {
  DIM_TO_TABLE_FALLBACK,
  DimRefContext,
  buildDimToTable,
} from '../hooks/useDimValues';
import type { ReferenceInfo } from '../types';

export function DimRefProvider({
  children,
  references,
}: {
  children: ReactNode;
  references: ReferenceInfo[] | null;
}) {
  const mapping = references ? buildDimToTable(references) : DIM_TO_TABLE_FALLBACK;
  return <DimRefContext.Provider value={mapping}>{children}</DimRefContext.Provider>;
}
