import { create, fromBinary, toBinary, type MessageInitShape } from '@bufbuild/protobuf';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { toDecimal } from './decimal';
import {
  DealEntry,
  DealReason,
  EnvelopeSchema,
  ErrorCode,
  MarginMode,
  OrderStatus,
  OrderType,
  Side,
  TimeInForce,
  type Envelope,
} from './gen/fxvps_client_v1_pb';
import type { Quote, TradingEvent } from '@fxvps/trading-core';
import { WsTradingApi } from './ws';

type Body = MessageInitShape<typeof EnvelopeSchema>['body'];
type Server = (env: Envelope, ws: FakeWs) => void;

/** In-memory WebSocket; frames the client sends go to `FakeWs.server`. */
class FakeWs {
  static last: FakeWs | undefined;
  static server: Server = () => undefined;
  readyState = 0;
  binaryType = 'blob';
  sent: Envelope[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: ((ev: { code: number; reason: string }) => void) | null = null;

  constructor(public url: string) {
    FakeWs.last = this;
    queueMicrotask(() => {
      this.readyState = 1;
      this.onopen?.();
    });
  }
  send(data: Uint8Array) {
    const env = fromBinary(EnvelopeSchema, data);
    this.sent.push(env);
    FakeWs.server(env, this);
  }
  close(code = 1000, reason = '') {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }
  push(body: Body) {
    const bytes = toBinary(EnvelopeSchema, create(EnvelopeSchema, { version: 1, body }));
    this.onmessage?.({ data: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) });
  }
}

const D = toDecimal;

/** Minimal client-gateway behaviour: handshake, symbol list, subscribe ack, market fills. */
const gateway: Server = (env, ws) => {
  const b = env.body;
  switch (b.case) {
    case 'hello':
      ws.push({ case: 'hello', value: { protocolVersion: 1, maxQuoteHz: 10 } });
      return;
    case 'auth':
      if (!b.value.token.startsWith('good')) {
        ws.push({ case: 'error', value: { code: ErrorCode.UNAUTHENTICATED, message: 'invalid token' } });
        ws.close(4001, 'unauthenticated');
        return;
      }
      ws.push({ case: 'authOk', value: { subject: 'u', accountIds: ['DEMO-1'] } });
      ws.push({ case: 'accountSnapshot', value: { accountId: 'DEMO-1', currency: 'USD', balance: D('100000'), positions: [] } });
      return;
    case 'symbolListRequest':
      ws.push({
        case: 'symbolList',
        value: {
          requestId: b.value.requestId,
          instruments: [
            { symbol: 'EURUSD', base: 'EUR', quote: 'USD', tickSize: D('0.00001'), digits: 5, qtyStep: D('1000'), contractSize: D('100000') },
            { symbol: 'USDJPY', base: 'USD', quote: 'JPY', tickSize: D('0.001'), digits: 3, qtyStep: D('1000'), contractSize: D('100000') },
          ],
        },
      });
      return;
    case 'subscribe':
      ws.push({ case: 'ack', value: { requestId: b.value.requestId } });
      return;
    case 'placeOrder': {
      const p = b.value;
      ws.push({ case: 'ack', value: { requestId: p.requestId } });
      ws.push({
        case: 'orderUpdate',
        value: {
          accountId: p.accountId,
          clientRequestId: p.requestId,
          orderId: 'O1',
          symbol: p.symbol,
          side: p.side,
          status: OrderStatus.FILLED,
          filledQty: p.qty,
          leavesQty: D('0'),
          avgPrice: D('1.08503'),
          lastQty: p.qty,
          lastPrice: D('1.08503'),
        },
      });
      ws.push({ case: 'positionUpdate', value: { accountId: p.accountId, position: { symbol: p.symbol, netQty: p.qty, avgPrice: D('1.08503') } } });
      return;
    }
    default:
  }
};

function setup(token = 'good') {
  const api = new WsTradingApi({ url: 'ws://gw/ws', token: () => token, WebSocketImpl: FakeWs as unknown as typeof WebSocket });
  const events: TradingEvent[] = [];
  api.onEvent((e) => events.push(e));
  const lastConn = () => events.filter((e) => e.type === 'connection').at(-1);
  return { api, events, lastConn };
}

