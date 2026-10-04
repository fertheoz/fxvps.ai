import { describe, expect, it } from 'vitest';
import type { Position } from '../../api/types';
import { dragProtection, hitLine } from '../chartDrag';

const pos = (side: 'buy' | 'sell'): Position => ({
  id: '7',
  accountId: 'A',
  symbol: 'EURUSD',
  side,
  volume: 10,
  openPrice: 1.1,
  openTime: 0,
  sl: side === 'buy' ? 1.09 : 1.11,
  tp: side === 'buy' ? 1.12 : 1.08,
  trailing: 0.002,
  commission: 0,
  swap: 0,
});
const q = { bid: 1.1, ask: 1.1001 };

describe('chart SL/TP drag', () => {
  it('finds the nearest line within the tolerance', () => {
    const lines = [
      { positionId: '1', kind: 'sl' as const, y: 100 },
      { positionId: '2', kind: 'tp' as const, y: 104 },
    ];
    expect(hitLine(lines, 103)?.positionId).toBe('2');
    expect(hitLine(lines, 99)?.positionId).toBe('1');
    expect(hitLine(lines, 120)).toBeUndefined();
  });

  it('moves one level and keeps the other and the trailing distance', () => {
    expect(dragProtection(pos('buy'), 'sl', 1.0951234, 5, q)).toEqual({ sl: 1.09512, tp: 1.12, trailing: 0.002 });
    expect(dragProtection(pos('buy'), 'tp', 1.13, 5, q)).toEqual({ sl: 1.09, tp: 1.13, trailing: 0.002 });
    expect(dragProtection(pos('sell'), 'sl', 1.105, 5, q)).toEqual({ sl: 1.105, tp: 1.08, trailing: 0.002 });
  });

  it('refuses levels through the market', () => {
    expect(dragProtection(pos('buy'), 'sl', 1.1005, 5, q)).toBeUndefined();
    expect(dragProtection(pos('buy'), 'tp', 1.099, 5, q)).toBeUndefined();
    expect(dragProtection(pos('sell'), 'sl', 1.1, 5, q)).toBeUndefined();
    expect(dragProtection(pos('sell'), 'tp', 1.11, 5, q)).toBeUndefined();
  });
});
