import type { Bar } from '../api/types';

/** Bucket start (unix seconds) for a timestamp and timeframe length. */
export function bucketStart(timeSec: number, tfSeconds: number): number {
  return Math.floor(timeSec / tfSeconds) * tfSeconds;
}

/**
 * Apply a tick to the last bar: update it in place (returns a new object) or
 * open a new bar if the tick falls into the next bucket.
 */
export function applyTick(last: Bar | undefined, price: number, timeSec: number, tfSeconds: number, volume = 1): Bar {
  const t = bucketStart(timeSec, tfSeconds);
  if (!last || t > last.time) return { time: t, open: price, high: price, low: price, close: price, volume };
  return {
    time: last.time,
    open: last.open,
    high: Math.max(last.high, price),
    low: Math.min(last.low, price),
    close: price,
    volume: last.volume + volume,
  };
}

/** One Heikin-Ashi bar from the raw bar and the previous HA bar. */
export function heikinAshiBar(prev: Bar | undefined, b: Bar): Bar {
  const close = (b.open + b.high + b.low + b.close) / 4;
  const open = prev ? (prev.open + prev.close) / 2 : (b.open + b.close) / 2;
  return { time: b.time, open, high: Math.max(b.high, open, close), low: Math.min(b.low, open, close), close, volume: b.volume };
}

/** Heikin-Ashi series (same times and volumes as the input). */
export function heikinAshi(bars: Bar[]): Bar[] {
  const out: Bar[] = [];
  for (const b of bars) out.push(heikinAshiBar(out[out.length - 1], b));
  return out;
}
