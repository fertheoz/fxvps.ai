import type { Deal, TradingApi, TradingEvent } from '@fxvps/trading-core';
import { formatPrice, minorToMajorString, volumeToLots } from '@fxvps/trading-core';
import { useTerminal } from '../store/terminal';
import { notifyEvent, setConnectionStatus, type NotifyEvent } from './commands';
import { isTauri } from './tauri';

function digits(symbol: string): number | undefined {
  return useTerminal.getState().symbols[symbol]?.digits;
}

function price(symbol: string, p: number): string {
  const d = digits(symbol);
  return d == null ? String(p) : formatPrice(p, d);
}

/** Maps a deal to a native notification, or null when it is not worth one. */
export function dealNotification(deal: Deal): NotifyEvent | null {
  const lots = volumeToLots(deal.volume);
  if (deal.reason === 'sl' || deal.reason === 'tp' || deal.reason === 'stop_out')
    return {
      kind: 'position_closed',
      symbol: deal.symbol,
      reason: deal.reason,
      lots,
      price: price(deal.symbol, deal.price),
      profit: minorToMajorString(deal.profit),
    };
  if (deal.entry === 'in' || deal.reason === 'order')
    return { kind: 'fill', symbol: deal.symbol, side: deal.side, lots, price: price(deal.symbol, deal.price) };
  return null;
}

/**
 * Forwards trading events to the desktop shell: connection state to the tray and
 * fills / SL / TP / stop-outs to OS notifications. No-op in the browser.
 * Returns an unsubscribe function.
 */
export function startNativeBridge(api: TradingApi, force = false): () => void {
  if (!force && !isTauri()) return () => undefined;
  let last = '';
  return api.onEvent((e: TradingEvent) => {
    if (e.type === 'connection') {
      // Forward state changes, and latency only when it moves by >= 10 ms.
      const key = `${e.state}:${e.latencyMs == null ? '' : Math.round(e.latencyMs / 10)}`;
      if (key === last) return;
      last = key;
      void setConnectionStatus(e.state, e.latencyMs);
    } else if (e.type === 'deal') {
      const n = dealNotification(e.deal);
      if (n) void notifyEvent(n);
    }
  });
}
