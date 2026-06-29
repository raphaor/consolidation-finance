// Thème clair/sombre : applique `data-theme` sur <html> et mémorise le choix
// dans localStorage. Le jeu de variables sombre vit dans index.css.

import { useEffect, useState } from 'react';

export type Theme = 'light' | 'dark';

const STORAGE_KEY = 'conso.theme';

function initialTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === 'light' || stored === 'dark') return stored;
  // À défaut de choix mémorisé, on suit la préférence système.
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

export function useTheme(): { theme: Theme; toggle: () => void } {
  const [theme, setTheme] = useState<Theme>(initialTheme);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem(STORAGE_KEY, theme);
  }, [theme]);

  return {
    theme,
    toggle: () => setTheme((t) => (t === 'dark' ? 'light' : 'dark')),
  };
}
