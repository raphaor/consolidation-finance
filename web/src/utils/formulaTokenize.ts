// Tokeniseur pour le langage de formules (cf. docs/FORMULES.md §2.1).
//
// La grammaire est minuscule : références `[ … ]`, fonctions (identifiant suivi
// de `(`), nombres, identifiants nus, opérateurs/séparateurs, blancs. On produit
// une suite de tokens typés que `FormulaEditor` colore par superposition.
//
// La coloration est purement lexicale (pas d'arbre) : on se contente de
// reconnaître les catégories utiles à la lecture. La validation sémantique
// (référence inconnue, arité, parenthèses) reste côté serveur (preview) — on se
// contente ici de souligner une référence dont le token n'est pas au catalogue.

export type TokenType = 'ref' | 'fn' | 'num' | 'id' | 'op' | 'ws';

export interface Token {
  type: TokenType;
  text: string;
}

// L'ordre des alternatives compte : on tente la référence (crochets), puis le
// nombre, puis la fonction (identifiant + lookahead `(`), puis l'identifiant nu,
// puis les opérateurs (bi-caractères d'abord), puis les blancs. `−` (U+2212) et
// `×` / `÷` sont acceptés en plus des formes ASCII `-` `*` `/`.
const TOKEN_RE = new RegExp(
  [
    '(?<ref>\\[[^\\]]*\\])',
    '(?<num>\\d+(?:\\.\\d+)?)',
    '(?<fn>[A-Za-z_][A-Za-z0-9_]*)(?=\\s*\\()',
    '(?<id>[A-Za-z_][A-Za-z0-9_]*)',
    '(?<op><>|>=|<=|[()+*/,;.=<>*\\-\\u2212\\u00d7\\u00f7])',
    '(?<ws>\\s+)',
  ].join('|'),
  'g',
);

export function tokenizeFormula(src: string): Token[] {
  const tokens: Token[] = [];
  let last = 0;
  for (const m of src.matchAll(TOKEN_RE)) {
    const idx = m.index ?? 0;
    if (idx > last) {
      // Caractère non reconnu (ex : `·`, accent hors crochets) : émis comme op
      // pour ne pas perdre l'alignement avec le textarea.
      tokens.push({ type: 'op', text: src.slice(last, idx) });
    }
    const g = m.groups ?? {};
    if (g.ref !== undefined) tokens.push({ type: 'ref', text: g.ref });
    else if (g.num !== undefined) tokens.push({ type: 'num', text: g.num });
    else if (g.fn !== undefined) tokens.push({ type: 'fn', text: g.fn });
    else if (g.id !== undefined) tokens.push({ type: 'id', text: g.id });
    else if (g.op !== undefined) tokens.push({ type: 'op', text: g.op });
    else if (g.ws !== undefined) tokens.push({ type: 'ws', text: g.ws });
    last = idx + m[0].length;
  }
  if (last < src.length) tokens.push({ type: 'op', text: src.slice(last) });
  return tokens;
}

// Extrait le « contexte crochet » à la position du curseur : renvoie l'indice
// du `[` ouvrant le plus proche et le texte saisi depuis (la requête), s'il n'y
// a ni `]` ni `;` entre les deux (sinon le crochet est fermé ou on a changé
// d'argument → pas d'autocomplétion). Sert à l'insertion `[token]`.
export interface BracketContext {
  bracketIdx: number;
  query: string;
}

export function getBracketContext(src: string, cursor: number): BracketContext | null {
  const before = src.slice(0, cursor);
  const open = before.lastIndexOf('[');
  if (open < 0) return null;
  const close = before.lastIndexOf(']');
  if (close > open) return null; // crochet déjà fermé après le `[` courant
  const semi = before.lastIndexOf(';');
  if (semi > open) return null; // on a changé d'argument
  return { bracketIdx: open, query: before.slice(open + 1) };
}
