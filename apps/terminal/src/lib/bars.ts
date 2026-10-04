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
