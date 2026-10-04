import { describe, expect, it } from 'vitest';
import { MockTradingApi } from './mock';
import type { Deal, Position, TradingEvent } from './types';

function setup() {
  const api = new MockTradingApi({ latencyMs: 0, tickIntervalMs: 0, now: () => 1_700_000_000_000 });
  const events: TradingEvent[] = [];
  api.onEvent((e) => events.push(e));
  const positions = () => {
    const last = [...events].reverse().find((e) => e.type === 'positions' && e.accountId === '100001');
    return last && last.type === 'positions' ? last.positions : ([] as Position[]);
  };
  const balance = async () => (await api.getAccounts()).find((a) => a.id === '100001')!.balance;
  return { api, events, positions, balance };
}

describe('MockTradingApi', () => {
  it('fills market orders at ask/bid and realizes P/L exactly on close', async () => {
    const { api, positions, balance } = setup();
    api.setMid('EURUSD', 1.085);
    const q = api.getQuote('EURUSD')!;
    const r = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 100 });
    expect(r).toMatchObject({ ok: true, price: q.ask });
    expect(positions()).toHaveLength(1);
    api.setMid('EURUSD', 1.0861); // +11 pips mid
    const exitBid = api.getQuote('EURUSD')!.bid;
    const before = await balance();
    await api.closePosition('100001', r.ok ? r.positionId! : '', undefined);
    const after = await balance();
    const expectedProfit = Math.round((exitBid - q.ask) * 100_000 * 100); // cents; integers here
    // profit + entry commission (-350) + exit commission (-350)
    expect(after - before).toBe(expectedProfit - 700);
    expect(positions()).toHaveLength(0);
  });

  it('supports partial close with pro-rata commission', async () => {
    const { api, positions } = setup();
    const r = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'sell', type: 'market', volume: 30 });
    if (!r.ok) throw new Error(r.error);
    await api.closePosition('100001', r.positionId!, 10);
    expect(positions()[0]).toMatchObject({ volume: 20, commission: -105 + 35 });
    const bad = await api.closePosition('100001', r.positionId!, 50);
    expect(bad.ok).toBe(false);
  });

  it('rejects orders exceeding free margin', async () => {
    const { api } = setup();
    // account 100002 has $10,000 at 1:200; 30 lots EURUSD needs ~$16k
    const r = await api.placeOrder({ accountId: '100002', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 3000 });
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toContain('margin.insufficient');
  });

  it('triggers buy limit when ask reaches the price', async () => {
    const { api, positions, events } = setup();
    api.setMid('EURUSD', 1.085);
    const r = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'limit', volume: 10, price: 1.08 });
    expect(r.ok && r.orderId).toBeTruthy();
    api.setMid('EURUSD', 1.082);
    expect(positions()).toHaveLength(0);
    api.setMid('EURUSD', 1.0795);
    expect(positions()).toHaveLength(1);
    expect(positions()[0]!.openPrice).toBeLessThanOrEqual(1.08);
    const deal = events.find((e): e is { type: 'deal'; deal: Deal } => e.type === 'deal')!.deal;
    expect(deal.reason).toBe('order');
  });

  it('stop-limit: triggers at stop, fills at limit', async () => {
    const { api, positions } = setup();
    api.setMid('EURUSD', 1.085);
    await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'stop_limit', volume: 10, price: 1.09, limitPrice: 1.088 });
    api.setMid('EURUSD', 1.0905); // trigger
    expect(positions()).toHaveLength(0);
    api.setMid('EURUSD', 1.0878); // ask <= limit
    expect(positions()).toHaveLength(1);
  });

  it('closes on stop loss', async () => {
    const { api, positions, events } = setup();
    api.setMid('EURUSD', 1.085);
    await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 10, sl: 1.083 });
    expect(positions()).toHaveLength(1);
    api.setMid('EURUSD', 1.0825);
    expect(positions()).toHaveLength(0);
    const out = events.filter((e) => e.type === 'deal').map((e) => (e.type === 'deal' ? e.deal : null)).at(-1);
    expect(out?.reason).toBe('sl');
  });

  it('streams quotes to subscribers', () => {
    const { api } = setup();
    const got: string[] = [];
    const off = api.subscribeQuotes(['EURUSD'], (qs) => qs.forEach((x) => got.push(x.symbol)));
    api.tick(['EURUSD', 'GBPUSD']);
    off();
    api.tick(['EURUSD']);
    expect(got).toEqual(['EURUSD', 'EURUSD']);
  });

  it('parity with the gateway: trailing stop, OCO groups, in-place order modify', async () => {
    const { api, events, positions } = setup();
    api.setMid('EURUSD', 1.1);
    const r = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 10, trailing: 0.001 });
    if (!r.ok) throw new Error(r.error);
    expect(positions()[0]!.trailing).toBe(0.001);
    api.setMid('EURUSD', 1.103);
    const sl = positions()[0]!.sl!;
    expect(sl).toBeGreaterThan(1.1);
    api.setMid('EURUSD', 1.1025); // never moves back
    expect(positions()[0]!.sl).toBe(sl);
    api.setMid('EURUSD', 1.1);
    expect(positions()).toHaveLength(0);
    expect(events.some((e) => e.type === 'deal' && e.deal.reason === 'sl')).toBe(true);

    const a = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'stop', volume: 10, price: 1.102, ocoGroup: 9 });
    const b = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'buy', type: 'limit', volume: 10, price: 1.09, ocoGroup: 9 });
    if (!a.ok || !b.ok) throw new Error('place');
    expect(await api.modifyOrder('100001', b.orderId!, { volume: 20, price: 1.091, sl: 1.08 })).toMatchObject({ ok: true });
    let orders = [...events].reverse().find((e) => e.type === 'orders');
    expect(orders).toMatchObject({ orders: [{ id: a.orderId }, { id: b.orderId, volume: 20, price: 1.091, sl: 1.08 }] });
    expect((await api.modifyOrder('100001', b.orderId!, { volume: 7 })).ok).toBe(false);
    api.setMid('EURUSD', 1.103);
    orders = [...events].reverse().find((e) => e.type === 'orders');
    expect(orders).toMatchObject({ orders: [] });
    expect(positions()).toHaveLength(1);
  });

  it('modifyPosition sets and clears the trailing distance', async () => {
    const { api, positions } = setup();
    const r = await api.placeOrder({ accountId: '100001', symbol: 'EURUSD', side: 'sell', type: 'market', volume: 10 });
    if (!r.ok) throw new Error(r.error);
    expect((await api.modifyPosition('100001', r.positionId!, undefined, undefined, 0.002)).ok).toBe(true);
    expect(positions()[0]!.trailing).toBe(0.002);
    expect((await api.modifyPosition('100001', r.positionId!, undefined, undefined, -1)).ok).toBe(false);
    await api.modifyPosition('100001', r.positionId!);
    expect(positions()[0]!.trailing).toBeUndefined();
    expect((await api.getAccounts())[0]!.marginMode).toBe('hedging');
  });
});
