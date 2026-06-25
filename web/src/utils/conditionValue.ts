// Helpers de (dé)sérialisation de la valeur d'une condition de sélection,
// partagés entre l'éditeur de règles et les postes d'indicateurs.
//
// Le moteur attend un **tableau** JSON pour l'opérateur `IN` (cf.
// `rules.rs::push_condition` / `indicators.rs::push_condition`), mais accepte
// aussi une chaîne « a,b » (scindée). On normalise donc en tableau côté UI à
// l'enregistrement (`parseCondVal`), et on reconstitule le texte d'affichage
// via `formatCondVal`.

/// Parse la valeur brute saisie selon l'opérateur :
/// - `IN` : la string est éclatée par virgule → tableau (le moteur attend un
///   tableau JSON pour `push_condition`).
/// - autres : retourne la string brute.
export function parseCondVal(op: string, raw: string): unknown {
  if (op === 'IN') {
    return raw
      .split(',')
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }
  return raw;
}

/// Formate la valeur d'une condition pour l'affichage dans un input texte :
/// - tableau → join par ', ' (réciproque de `parseCondVal`).
/// - autres → toString.
export function formatCondVal(val: unknown): string {
  if (Array.isArray(val)) return val.join(', ');
  return String(val ?? '');
}
