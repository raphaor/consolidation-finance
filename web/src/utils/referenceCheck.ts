// Garde-fou de cohérence : vérifie que les `optionsFrom` déclarés dans
// MASTER_TABLES (config de formulaire front) ne divergent pas du graphe de
// références serveur (`GET /api/meta/references`, source de vérité).
//
// On ne supprime pas `optionsFrom` (il porte aussi le libellé d'affichage choisi
// par usage, que le registre serveur ne connaît pas), mais toute divergence sur
// la cible (table / colonne) est signalée → le serveur reste autoritaire.

import { MASTER_TABLES } from '../types';
import type { ReferenceInfo } from '../types';

/// Retourne la liste des incohérences entre les `optionsFrom` du front et le
/// graphe de références serveur. Vide = tout est aligné.
export function findReferenceDrift(refs: ReferenceInfo[]): string[] {
  const problems: string[] = [];
  for (const def of MASTER_TABLES) {
    for (const col of def.columns) {
      if (col.type !== 'select' || !col.optionsFrom) continue;
      const ref = refs.find(
        (r) => r.table === def.table && r.column === col.name,
      );
      if (!ref) {
        problems.push(
          `${def.table}.${col.name} : optionsFrom sans référence serveur correspondante`,
        );
        continue;
      }
      if (
        ref.target_table !== col.optionsFrom.table ||
        ref.target_column !== col.optionsFrom.value
      ) {
        problems.push(
          `${def.table}.${col.name} : optionsFrom → ${col.optionsFrom.table}.${col.optionsFrom.value}` +
            ` ≠ serveur → ${ref.target_table}.${ref.target_column}`,
        );
      }
    }
  }
  return problems;
}
