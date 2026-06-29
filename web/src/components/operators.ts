// Constantes partagées pour les sélecteurs de condition (éditeur de règles,
// postes d'indicateurs). Le moteur attend les chaînes brutes ('=', 'IS NULL'…),
// l'UI affiche des symboles compacts via `OP_SYMBOL`. Sont ici (et non dans le
// fichier des composants) pour ne pas mélanger exports de composants et de
// constantes (règle `react-refresh/only-export-components`).

export const OPS = ['=', '!=', '>', '<', '>=', '<=', 'IN', 'IS NULL', 'IS NOT NULL'];

export const NULL_OPS = new Set(['IS NULL', 'IS NOT NULL']);

// Symbole compact affiché par `OpSelect` ; le JSON conserve la chaîne d'origine.
export const OP_SYMBOL: Record<string, string> = {
  '=': '=',
  '!=': '≠',
  '>': '>',
  '<': '<',
  '>=': '≥',
  '<=': '≤',
  IN: '∈',
  'IS NULL': '∅',
  'IS NOT NULL': '≠∅',
};

// Libellé en clair (infobulle + option du sélecteur).
export const OP_LABEL: Record<string, string> = {
  '=': 'égal',
  '!=': 'différent',
  '>': 'supérieur',
  '<': 'inférieur',
  '>=': 'supérieur ou égal',
  '<=': 'inférieur ou égal',
  IN: 'dans la liste',
  'IS NULL': 'est nul',
  'IS NOT NULL': 'non nul',
};
