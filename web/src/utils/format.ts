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

