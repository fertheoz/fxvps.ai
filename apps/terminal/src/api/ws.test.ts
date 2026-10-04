import { create, fromBinary, toBinary, type MessageInitShape } from '@bufbuild/protobuf';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { toDecimal } from './decimal';
import { EnvelopeSchema, ErrorCode, OrderStatus, Side, type Envelope } from './gen/fxvps_client_v1_pb';
import type { Quote, TradingEvent } from './types';
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
      if (b.value.token !== 'good') {
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
