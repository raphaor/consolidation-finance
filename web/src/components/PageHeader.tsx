// En-tête de page standard : titre (h2) + indication facultative.
// Uniformise le markup `.page__header` recopié dans chaque page (et corrige
// l'usage divergent de <h1> au profit de <h2>).

import type { ReactNode } from 'react';

export function PageHeader({ title, hint }: { title: string; hint?: ReactNode }) {
  return (
    <div className="page__header">
      <h2>{title}</h2>
      {hint && <p className="page__hint">{hint}</p>}
    </div>
  );
}
