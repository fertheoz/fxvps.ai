import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useTerminal } from '../store/terminal';
import { useT } from '../hooks';

export type DockSide = 'left' | 'right';

/** 📌 in a side panel's header: pinned panels sit in the split layout, unpinned ones fold into an edge strip. */
export function PinButton({ side }: { side: DockSide }) {
  const t = useT();
  const pinned = useTerminal((s) => s.sidePinned[side]);
  const setPinned = useTerminal((s) => s.setSidePinned);
  return (
    <button
      className={`ml-auto w-5 h-5 grid place-items-center rounded text-[11px] ${pinned ? 'text-muted hover:text-fg' : 'text-accent'} hover:bg-panel-2`}
      title={pinned ? t('side.unpin') : t('side.pin')}
      aria-label={pinned ? t('side.unpin') : t('side.pin')}
      aria-pressed={pinned}
      onClick={() => setPinned(side, !pinned)}
      data-testid={`pin-${side}`}
    >
      {pinned ? '📌' : '📍'}
    </button>
  );
}

/**
 * Edge strip for an unpinned side panel: a vertical label; hovering slides the
 * panel out over the workspace, leaving it folds it back after a short delay.
 */
export function SideDock({ side, title, children }: { side: DockSide; title: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const show = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setOpen(true);
  };
  const hide = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => setOpen(false), 350);
  };
  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);
  const left = side === 'left';
  return (
    <div
      className={`relative shrink-0 w-6 bg-panel ${left ? 'border-r' : 'border-l'} border-line`}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={show}
      onBlur={hide}
      data-testid={`dock-${side}`}
      data-open={open}
    >
      <button
        className="w-6 h-full flex items-start justify-center pt-2 text-[11px] text-muted hover:text-fg"
        style={{ writingMode: 'vertical-rl' }}
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
      >
        {title}
      </button>
      <div
        className={`absolute top-0 bottom-0 ${left ? 'left-6' : 'right-6'} w-[320px] z-30 bg-panel border ${left ? 'border-l-0' : 'border-r-0'} border-line shadow-2xl transition-all duration-200 ease-out ${
          open ? 'translate-x-0 opacity-100' : `${left ? '-translate-x-3' : 'translate-x-3'} opacity-0 pointer-events-none`
        }`}
        aria-hidden={!open}
      >
        {children}
      </div>
    </div>
  );
}
