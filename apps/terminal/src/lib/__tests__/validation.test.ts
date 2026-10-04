import { describe, expect, it } from 'vitest';
import { validateTicket, type TicketInput } from '../validation';
import { quotes, specs } from '../../test/fixtures';

const ctx = { spec: specs.EURUSD!, quote: quotes.EURUSD, now: 1000 };
const keys = (i: Partial<TicketInput>, extra: Partial<typeof ctx & { requiredMargin: number; freeMargin: number }> = {}) =>
  validateTicket({ side: 'buy', type: 'market', volume: 10, ...i }, { ...ctx, ...extra }).map((e) => e.key);

describe('validateTicket', () => {
  it('accepts a plain market order', () => {
    expect(keys({})).toEqual([]);
  });
  it('validates volume bounds and step', () => {
    expect(keys({ volume: null })).toContain('volume.required');
    expect(keys({ volume: 0 })).toContain('volume.required');
    expect(keys({ volume: 20_000 })).toContain('volume.max');
    const us500 = { ...ctx, spec: specs.US500!, quote: { bid: 5200, ask: 5200.5 } };
    expect(validateTicket({ side: 'buy', type: 'market', volume: 15 }, us500).map((e) => e.key)).toContain('volume.step');
    expect(validateTicket({ side: 'buy', type: 'market', volume: 5 }, us500).map((e) => e.key)).toContain('volume.min');
  });
  it('checks pending price side', () => {
    expect(keys({ type: 'limit', price: 1.08 })).toEqual([]); // buy limit below ask
    expect(keys({ type: 'limit', price: 1.09 })).toContain('price.side');
    expect(keys({ type: 'stop', price: 1.09 })).toEqual([]);
    expect(keys({ type: 'stop', price: 1.08 })).toContain('price.side');
    expect(keys({ side: 'sell', type: 'limit', price: 1.09 })).toEqual([]);
    expect(keys({ side: 'sell', type: 'stop', price: 1.09 })).toContain('price.side');
    expect(keys({ type: 'limit' })).toContain('price.required');
  });
  it('checks stop-limit relationship', () => {
    expect(keys({ type: 'stop_limit', price: 1.09, limitPrice: 1.089 })).toEqual([]);
    expect(keys({ type: 'stop_limit', price: 1.09, limitPrice: 1.091 })).toContain('limitPrice.side');
    expect(keys({ type: 'stop_limit', price: 1.09 })).toContain('limitPrice.required');
  });
  it('checks SL/TP relative to entry', () => {
    expect(keys({ sl: 1.08, tp: 1.09 })).toEqual([]);
    expect(keys({ sl: 1.09 })).toContain('sl.side');
    expect(keys({ tp: 1.08 })).toContain('tp.side');
    expect(keys({ side: 'sell', sl: 1.09, tp: 1.08 })).toEqual([]);
    // entry for a buy limit is the limit price
    expect(keys({ type: 'limit', price: 1.08, sl: 1.081 })).toContain('sl.side');
  });
  it('rejects past expiry on pending orders only', () => {
    expect(keys({ type: 'limit', price: 1.08, expiry: 500 })).toContain('expiry.past');
    expect(keys({ type: 'limit', price: 1.08, expiry: 5000 })).toEqual([]);
  });
  it('checks free margin for market orders', () => {
    expect(keys({}, { requiredMargin: 200, freeMargin: 100 })).toContain('margin.insufficient');
    expect(keys({}, { requiredMargin: 100, freeMargin: 100 })).toEqual([]);
  });
});
