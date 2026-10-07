import type { DepthLevel, Side } from '../api/types';
import { big } from './money';

export interface VwapFill {
  /** Volume that could be filled (centi-lots). */
  filled: number;
  /** Volume-weighted average price, rounded to `digits`, null if nothing filled. */
  avgPrice: number | null;
  /** Worst price touched. */
  worstPrice: number | null;
  levelsUsed: number;
}

/**
 * Walk the book for a market order of `volume`.
 * Buys consume asks (ascending), sells consume bids (descending).
 */
/** Levels further than this from the top of the book are not fillable: the
 *  engine's circuit breaker (0.5 % of the requested price) never lets an order
 *  reach them, so the ladder must not pretend they fill. */
export const FILL_GUARD_BPS = 50;

export function vwapFill(side: Side, volume: number, bids: DepthLevel[], asks: DepthLevel[], digits: number, guardBps = FILL_GUARD_BPS): VwapFill {
  let book = side === 'buy' ? [...asks].sort((a, b) => a.price - b.price) : [...bids].sort((a, b) => b.price - a.price);
  const top = book[0]?.price;
  if (top !== undefined && guardBps > 0) {
    const limit = top * (guardBps / 10_000);
    book = book.filter((l) => Math.abs(l.price - top) <= limit);
  }
  let remaining = volume;
  let notional = big(0);
  let filled = 0;
  let worst: number | null = null;
  let used = 0;
  for (const lvl of book) {
    if (remaining <= 0) break;
    const take = Math.min(remaining, lvl.volume);
    if (take <= 0) continue;
    notional = notional.plus(big(lvl.price).times(take));
    filled += take;
    remaining -= take;
    worst = lvl.price;
    used++;
  }
  return {
    filled,
    avgPrice: filled > 0 ? Number(notional.div(filled).round(digits).toFixed(digits)) : null,
    worstPrice: worst,
    levelsUsed: used,
  };
}
