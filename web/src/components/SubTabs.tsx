// Barre de sous-onglets interne à une page (à ne pas confondre avec la
// navigation secondaire de `Layout`). Factorise les boutons `subtab` recodés à
// la main dans plusieurs pages. Générique sur le type d'identifiant d'onglet.

export interface SubTabItem<T extends string> {
  id: T;
  label: string;
}

export function SubTabs<T extends string>({
  items,
  active,
  onChange,
}: {
  items: SubTabItem<T>[];
  active: T;
  onChange: (id: T) => void;
}) {
  return (
    <div className="subtabs">
      {items.map((it) => (
        <button
          key={it.id}
          type="button"
          className={`subtab ${active === it.id ? 'subtab--active' : ''}`}
          onClick={() => onChange(it.id)}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}