describe('WsTradingApi against the client-gateway protocol', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'clearTimeout', 'clearInterval'] });
    FakeWs.last = undefined;
    FakeWs.server = gateway;
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('handshakes with Hello + Auth and exposes accounts and symbols', async () => {
    const { api } = setup();
    await api.connect();
    const ws = FakeWs.last!;
    expect(ws.binaryType).toBe('arraybuffer');
    expect(ws.sent[0]!.body.case).toBe('hello');
    expect(ws.sent[1]!.body).toMatchObject({ case: 'auth', value: { token: 'good' } });
    expect(await api.getAccounts()).toEqual([
      { id: 'DEMO-1', name: 'DEMO-1', currency: 'USD', balance: 10_000_000, leverage: 100, isDemo: true },
    ]);
    const syms = await api.getSymbols();
    expect(syms.map((s) => [s.name, s.digits, s.pipSize, s.contractSize, s.volumeStep])).toEqual([
      ['EURUSD', 5, '0.0001', 100000, 1],
      ['USDJPY', 3, '0.01', 100000, 1],
    ]);
    api.disconnect();
  });

  it('rejects a bad token without reconnecting', async () => {
    const { api, lastConn } = setup('bad');
    await expect(api.connect()).rejects.toThrow(/UNAUTHENTICATED/);
    expect(lastConn()).toMatchObject({ state: 'disconnected' });
  });

  it('reconnects with the refreshed token when the session token expires', async () => {
    let token = 'good';
    const api = new WsTradingApi({ url: 'ws://gw/ws', token: () => token, WebSocketImpl: FakeWs as unknown as typeof WebSocket });
    const states: string[] = [];
    api.onEvent((e) => e.type === 'connection' && states.push(e.state));
    await api.connect();
    const first = FakeWs.last!;
    token = 'good-refreshed';
    first.close(4001, 'token expired');
    await vi.advanceTimersByTimeAsync(600);
    const second = FakeWs.last!;
    expect(second).not.toBe(first);
    expect(second.sent[1]!.body).toMatchObject({ case: 'auth', value: { token: 'good-refreshed' } });
    expect(states.at(-1)).toBe('connected');
    api.disconnect();
  });

  it('subscribes, maps quote batches and resubscribes after a reconnect', async () => {
    const { api, lastConn } = setup();
    await api.connect();
    await api.getSymbols();
    const got: Quote[] = [];
    api.subscribeQuotes(['EURUSD', 'XAUUSD'], (q) => got.push(...q));
    const sub = FakeWs.last!.sent.find((e) => e.body.case === 'subscribe')!;
    // Symbols the gateway does not serve are not sent (they would fail the whole request).
    expect(sub.body.value).toMatchObject({ symbols: ['EURUSD'] });
    FakeWs.last!.push({
      case: 'quoteBatch',
      value: { quotes: [{ symbol: 'EURUSD', bid: D('1.08501'), ask: D('1.08503'), tsNs: 1_700_000_000_000_000_000n }] },
    });
    expect(got).toEqual([
      { symbol: 'EURUSD', bid: 1.08501, ask: 1.08503, time: 1_700_000_000_000, dayOpen: 1.08501, dayHigh: 1.08501, dayLow: 1.08501 },
    ]);

    const first = FakeWs.last!;
    first.close(4008, 'slow consumer');
    expect(lastConn()).toMatchObject({ state: 'reconnecting' });
    await vi.advanceTimersByTimeAsync(600);
    const second = FakeWs.last!;
    expect(second).not.toBe(first);
    expect(lastConn()).toMatchObject({ state: 'connected' });
    expect(second.sent.map((e) => e.body.case)).toEqual(['hello', 'auth', 'subscribe']);
    expect(second.sent[2]!.body.value).toMatchObject({ symbols: ['EURUSD'] });
    api.disconnect();
  });

  it('places a market order with a request id, returns the fill price and tracks the position', async () => {
    const { api, events } = setup();
    await api.connect();
    await api.getSymbols();
    const r = await api.placeOrder({ accountId: 'DEMO-1', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 25, clientId: 'req-1' });
    expect(r).toEqual({ ok: true, orderId: 'req-1', positionId: 'DEMO-1:EURUSD', price: 1.08503 });
    const place = FakeWs.last!.sent.find((e) => e.body.case === 'placeOrder')!;
    expect(place.body.value).toMatchObject({ requestId: 'req-1', accountId: 'DEMO-1', symbol: 'EURUSD', side: Side.BUY });
    // 0.25 lot of EURUSD = 25,000 EUR on the wire.
    expect(place.body.value).toMatchObject({ qty: { value: 25000n, scale: 0 } });
    const last = events.filter((e) => e.type === 'positions').at(-1);
    expect(last).toMatchObject({ positions: [{ id: 'DEMO-1:EURUSD', side: 'buy', volume: 25, openPrice: 1.08503 }] });
    expect(events.some((e) => e.type === 'deal' && e.deal.volume === 25)).toBe(true);
    api.disconnect();
  });

  it('maps a request-scoped Error to a failed result', async () => {
    const { api } = setup();
    await api.connect();
    FakeWs.server = (env, ws) => {
      if (env.body.case === 'cancelOrder')
        ws.push({ case: 'error', value: { requestId: env.body.value.requestId, code: ErrorCode.UNKNOWN_ORDER, message: 'nope' } });
    };
    expect(await api.cancelOrder('DEMO-1', 'x')).toEqual({ ok: false, error: 'UNKNOWN_ORDER: nope' });
    api.disconnect();
  });

  it('measures latency with Ping/Pong and reconnects when the link goes silent', async () => {
    const { api, events } = setup();
    await api.connect();
    FakeWs.server = (env, ws) => {
      if (env.body.case === 'ping') ws.push({ case: 'pong', value: { nonce: env.body.value.nonce } });
      else gateway(env, ws);
    };
    await vi.advanceTimersByTimeAsync(5_000);
    expect(events.some((e) => e.type === 'connection' && e.latencyMs !== undefined)).toBe(true);
    const ws = FakeWs.last!;
    // Nothing arrives any more (pings swallowed, no heartbeats).
    FakeWs.server = (env, w) => {
      if (env.body.case !== 'ping') gateway(env, w);
    };
    await vi.advanceTimersByTimeAsync(36_000);
    expect(ws.readyState).toBe(3);
    expect(FakeWs.last).not.toBe(ws);
    api.disconnect();
  });
});

