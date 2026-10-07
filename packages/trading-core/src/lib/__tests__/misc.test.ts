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
