import { useEffect, useRef, type ReactNode } from 'react';

export interface MenuItem {
  label: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  /** Draws a divider above the item. */
  separator?: boolean;
  /** Small hint at the right (shortcut, state). */
  hint?: ReactNode;
}

/**
 * Right-click menu at a screen position. Closes on outside click, Escape,
 * scroll or window blur; kept inside the viewport.
 */
export function ContextMenu({ x, y, items, onClose, testId }: { x: number; y: number; items: MenuItem[]; onClose: () => void; testId?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) {
      const r = el.getBoundingClientRect();
      if (r.right > window.innerWidth) el.style.left = `${Math.max(0, x - r.width)}px`;
      if (r.bottom > window.innerHeight) el.style.top = `${Math.max(0, y - r.height)}px`;
    }
    const down = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('mousedown', down, true);
    window.addEventListener('keydown', key);
    window.addEventListener('scroll', onClose, true);
    window.addEventListener('blur', onClose);
    return () => {
      window.removeEventListener('mousedown', down, true);
      window.removeEventListener('keydown', key);
      window.removeEventListener('scroll', onClose, true);
      window.removeEventListener('blur', onClose);
    };
  }, [x, y, onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      data-testid={testId ?? 'context-menu'}
      className="fixed z-50 min-w-[200px] py-1 rounded-md border border-line bg-panel shadow-2xl text-[12px]"
      style={{ left: x, top: y }}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((it, i) => (
        <div key={i}>
          {it.separator && <div className="my-1 border-t border-line/70" />}
          <button
            role="menuitem"
            disabled={it.disabled}
            className="w-full flex items-center justify-between gap-4 px-3 h-7 text-left hover:bg-hover disabled:opacity-40 disabled:hover:bg-transparent"
            onClick={() => {
              if (it.disabled) return;
              it.onClick?.();
              onClose();
            }}
          >
            <span>{it.label}</span>
            {it.hint !== undefined && <span className="text-muted text-[11px]">{it.hint}</span>}
          </button>
        </div>
      ))}
    </div>
  );
}