/** v1.2 gateway: account info, position ids, native order / position commands, history. */
function gatewayV12(): Server & { deals: number } {
  const deals = 5;
  const srv: Server = (env, ws) => {
    const b = env.body;
    switch (b.case) {
      case 'auth':
        ws.push({
          case: 'authOk',
          value: { subject: 'u', accountIds: ['DEMO-H1'], accounts: [{ accountId: 'DEMO-H1', currency: 'USD', marginMode: MarginMode.HEDGING, leverage: 30, group: 'g' }] },
        });
        ws.push({
          case: 'accountSnapshot',
          value: {
            accountId: 'DEMO-H1',
            currency: 'USD',
            balance: D('10000'),
            marginMode: MarginMode.HEDGING,
            leverage: 30,
            positions: [
              { symbol: 'EURUSD', positionId: '11', side: Side.BUY, netQty: D('100000'), qty: D('100000'), avgPrice: D('1.1'), sl: D('1.09'), tp: D('1.12'), trailingDistance: D('0.002'), openTimeNs: 5_000_000n },
              { symbol: 'EURUSD', positionId: '12', side: Side.SELL, netQty: D('-50000'), qty: D('50000'), avgPrice: D('1.1002') },
            ],
          },
        });
        return;
      case 'orderListRequest':
        ws.push({
          case: 'orderList',
          value: {
            requestId: b.value.requestId,
            accountId: b.value.accountId,
            orders: [
              {
                accountId: 'DEMO-H1', clientRequestId: 'sl1', orderId: '20', symbol: 'EURUSD', side: Side.BUY, status: OrderStatus.NEW,
                orderType: OrderType.STOP_LIMIT, qty: D('10000'), leavesQty: D('10000'), filledQty: D('0'), stopPrice: D('1.11'), limitPrice: D('1.111'),
                ocoGroup: 3n, expireAtNs: 9_000_000_000n, createdNs: 1_000_000n, stopTriggered: true,
              },
            ],
          },
        });
        return;
      case 'placeOrder':
      case 'modifyPosition':
      case 'modifyOrder':
        ws.push({ case: 'ack', value: { requestId: b.value.requestId } });
        return;
      case 'closePosition': {
        const c = b.value;
        ws.push({ case: 'ack', value: { requestId: c.requestId } });
        ws.push({
          case: 'orderUpdate',
          value: {
            accountId: c.accountId, clientRequestId: c.requestId, orderId: '30', symbol: 'EURUSD', side: Side.SELL, status: OrderStatus.FILLED,
            filledQty: c.qty, leavesQty: D('0'), avgPrice: D('1.10100'), closePositionId: c.positionId, orderType: OrderType.MARKET,
          },
        });
        ws.push({
          case: 'dealUpdate',
          value: {
            accountId: c.accountId,
            deal: { dealId: '99', positionId: c.positionId, symbol: 'EURUSD', side: Side.SELL, entry: DealEntry.OUT, qty: c.qty, price: D('1.101'), realizedPnl: D('50'), commission: D('-1.75'), tsNs: 7_000_000n, reason: DealReason.CLIENT },
          },
        });
        return;
      }
      case 'dealHistoryRequest': {
        const r = b.value;
        const start = r.cursor ? Number(r.cursor) : 0;
        const ids = Array.from({ length: deals }, (_, i) => i + 1).filter((i) => i > start).slice(0, Math.min(r.limit, 2)); // server page cap
        const last = ids[ids.length - 1]!;
        ws.push({
          case: 'dealHistory',
          value: {
            requestId: r.requestId,
            accountId: r.accountId,
            nextCursor: last < deals ? String(last) : '',
            deals: ids.map((i) => ({
              dealId: String(i), positionId: '11', symbol: 'EURUSD', side: Side.BUY, entry: i % 2 ? DealEntry.IN : DealEntry.OUT, qty: D('10000'),
              price: D('1.1'), realizedPnl: D(i % 2 ? '0' : '12.34'), commission: D('-0.35'), tsNs: BigInt(i) * 1_000_000n,
              reason: i === 4 ? DealReason.STOP_LOSS : DealReason.CLIENT,
            })),
          },
        });
        return;
      }
      default:
        gateway(env, ws);
    }
  };
  return Object.assign(srv, { deals });
}

