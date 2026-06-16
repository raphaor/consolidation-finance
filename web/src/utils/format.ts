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
