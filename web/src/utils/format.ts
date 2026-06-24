// Utilitaires de formatage.

const nbFormatter = new Intl.NumberFormat('fr-FR', {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

export function formatAmount(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return nbFormatter.format(value);
}

export function formatInt(value: number): string {
  return new Intl.NumberFormat('fr-FR').format(value);
}

/// Formate le libellé d'une option de dropdown sous la forme « code - libellé ».
/// Si le libellé est vide, égal au code, ou absent : renvoie juste le code.
/// Utilisé partout où l'on expose un choix entre membres d'une dimension pour
/// donner à l'utilisateur le code (clé technique) ET le sens (libellé).
export function formatOptionLabel(code: string, libelle?: string | null): string {
  const c = code ?? '';
  const l = (libelle ?? '').trim();
  if (!l || l === c) return c;
  return `${c} - ${l}`;
}

// Comparateur alphabétique pour l'affichage : locale FR, insensible à la
// casse/accents, tri numérique naturel (« 2 » avant « 10 »). Utilisé pour trier
// les options de menus déroulants et les lignes de tables sur la valeur affichée.
const collator = new Intl.Collator('fr', { sensitivity: 'base', numeric: true });

export function compareText(a: string, b: string): number {
  return collator.compare(a, b);
}

// Renvoie une copie triée de `rows` par le texte affiché (`getText`).
export function sortForDisplay<T>(rows: T[], getText: (row: T) => string): T[] {
  return [...rows].sort((a, b) => compareText(getText(a), getText(b)));
}

