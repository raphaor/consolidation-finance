// Wrapper minimal autour de MasterDetailEditor : un titre + l'éditeur configuré.
// Mutualise les pages « liste d'objets → grille » qui ne diffèrent que par leur
// config (Schémas de flux, Jeux de périmètre, Jeux de taux).

import {
  MasterDetailEditor,
  type MasterDetailConfig,
} from '../components/MasterDetailEditor';

interface Props {
  title: string;
  config: MasterDetailConfig;
}

export function MasterDetailPage({ title, config }: Props) {
  return (
    <section className="page">
      <div className="page__header">
        <h1 className="page__title">{title}</h1>
      </div>
      <MasterDetailEditor config={config} />
    </section>
  );
}
