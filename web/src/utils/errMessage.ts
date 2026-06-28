// Normalisation d'une erreur catchée en message affichable.
// Remplace les motifs récurrents `e instanceof Error ? e.message : String(e)`
// et `e instanceof Error ? e.message : 'erreur'`.
//
// `fallback` : message à afficher quand l'objet catché n'est pas une `Error`
// (par défaut `String(e)`, plus diagnostique ; passer un libellé FR pour l'UI).

export function errMsg(e: unknown, fallback?: string): string {
  if (e instanceof Error) return e.message;
  return fallback ?? String(e);
}
