// État + actions d'un éditeur CRUD « bibliothèque + formulaire » (Coefficients,
// Postes, Indicateurs). Factorise le triplet d'états selected/form/saving et les
// callbacks reload/open/save/remove recopiés à l'identique sur ces pages.
//
// La config (mappings item↔form, appels API, message de confirmation) est
// fournie par la page et lue via une ref, pour garder des callbacks stables sans
// imposer à l'appelant de mémoïser sa config.

import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react';
import { errMsg } from '../utils/errMessage';

export interface CrudConfig<TItem, TForm> {
  list: () => Promise<TItem[]>;
  keyOf: (item: TItem) => string;
  emptyForm: TForm;
  toForm: (item: TItem) => TForm;
  codeOf: (form: TForm) => string;
  create: (form: TForm) => Promise<unknown>;
  update: (code: string, form: TForm) => Promise<unknown>;
  remove: (code: string) => Promise<unknown>;
  confirmRemove: (code: string) => string;
  onError: (msg: string | null) => void;
}

export interface CrudResource<TItem, TForm> {
  items: TItem[];
  selected: string | 'new' | null;
  setSelected: Dispatch<SetStateAction<string | 'new' | null>>;
  form: TForm;
  setForm: Dispatch<SetStateAction<TForm>>;
  saving: boolean;
  reload: () => Promise<void>;
  open: (item: TItem | 'new') => void;
  // Ouvre l'éditeur en création, pré-rempli (ex. « Dupliquer »).
  startDraft: (form: TForm) => void;
  save: () => Promise<void>;
  remove: (code: string) => Promise<void>;
}

export function useCrudResource<TItem, TForm>(
  config: CrudConfig<TItem, TForm>,
): CrudResource<TItem, TForm> {
  // La config est lue via une ref pour garder des callbacks stables. La ref est
  // synchronisée après commit (jamais écrite pendant le rendu) ; les callbacks
  // n'étant invoqués que sur interaction, ils voient toujours la config à jour.
  const cfg = useRef(config);
  useEffect(() => {
    cfg.current = config;
  });

  const [items, setItems] = useState<TItem[]>([]);
  const [selected, setSelected] = useState<string | 'new' | null>(null);
  const [form, setForm] = useState<TForm>(config.emptyForm);
  const [saving, setSaving] = useState(false);

  const reload = useCallback(async () => {
    try {
      setItems(await cfg.current.list());
    } catch (e) {
      cfg.current.onError(errMsg(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const open = useCallback((item: TItem | 'new') => {
    cfg.current.onError(null);
    if (item === 'new') {
      setSelected('new');
      setForm(cfg.current.emptyForm);
    } else {
      setSelected(cfg.current.keyOf(item));
      setForm(cfg.current.toForm(item));
    }
  }, []);

  const startDraft = useCallback((draft: TForm) => {
    cfg.current.onError(null);
    setSelected('new');
    setForm(draft);
  }, []);

  const save = useCallback(async () => {
    cfg.current.onError(null);
    setSaving(true);
    try {
      const code = cfg.current.codeOf(form);
      if (selected === 'new') await cfg.current.create(form);
      else if (selected) await cfg.current.update(selected, form);
      await reload();
      setSelected(code);
    } catch (e) {
      cfg.current.onError(errMsg(e));
    } finally {
      setSaving(false);
    }
  }, [form, selected, reload]);

  const remove = useCallback(
    async (code: string) => {
      if (!confirm(cfg.current.confirmRemove(code))) return;
      cfg.current.onError(null);
      try {
        await cfg.current.remove(code);
        await reload();
        setSelected((cur) => (cur === code ? null : cur));
      } catch (e) {
        cfg.current.onError(errMsg(e));
      }
    },
    [reload],
  );

  return {
    items,
    selected,
    setSelected,
    form,
    setForm,
    saving,
    reload,
    open,
    startDraft,
    save,
    remove,
  };
}