describe('WsTradingApi with a v1.2 gateway', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'setInterval', 'clearTimeout', 'clearInterval'] });
    FakeWs.last = undefined;
    FakeWs.server = gatewayV12();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const sent = (c: string) => FakeWs.last!.sent.filter((e) => e.body.case === c).map((e) => e.body.value);

  it('maps account info, hedged positions with protection and the pending order list', async () => {
    const { api, events } = setup();
    await api.connect();
    expect(await api.getAccounts()).toEqual([
      { id: 'DEMO-H1', name: 'DEMO-H1', currency: 'USD', balance: 1_000_000, leverage: 30, isDemo: true, marginMode: 'hedging' },
    ]);
    const pos = events.filter((e) => e.type === 'positions').at(-1);
    expect(pos).toMatchObject({
      positions: [
        { id: '11', side: 'buy', volume: 100, openPrice: 1.1, sl: 1.09, tp: 1.12, trailing: 0.002, openTime: 5 },
        { id: '12', side: 'sell', volume: 50, openPrice: 1.1002, sl: undefined, tp: undefined },
      ],
    });
    const orders = events.filter((e) => e.type === 'orders').at(-1);
    expect(orders).toMatchObject({
      orders: [{ id: 'sl1', type: 'stop_limit', price: 1.11, limitPrice: 1.111, volume: 10, ocoGroup: 3, expiry: 9_000, createdAt: 1, triggered: true }],
    });
    api.disconnect();
  });

  it('sends stop / stop-limit orders with protection, OCO and GTD expiry', async () => {
    const { api } = setup();
    await api.connect();
    await api.getSymbols();
    const r = await api.placeOrder({
      accountId: 'DEMO-H1', symbol: 'EURUSD', side: 'buy', type: 'stop_limit', volume: 10, price: 1.11, limitPrice: 1.1105,
      sl: 1.1, tp: 1.13, trailing: 0.0015, ocoGroup: 4, expiry: 2_000, clientId: 'p1',
    });
    expect(r).toEqual({ ok: true, orderId: 'p1' });
    expect(sent('placeOrder')[0]).toMatchObject({
      requestId: 'p1', orderType: OrderType.STOP_LIMIT, stopPrice: { value: 111n, scale: 2 }, limitPrice: { value: 11105n, scale: 4 },
      sl: { value: 11n, scale: 1 }, tp: { value: 113n, scale: 2 }, trailingDistance: { value: 15n, scale: 4 },
      ocoGroup: 4n, expireAtNs: 2_000_000_000n, tif: TimeInForce.GTD, qty: { value: 10000n, scale: 0 },
    });
    await api.placeOrder({ accountId: 'DEMO-H1', symbol: 'EURUSD', side: 'sell', type: 'stop', volume: 10, price: 1.09, clientId: 'p2' });
    expect(sent('placeOrder')[1]).toMatchObject({ orderType: OrderType.STOP, stopPrice: { value: 109n, scale: 2 } });
    expect(sent('placeOrder')[1]).not.toHaveProperty('limitPrice');
    api.disconnect();
  });

  it('modifies positions and pending orders natively and closes partially', async () => {
    const { api, events } = setup();
    await api.connect();
    await api.getSymbols();
    expect(await api.modifyPosition('DEMO-H1', '11', 1.095, undefined, 0.002)).toEqual({ ok: true, positionId: '11' });
    expect(sent('modifyPosition')[0]).toMatchObject({ positionId: '11', sl: { value: 1095n, scale: 3 }, trailingDistance: { value: 2n, scale: 3 } });
    expect(sent('modifyPosition')[0]).not.toHaveProperty('tp');
    expect(await api.modifyOrder('DEMO-H1', 'sl1', { price: 1.112, volume: 20, tp: 1.2 })).toEqual({ ok: true, orderId: 'sl1' });
    // stop-limit: `price` is the stop; protection is replaced as a whole (sl kept from the order: none)
    expect(sent('modifyOrder')[0]).toMatchObject({
      targetRequestId: 'sl1', stopPrice: { value: 1112n, scale: 3 }, qty: { value: 20000n, scale: 0 },
      replaceProtection: true, tp: { value: 12n, scale: 1 }, clearExpiry: false,
    });
    expect(sent('modifyOrder')[0]).not.toHaveProperty('sl');
    expect(sent('modifyOrder')[0]).not.toHaveProperty('limitPrice');
    await api.modifyOrder('DEMO-H1', 'sl1', { expiry: undefined });
    expect(sent('modifyOrder')[1]).toMatchObject({ clearExpiry: true, replaceProtection: false });

    const r = await api.closePosition('DEMO-H1', '11', 50);
    expect(r).toEqual({ ok: true, orderId: expect.any(String), positionId: '11', price: 1.101 });
    expect(sent('closePosition')[0]).toMatchObject({ positionId: '11', qty: { value: 50000n, scale: 0 } });
    // full close sends no qty
    void api.closePosition('DEMO-H1', '12');
    expect(sent('closePosition')[1]).toMatchObject({ positionId: '12' });
    expect(sent('closePosition')[1]).not.toHaveProperty('qty');
    const deal = events.find((e) => e.type === 'deal');
    expect(deal).toMatchObject({ deal: { id: '99', positionId: '11', entry: 'out', volume: 50, price: 1.101, profit: 5000, commission: -175, reason: 'client', time: 7 } });
    api.disconnect();
  });

  it('loads the deal history page by page', async () => {
    const { api } = setup();
    await api.connect();
    await api.getSymbols();
    const h = await api.getHistory('DEMO-H1');
    expect(h.map((d) => d.id)).toEqual(['1', '2', '3', '4', '5']);
    expect(h[1]).toMatchObject({ entry: 'out', profit: 1234, commission: -35, volume: 10 });
    expect(h[3]!.reason).toBe('sl');
    expect(sent('dealHistoryRequest').map((r) => (r as { cursor: string }).cursor)).toEqual(['', '2', '4']);
    api.disconnect();
  });
});
