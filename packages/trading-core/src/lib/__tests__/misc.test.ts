import { describe, expect, it, vi } from 'vitest';
import { bollinger, ema, rsi, sma } from '../indicators';
import { vwapFill } from '../depth';
import { applyTick, bucketStart } from '../bars';
import { RafBatcher } from '../rafBatcher';

describe('indicators', () => {
  it('sma', () => {
    expect(sma([1, 2, 3, 4, 5], 3)).toEqual([null, null, 2, 3, 4]);
  });
  it('ema seeds with sma and converges', () => {
    const e = ema([1, 2, 3, 4, 5], 3);
    expect(e[2]).toBe(2);
    expect(e[3]).toBeCloseTo(3);
    expect(e[4]).toBeCloseTo(4);
  });
  it('bollinger of a constant series has zero width', () => {
    const b = bollinger([5, 5, 5, 5], 3)[3]!;
    expect(b).toEqual({ middle: 5, upper: 5, lower: 5 });
  });
  it('rsi is 100 for monotonic rises and ~50 for alternation', () => {
    expect(rsi([1, 2, 3, 4, 5, 6], 3)[5]).toBe(100);
    const alt = Array.from({ length: 40 }, (_, i) => (i % 2 ? 11 : 10));
    expect(rsi(alt, 14)[39]!).toBeGreaterThan(40);
    expect(rsi(alt, 14)[39]!).toBeLessThan(60);
  });
});

describe('vwapFill', () => {
  const asks = [
    { price: 1.1002, volume: 100 },
    { price: 1.1001, volume: 50 },
  ];
  const bids = [{ price: 1.1, volume: 30 }];
  it('walks the book in price priority', () => {
    const f = vwapFill('buy', 100, bids, asks, 5);
    // 50 @ 1.1001 + 50 @ 1.1002 -> 1.10015
    expect(f).toEqual({ filled: 100, avgPrice: 1.10015, worstPrice: 1.1002, levelsUsed: 2 });
  });
  it('reports partial fills', () => {
    const f = vwapFill('sell', 100, bids, asks, 5);
    expect(f.filled).toBe(30);
    expect(f.avgPrice).toBe(1.1);
  });
  it('ignores levels beyond the fill guard (stale far orders in a deep book)', () => {
    const deep = [
      { price: 4108.9, volume: 100 },
      { price: 5304.57, volume: 5000 }, // 29 % away: never fillable
    ];
    const f = vwapFill('buy', 1000, [], deep, 2);
    expect(f).toEqual({ filled: 100, avgPrice: 4108.9, worstPrice: 4108.9, levelsUsed: 1 });
    expect(vwapFill('buy', 1000, [], deep, 2, 0).filled).toBe(1000);
  });
});

describe('bars', () => {
  it('updates current bar and opens the next one', () => {
    expect(bucketStart(125, 60)).toBe(120);
    const b1 = applyTick(undefined, 10, 125, 60);
    expect(b1).toMatchObject({ time: 120, open: 10, close: 10 });
    const b2 = applyTick(b1, 12, 130, 60);
    expect(b2).toMatchObject({ time: 120, open: 10, high: 12, low: 10, close: 12, volume: 2 });
    const b3 = applyTick(b2, 9, 181, 60);
    expect(b3).toMatchObject({ time: 180, open: 9 });
  });
});

describe('RafBatcher', () => {
  it('conflates per key and flushes once per frame', () => {
    const frames: (() => void)[] = [];
    const flush = vi.fn();
    const b = new RafBatcher<number>(flush, (cb) => frames.push(cb));
    b.push('a', 1);
    b.push('a', 2);
    b.push('b', 3);
    expect(frames).toHaveLength(1);
    expect(flush).not.toHaveBeenCalled();
    frames[0]!();
    expect(flush).toHaveBeenCalledWith([2, 3]);
    b.push('a', 4);
    expect(frames).toHaveLength(2);
  });
});

describe('heikinAshi', () => {
  it('averages the bar and chains the open from the previous HA bar', async () => {
    const { heikinAshi } = await import('../bars');
    const ha = heikinAshi([
      { time: 0, open: 10, high: 14, low: 8, close: 12, volume: 1 },
      { time: 60, open: 12, high: 16, low: 11, close: 15, volume: 2 },
    ]);
    expect(ha[0]).toEqual({ time: 0, open: 11, high: 14, low: 8, close: 11, volume: 1 });
    expect(ha[1]).toMatchObject({ open: 11, close: 13.5, high: 16, low: 11 });
  });
});

describe('renko', () => {
  it('builds whole-box bricks with strictly increasing times', async () => {
    const { renko } = await import('../bars');
    const bar = (time: number, close: number) => ({ time, open: close, high: close, low: close, close, volume: 1 });
    // 10 -> 13: three up bricks in one bar; 11.5: no brick (a reversal needs a close below 11);
    // 9: the reversal starts at the last brick's open (12) -> 12-11, 11-10, 10-9
    const r = renko([bar(0, 10), bar(60, 13), bar(120, 11.5), bar(180, 9)], 1);
    expect(r.map((x) => [x.open, x.close])).toEqual([[10, 11], [11, 12], [12, 13], [12, 11], [11, 10], [10, 9]]);
    expect(r.map((x) => x.time)).toEqual([60, 61, 62, 180, 181, 182]);
  });
});

