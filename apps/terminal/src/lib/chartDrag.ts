import type { Position } from '@fxvps/trading-core';
import { closePrice, roundPrice } from '@fxvps/trading-core';

/** A protective line on the chart that can be dragged to a new price. */
export interface DragLine {
  positionId: string;
  kind: 'sl' | 'tp';
  /** Pixel y of the line (series.priceToCoordinate). */
  y: number;
}

/** Line under the pointer (within `tolerance` px), nearest first. */
export function hitLine<T extends DragLine>(lines: readonly T[], y: number, tolerance = 5): T | undefined {
  let best: T | undefined;
  let bestD = tolerance + 1;
  for (const l of lines) {
    const d = Math.abs(l.y - y);
    if (d <= tolerance && d < bestD) {
      best = l;
      bestD = d;
    }
  }
  return best;
}

/**
 * New protection after dragging the `kind` line of `p` to `price`: the other
 * level and the trailing distance are kept. Returns undefined when the new
 * level would be on the wrong side of the closing price (it would trigger at
 * once), mirroring the server's validation.
 */
export function dragProtection(
  p: Position,
  kind: 'sl' | 'tp',
  price: number,
  digits: number,
  quote: { bid: number; ask: number },
): { sl?: number; tp?: number; trailing?: number } | undefined {
  const px = roundPrice(price, digits);
  const cp = closePrice(p.side, quote);
  const buy = p.side === 'buy';
  if (kind === 'sl' && (buy ? px >= cp : px <= cp)) return undefined;
  if (kind === 'tp' && (buy ? px <= cp : px >= cp)) return undefined;
  return { sl: kind === 'sl' ? px : p.sl, tp: kind === 'tp' ? px : p.tp, trailing: p.trailing };
}
