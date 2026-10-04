import type { Bar } from '@fxvps/trading-core';

export interface CandleGeom {
  x: number;
  width: number;
  wickTop: number;
  wickBottom: number;
  bodyTop: number;
  bodyHeight: number;
  up: boolean;
}

export interface CandleLayout {
  candles: CandleGeom[];
  min: number;
  max: number;
  /** Maps a price to a y pixel. */
  y: (price: number) => number;
}

/** Pure layout of OHLC bars into SVG rectangles (tested without rendering). */
export function layoutCandles(bars: readonly Bar[], width: number, height: number, pad = 4): CandleLayout {
  if (bars.length === 0 || width <= 0 || height <= 0) {
    return { candles: [], min: 0, max: 0, y: () => 0 };
  }
  let min = Infinity;
  let max = -Infinity;
  for (const b of bars) {
    if (b.low < min) min = b.low;
    if (b.high > max) max = b.high;
  }
  const range = max - min || 1;
  const inner = height - pad * 2;
  const y = (p: number) => pad + ((max - p) / range) * inner;
  const slot = width / bars.length;
  const bodyW = Math.max(1, slot * 0.7);
  const candles = bars.map((b, i) => {
    const up = b.close >= b.open;
    const top = y(Math.max(b.open, b.close));
    const bottom = y(Math.min(b.open, b.close));
    return {
      x: i * slot + (slot - bodyW) / 2,
      width: bodyW,
      wickTop: y(b.high),
      wickBottom: y(b.low),
      bodyTop: top,
      bodyHeight: Math.max(1, bottom - top),
      up,
    };
  });
  return { candles, min, max, y };
}