describe('backtestMaCross', () => {
  it('trades crosses at the next open, deterministically, with costs', async () => {
    const { backtestMaCross } = await import('../backtest');
    // falls, then rises, then falls: one long and one short leg
    const closes = [10, 9, 8, 7, 8, 9, 10, 11, 12, 11, 10, 9, 8, 7];
    const bars = closes.map((c, i) => ({ time: i * 60, open: c, high: c + 0.5, low: c - 0.5, close: c, volume: 1 }));
    const p = { fast: 2, slow: 3, kind: 'sma' as const, units: 1, costPrice: 0.1 };
    const r = backtestMaCross(bars, p);
    expect(r.trades.map((t) => t.side)).toEqual(['buy', 'sell']);
    expect(r.trades[0]?.entryTime).toBeGreaterThan(0);
    // same input, same output
    expect(backtestMaCross(bars, p)).toEqual(r);
    expect(r.net).toBeCloseTo(r.trades.reduce((a, t) => a + t.pnl, 0));
    expect(r.equity.length).toBe(r.trades.length);
  });

  // fast 1 / slow 2 SMA: a cross at bar i only depends on closes i-3..i-1, so
  // the bars below place the signals exactly: long at bar 3's open (100),
  // short at bar 5's open. SL / TP 5 price units, no costs, 1 unit.
  type B = [open: number, high: number, low: number, close: number];
  const mk = (rows: B[]) => rows.map(([open, high, low, close], i) => ({ time: i * 60, open, high, low, close, volume: 1 }));
  const head: B[] = [
    [100, 100, 100, 100],
    [100, 100, 99, 99],
    [99, 101, 99, 101], // up cross -> long at the next open
    [100, 102, 99, 101], // long @100, SL 95 / TP 105 untouched
  ];
  const quiet: B = [101, 101.5, 97.5, 98]; // no signal, no SL / TP; down cross -> short at the next open
  const sl = { fast: 1, slow: 2, kind: 'sma' as const, units: 1, costPrice: 0, stopPrice: 5, takePrice: 5 };
  const brief = (r: { trades: { side: string; entry: number; exit: number; reason: string; pnl: number }[] }) =>
    r.trades.map((t) => [t.side, t.entry, t.exit, t.reason, t.pnl]);

  it('executes the signal at the open before the bar range, then checks the new position on its entry bar', async () => {
    const { backtestMaCross } = await import('../backtest');
    // the reversal bar opens at 96 and later trades up to 106: the long is closed by
    // the signal at 96 (not by its 105 target, which comes after the exit), and the
    // short opened at 96 is stopped at 101 inside the same bar
    const r = backtestMaCross(mk([...head, quiet, [96, 106, 95.5, 100]]), sl);
    expect(brief(r)).toEqual([
      ['buy', 100, 96, 'signal', -4],
      ['sell', 96, 101, 'stop', -5],
    ]);
    expect(r.trades[1]).toMatchObject({ entryTime: 300, exitTime: 300 });
    // stop and target both inside the entry bar: the stop wins
    const both = backtestMaCross(mk([...head, quiet, [96, 106, 90, 100]]), sl);
    expect(brief(both)[1]).toEqual(['sell', 96, 101, 'stop', -5]);
  });

  it('closes a held position on its target inside a bar with no signal; the later signal opens a fresh one', async () => {
    const { backtestMaCross } = await import('../backtest');
    const r = backtestMaCross(mk([...head, [101, 106, 97.5, 98], [96, 97, 95, 95.5]]), sl);
    expect(brief(r)).toEqual([
      ['buy', 100, 105, 'take', 5],
      ['sell', 96, 95.5, 'end', 0.5],
    ]);
  });

  it('fills a stop the bar gaps through at the open, a gapped target at the target', async () => {
    const { backtestMaCross } = await import('../backtest');
    const longGap = backtestMaCross(mk([...head, [93, 94, 92, 93.5]]), sl);
    expect(brief(longGap)).toEqual([['buy', 100, 93, 'stop', -7]]);
    // mirror image around 100: a short gapped through its stop at 105
    const m = (rows: B[]) => rows.map(([o, h, l, c]): B => [200 - o, 200 - l, 200 - h, 200 - c]);
    const shortGap = backtestMaCross(mk(m([...head, [93, 94, 92, 93.5]])), sl);
    expect(brief(shortGap)).toEqual([['sell', 100, 107, 'stop', -7]]);
    // a favourable gap past the target is still booked at the target (pessimistic)
    const takeGap = backtestMaCross(mk([...head, [107, 108, 106.5, 107]]), sl);
    expect(brief(takeGap)).toEqual([['buy', 100, 105, 'take', 5]]);
  });
});
