import { useEffect } from 'react';
import { TIMEFRAMES } from '@fxvps/trading-core';
import { useTerminal, type ChartLayout } from './store/terminal';
import { trade } from './store/api';
import type { MessageKey } from './i18n';

/** Single source of truth for the shortcut list (also rendered in the help dialog and README). */
export const SHORTCUTS: { keys: string; desc: MessageKey }[] = [
  { keys: 'F9', desc: 'sc.F9' },
  { keys: 'Ctrl/⌘ + K', desc: 'sc.palette' },
  { keys: 'F7 / F8', desc: 'sc.F7F8' },
  { keys: 'Alt + 1 / 2 / 4', desc: 'sc.layout' },
  { keys: '[ / ]', desc: 'sc.tf' },
  { keys: 'Shift + B / Shift + S', desc: 'sc.buySell' },
  { keys: 'Alt + T', desc: 'sc.theme' },
  { keys: '/', desc: 'sc.search' },
  { keys: '?', desc: 'sc.help' },
  { keys: 'Esc', desc: 'sc.esc' },
];

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable;
}

export function handleShortcut(e: KeyboardEvent): boolean {
  const s = useTerminal.getState();
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.key.toLowerCase() === 'k') {
    s.setPaletteOpen(!s.paletteOpen);
    return true;
  }
  if (e.key === 'F9') {
    s.openTicket();
    return true;
  }
  if (e.key === 'Escape') {
    if (s.ticket) s.closeTicket();
    else if (s.paletteOpen) s.setPaletteOpen(false);
    else if (s.shortcutsOpen) s.setShortcutsOpen(false);
    else return false;
    return true;
  }
  if (e.key === 'F7' || e.key === 'F8') {
    const n = s.layout;
    s.setActiveChart((s.activeChart + (e.key === 'F8' ? 1 : n - 1)) % n);
    return true;
  }
  if (isTyping(e.target) || mod) return false;
  if (e.altKey && ['1', '2', '4', '6'].includes(e.key)) {
    s.setLayout(Number(e.key) as ChartLayout);
    return true;
  }
  if (e.altKey && e.code === 'KeyT') {
    s.toggleTheme();
    return true;
  }
  if (e.key === '[' || e.key === ']') {
    const cur = s.charts[s.activeChart];
    if (!cur) return false;
    const i = TIMEFRAMES.indexOf(cur.timeframe);
    const next = TIMEFRAMES[Math.min(TIMEFRAMES.length - 1, Math.max(0, i + (e.key === ']' ? 1 : -1)))]!;
    s.setChartTimeframe(next);
    return true;
  }
  if (e.shiftKey && (e.key === 'B' || e.key === 'S')) {
    const sym = s.charts[s.activeChart]?.symbol;
    if (sym) void trade.market(sym, e.key === 'B' ? 'buy' : 'sell', s.oneClickVolume);
    return true;
  }
  if (e.key === '/') {
    document.getElementById('mw-search')?.focus();
    return true;
  }
  if (e.key === '?') {
    s.setShortcutsOpen(true);
    return true;
  }
  return false;
}

export function useKeyboardShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (handleShortcut(e)) e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}
