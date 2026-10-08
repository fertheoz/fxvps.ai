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

/** Renko box size: half the average bar range of the last 50 bars, at least one point. */
export function renkoBox(bars: Bar[], point: number): number {
  const tail = bars.slice(-50);
  if (!tail.length) return point;
  const avg = tail.reduce((a, b) => a + (b.high - b.low), 0) / tail.length;
  return Math.max(point, Math.round(avg / 2 / point) * point);
}

/**
 * Renko bricks from closes: a new brick when the close moves a full box
 * beyond the last brick's top (up) or bottom (down). Bricks formed in the
 * same bar get that bar's time plus 1 s per extra brick, so times stay
 * strictly increasing for the chart.
 */
export function renko(bars: Bar[], box: number): Bar[] {
  const out: Bar[] = [];
  if (!bars.length || !(box > 0)) return out;
  let lo = bars[0].close;
  let hi = bars[0].close;
  const eps = box * 1e-9;
  for (const b of bars) {
    let j = 0;
    for (;;) {
      if (b.close >= hi + box - eps) {
        const open = hi;
        hi = open + box;
        lo = open;
        out.push({ time: b.time + j, open, high: hi, low: open, close: hi, volume: j === 0 ? b.volume : 0 });
      } else if (b.close <= lo - box + eps) {
        const open = lo;
        lo = open - box;
        hi = open;
        out.push({ time: b.time + j, open, high: open, low: lo, close: lo, volume: j === 0 ? b.volume : 0 });
      } else break;
      j++;
    }
  }
  return out;
}
