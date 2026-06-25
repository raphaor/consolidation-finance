// Éditeur de formules à coloration syntaxique (superposition d'un <pre> coloré
// sous un <textarea> transparent) + autocomplétion au `[`. Composant contrôlé
// partagé par les pages Coefficients et Indicateurs (cf. docs/FORMULES.md §5).
//
// Le catalogue d'opérandes est injecté (`operands`) : il alimente (1) le
// soulignement d'une référence inconnue et (2) la liste d'autocomplétion filtrée
// à mesure qu'on saisit le contenu des crochets.
//
// Le `ref` expose `insert(fragment)` pour que la page puisse insérer une
// fonction ou un opérande à la position du curseur (les boutons « chips »).

import {
  type ChangeEvent,
  type KeyboardEvent,
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';
import { getBracketContext, tokenizeFormula } from '../utils/formulaTokenize';

export interface FormulaOperand {
  token: string;
  label: string;
}

export interface FormulaEditorHandle {
  insert: (fragment: string) => void;
  focus: () => void;
}

interface FormulaEditorProps {
  value: string;
  onChange: (v: string) => void;
  operands: FormulaOperand[];
  readOnly?: boolean;
  rows?: number;
  placeholder?: string;
}

export const FormulaEditor = forwardRef<FormulaEditorHandle, FormulaEditorProps>(
  function FormulaEditor({ value, onChange, operands, readOnly, rows = 4, placeholder }, ref) {
    const taRef = useRef<HTMLTextAreaElement>(null);
    const preRef = useRef<HTMLPreElement>(null);
    const [cursor, setCursor] = useState(value.length);

    const tokens = useMemo(() => tokenizeFormula(value), [value]);
    const knownSet = useMemo(() => new Set(operands.map((o) => o.token)), [operands]);

    // Contexte d'autocomplétion : cursor dans un crochet ouvert.
    const ctx = useMemo(
      () => (readOnly ? null : getBracketContext(value, cursor)),
      [value, cursor, readOnly],
    );
    const ctxKey = ctx ? `${ctx.bracketIdx}:${ctx.query}` : null;

    const matches = useMemo<FormulaOperand[]>(() => {
      if (!ctx) return [];
      const q = ctx.query.trim().toLowerCase();
      const list = operands.filter((o) => {
        if (q === '') return true;
        return o.token.toLowerCase().includes(q) || o.label.toLowerCase().includes(q);
      });
      // Token commençant par la requête en premier, puis ordre original.
      list.sort((a, b) => {
        const ax = a.token.toLowerCase().startsWith(q) ? 0 : 1;
        const bx = b.token.toLowerCase().startsWith(q) ? 0 : 1;
        return ax - bx;
      });
      return list.slice(0, 12);
    }, [ctx, operands]);

    const [activeIdx, setActiveIdx] = useState(0);
    const [suppressedKey, setSuppressedKey] = useState<string | null>(null);

    // Réinitialise la suppression (Escape) et le curseur actif quand le
    // contexte change (autre crochet ou autre requête).
    useEffect(() => {
      setSuppressedKey(null);
      setActiveIdx(0);
    }, [ctxKey]);

    const open = !!ctx && matches.length > 0 && suppressedKey !== ctxKey;

    const syncCursor = () => {
      const ta = taRef.current;
      if (ta) setCursor(ta.selectionStart);
    };

    const accept = (op: FormulaOperand) => {
      if (!ctx) return;
      const ta = taRef.current;
      const start = ctx.bracketIdx;
      const end = ta ? ta.selectionStart : value.length;
      const repl = `[${op.token}]`;
      const next = value.slice(0, start) + repl + value.slice(end);
      onChange(next);
      setSuppressedKey(null);
      requestAnimationFrame(() => {
        if (ta) {
          const pos = start + repl.length;
          ta.focus();
          ta.setSelectionRange(pos, pos);
          setCursor(pos);
        }
      });
    };

    const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (!open || matches.length === 0) return;
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setActiveIdx((i) => (i + 1) % matches.length);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setActiveIdx((i) => (i - 1 + matches.length) % matches.length);
      } else if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        accept(matches[activeIdx]);
      } else if (e.key === 'Escape') {
        e.preventDefault();
        setSuppressedKey(ctxKey);
      }
    };

    const handleChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
      onChange(e.target.value);
      setCursor(e.target.selectionStart);
    };

    const onScroll = () => {
      if (preRef.current && taRef.current) {
        preRef.current.scrollTop = taRef.current.scrollTop;
        preRef.current.scrollLeft = taRef.current.scrollLeft;
      }
    };

    useImperativeHandle(
      ref,
      () => ({
        insert(fragment: string) {
          const ta = taRef.current;
          const v = value;
          const start = ta ? (ta.selectionStart ?? v.length) : v.length;
          const end = ta ? (ta.selectionEnd ?? v.length) : v.length;
          const next = v.slice(0, start) + fragment + v.slice(end);
          onChange(next);
          requestAnimationFrame(() => {
            if (ta) {
              const pos = start + fragment.length;
              ta.focus();
              ta.setSelectionRange(pos, pos);
              setCursor(pos);
            }
          });
        },
        focus() {
          taRef.current?.focus();
        },
      }),
      [value, onChange],
    );

    return (
      <div className="formula-editor">
        <pre className="formula-editor__overlay" aria-hidden="true" ref={preRef}>
          {tokens.map((t, i) => {
            if (t.type === 'ref') {
              const inner = t.text.slice(1, -1);
              const known = inner === '' || knownSet.has(inner);
              return (
                <span key={i} className={`tok tok-ref${known ? '' : ' tok-err'}`}>
                  {t.text}
                </span>
              );
            }
            if (t.type === 'fn') return <span key={i} className="tok tok-fn">{t.text}</span>;
            if (t.type === 'num') return <span key={i} className="tok tok-num">{t.text}</span>;
            if (t.type === 'op') return <span key={i} className="tok tok-op">{t.text}</span>;
            return t.text;
          })}
          {value.endsWith('\n') ? ' ' : ''}
        </pre>
        <textarea
          ref={taRef}
          className="formula-editor__ta"
          rows={rows}
          value={value}
          readOnly={readOnly}
          spellCheck={false}
          placeholder={placeholder}
          onChange={handleChange}
          onKeyDown={onKeyDown}
          onKeyUp={syncCursor}
          onClick={syncCursor}
          onSelect={syncCursor}
          onScroll={onScroll}
        />
        {open && (
          <div className="formula-editor__complete" role="listbox">
            {matches.map((op, i) => (
              <button
                key={op.token}
                type="button"
                role="option"
                aria-selected={i === activeIdx}
                className={`formula-editor__opt${i === activeIdx ? ' is-active' : ''}`}
                // onMouseDown (et non onClick) + preventDefault pour garder le
                // focus sur le textarea et préserver la position du curseur
                // lue par `accept`.
                onMouseDown={(e) => {
                  e.preventDefault();
                  accept(op);
                }}
                title={op.label}
              >
                <span className="formula-editor__opt-token">{op.token}</span>
                <span className="formula-editor__opt-label">{op.label}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    );
  },
);
