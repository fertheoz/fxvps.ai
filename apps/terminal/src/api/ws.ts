import Big from 'big.js';
import { create, fromBinary, toBinary, type MessageInitShape } from '@bufbuild/protobuf';
import {
  DealEntry,
  DealReason,
  EnvelopeSchema,
  ErrorCode,
  MarginMode as WireMarginMode,
  OrderStatus,
  OrderType as WireOrderType,
  Side as WireSide,
  TimeInForce,
  Timeframe as WireTimeframe,
  type AccountInfo,
  type AccountSnapshot,
  type Deal as WireDeal,
  type Envelope,
  type Instrument,
  type OrderUpdate,
  type Position as WirePosition,
  type Quote as WireQuote,
} from './gen/fxvps_client_v1_pb';
import { decimalToBig, decimalToString, toDecimal } from './decimal';
import { MOCK_SYMBOLS, toSpec } from '@fxvps/trading-core';
import type {
  Account,
  Bar,
  ConnectionState,
  Deal,
  Depth,
  OrderChanges,
  OrderRequest,
  OrderResult,
  PendingOrder,
  Position,
  Quote,
  SymbolSpec,
  Timeframe,
  TradingApi,
  TradingEvent,
  Unsubscribe,
} from '@fxvps/trading-core';

/**
 * Adapter for services/client-gateway. Wire protocol: binary protobuf frames,
 * schema crates/client-proto/proto/fxvps_client_v1.proto (TS types generated
 * into ./gen with `pnpm proto:gen`), semantics in crates/client-proto/PROTOCOL.md.
 *
 * Unit mapping (terminal <-> wire):
 *  - volume: centi-lots <-> base units (volume * contractSize / 100)
 *  - money:  minor units (cents) <-> Decimal major units
 *  - prices: numbers rounded to `digits` <-> Decimal (exact, via big.js)
 * Positions are keyed by the server `position_id` (protocol v1.2; hedging
 * accounts hold several per symbol). Pre-v1.2 gateways report one net position
 * per symbol without an id: those get `<account>:<symbol>`.
 */

export const PROTOCOL_VERSION = 1;
const PING_MS = 5_000;
/** No frame at all for this long (server heartbeats every 15 s) = dead link. */
const IDLE_TIMEOUT_MS = 35_000;
const REQUEST_TIMEOUT_MS = 10_000;
const FILL_TIMEOUT_MS = 10_000;

type Body = MessageInitShape<typeof EnvelopeSchema>['body'];
type InBody = Envelope['body'];

export interface WsOptions {
  url: string;
  /** JWT bearer token; sent in the `Auth` frame, never in the URL. */
  token?: () => string | undefined;
  WebSocketImpl?: typeof WebSocket;
  clientName?: string;
  /** Requested max quote batches per second (0 = server default). */
  maxQuoteHz?: number;
}

interface Pending {
  resolve: (body: InBody) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface FillWaiter {
  resolve: (u: OrderUpdate | undefined) => void;
  timer: ReturnType<typeof setTimeout>;
}

const TF_TO_WIRE: Record<Timeframe, WireTimeframe> = {
  M1: WireTimeframe.M1,
  M5: WireTimeframe.M5,
  M15: WireTimeframe.M15,
  M30: WireTimeframe.M30,
  H1: WireTimeframe.H1,
  H4: WireTimeframe.H4,
  D1: WireTimeframe.D1,
  // Not offered by the gateway: fall back to daily candles.
  W1: WireTimeframe.D1,
  MN: WireTimeframe.D1,
};

const WIRE_TYPE: Record<OrderRequest['type'], WireOrderType> = {
  market: WireOrderType.MARKET,
  limit: WireOrderType.LIMIT,
  stop: WireOrderType.STOP,
  stop_limit: WireOrderType.STOP_LIMIT,
};
const FROM_WIRE_TYPE: Partial<Record<WireOrderType, PendingOrder['type']>> = {
  [WireOrderType.LIMIT]: 'limit',
  [WireOrderType.STOP]: 'stop',
  [WireOrderType.STOP_LIMIT]: 'stop_limit',
};
const DEAL_REASON: Record<DealReason, Deal['reason']> = {
  [DealReason.DEAL_REASON_UNSPECIFIED]: 'client',
  [DealReason.CLIENT]: 'client',
  [DealReason.STOP_LOSS]: 'sl',
  [DealReason.TAKE_PROFIT]: 'tp',
  [DealReason.STOP_OUT]: 'stop_out',
};
const HISTORY_PAGE = 1000;
const HISTORY_MAX_PAGES = 50;

const nsToMs = (ns: bigint): number => Number(ns / 1_000_000n);
const msToNs = (ms: number): bigint => BigInt(Math.round(ms)) * 1_000_000n;
/** Money (major units) -> integer minor units. */
const minor = (d: Parameters<typeof decimalToBig>[0]): number => Number((decimalToBig(d) ?? new Big(0)).times(100).round(0).toFixed(0));

export function marginModeFromWire(m: WireMarginMode): Account['marginMode'] {
  return m === WireMarginMode.HEDGING ? 'hedging' : m === WireMarginMode.NETTING ? 'netting' : undefined;
}

const TERMINAL_STATUSES = new Set([OrderStatus.FILLED, OrderStatus.CANCELED, OrderStatus.REJECTED, OrderStatus.EXPIRED]);
const RESTING_STATUSES = new Set([OrderStatus.PENDING_NEW, OrderStatus.NEW, OrderStatus.PARTIALLY_FILLED, OrderStatus.REPLACED]);

function defaultContractSize(base: string, quote: string): string {
  if (base === 'XAU') return '100';
  if (base === 'XAG') return '5000';
  return base.length === 3 && quote.length === 3 ? '100000' : '1';
}

/** Wire Instrument -> terminal SymbolSpec (missing fields get FX defaults). */
export function instrumentToSpec(i: Instrument): SymbolSpec {
  const base = i.base || i.symbol.slice(0, 3);
  const quote = i.quote || i.symbol.slice(3, 6);
  const digits = i.tickSize ? i.digits : quote === 'JPY' ? 3 : 5;
  const tick = decimalToBig(i.tickSize) ?? new Big(1).div(new Big(10).pow(digits));
  const metal = base === 'XAU' || base === 'XAG';
  const fxLike = !metal && base.length === 3 && quote.length === 3;
  // FX convention: 5/3-digit quotes have a fractional pip (pip = 10 ticks).
  const pip = fxLike && (digits === 5 || digits === 3) ? tick.times(10) : tick;
  const contractSize = Number(decimalToString(i.contractSize) ?? defaultContractSize(base, quote));
  // qty_step (base units) -> centi-lots, at least 1.
  const stepUnits = decimalToBig(i.qtyStep);
  const step = stepUnits ? Math.max(1, Number(stepUnits.times(100).div(contractSize).round(0, Big.roundUp).toFixed(0))) : 1;
  return {
    name: i.symbol,
    description: `${base}/${quote}`,
    base,
    quote,
    digits,
    pipSize: pip.toFixed(),
    contractSize,
    minVolume: step,
    maxVolume: 10_000,
    volumeStep: step,
    marginRate: 1,
    category: metal ? 'metal' : fxLike ? 'fx' : 'index',
  };
}

const randomId = (): string =>
  typeof crypto !== 'undefined' && 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;

const errorText = (code: ErrorCode, message: string) => `${ErrorCode[code] ?? code}${message ? `: ${message}` : ''}`;

export class WsTradingApi implements TradingApi {
  private ws: WebSocket | undefined;
  private seq = 0n;
  private nextReq = 1;
  private pending = new Map<string, Pending>();
  private fillWaiters = new Map<string, FillWaiter>();
  private handshakeWaiter: { pred: (b: InBody) => boolean; resolve: (b: InBody) => void } | undefined;
  private listeners = new Set<(e: TradingEvent) => void>();
  private quoteSubs = new Set<{ symbols: Set<string>; cb: (q: Quote[]) => void }>();
  private depthSubs = new Set<{ symbol: string; cb: (d: Depth) => void }>();
  private closedByUser = false;
  private attempt = 0;
  private pingTimer: ReturnType<typeof setInterval> | undefined;
  private idleTimer: ReturnType<typeof setTimeout> | undefined;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  private pingSent = new Map<bigint, number>();
  private pingNonce = 0n;
  private state: ConnectionState = 'disconnected';
  /** Last connection-level (no request id) gateway error, reported on close. */
  private lastError: string | undefined;
  /** Token sent in the last Auth frame (to tell an expired one from a bad one). */
  private sentToken = '';

  private accountIds: string[] = [];
  private accounts = new Map<string, Account>();
  private infos = new Map<string, AccountInfo>();
  /** Server speaks protocol v1.2 (position ids, deals, native modify). */
  private v12 = false;
  private snapshotsPending = new Set<string>();
  private resolveSnapshots: () => void = () => undefined;
  private symbols = new Map<string, SymbolSpec>();
  private positions = new Map<string, Map<string, Position>>();
  private orders = new Map<string, Map<string, PendingOrder>>();
  /** PlaceOrder request_id -> what was asked for (classifies OrderUpdates of pre-v1.2 gateways). */
  private placed = new Map<string, { type: OrderRequest['type']; price?: number }>();
  private day = new Map<string, { open: number; high: number; low: number }>();
  private dealSeq = 0;

  constructor(private readonly opts: WsOptions) {}

  connect(): Promise<void> {
    this.closedByUser = false;
    clearTimeout(this.reconnectTimer);
    this.setState(this.attempt ? 'reconnecting' : 'connecting');
    const Impl = this.opts.WebSocketImpl ?? WebSocket;
    return new Promise<void>((resolve, reject) => {
      let settled = false;
      const fail = (e: Error) => {
        if (!settled) {
          settled = true;
          reject(e);
        }
      };
      let ws: WebSocket;
      try {
        ws = new Impl(this.opts.url);
      } catch (e) {
        fail(e as Error);
        this.setState('disconnected');
        return;
      }
      ws.binaryType = 'arraybuffer';
      this.ws = ws;
      this.seq = 0n;
      ws.onopen = () => {
        this.handshake()
          .then(() => {
            this.attempt = 0;
            this.setState('connected');
            this.resubscribe();
            this.startPing();
            settled = true;
            resolve();
          })
          .catch((e: Error) => {
            this.emitJournal('error', `Gateway handshake failed: ${e.message}`);
            fail(e);
            ws.close();
          });
      };
      ws.onmessage = (ev: MessageEvent) => this.onFrame(ev.data);
      ws.onerror = () => fail(new Error(`Cannot connect to ${this.opts.url}`));
      ws.onclose = (ev: CloseEvent) => {
        if (this.ws !== ws) return;
        this.ws = undefined;
        this.handshakeWaiter = undefined;
        clearInterval(this.pingTimer);
        clearTimeout(this.idleTimer);
        for (const p of this.pending.values()) {
          clearTimeout(p.timer);
          p.reject(new Error('Connection closed'));
        }
        this.pending.clear();
        fail(new Error(this.lastError ?? `Connection closed (${ev.code}${ev.reason ? ` ${ev.reason}` : ''})`));
        this.lastError = undefined;
        if (this.closedByUser) {
          this.setState('disconnected');
          return;
        }
        // Expired session token: the identity session has refreshed it by now, so reconnect with the new one.
        if (ev.code === 4001 && this.sentToken && this.opts.token?.() && this.opts.token() !== this.sentToken) {
          this.emitJournal('info', 'Session token renewed; reconnecting');
          this.scheduleReconnect();
          return;
        }
        // Bad credentials will not get better by retrying.
        if (ev.code === 4001) {
          this.emitJournal('error', 'Gateway rejected the token (4001)');
          this.setState('disconnected');
          return;
        }
        this.emitJournal('warn', `Gateway connection lost (${ev.code}); reconnecting`);
        this.scheduleReconnect();
      };
    });
  }

  disconnect(): void {
    this.closedByUser = true;
    clearTimeout(this.reconnectTimer);
    this.ws?.close(1000);
  }

  getAccounts(): Promise<Account[]> {
    return Promise.resolve(this.accountIds.map((id) => this.accounts.get(id)).filter((a): a is Account => !!a));
  }

  async getSymbols(): Promise<SymbolSpec[]> {
    let specs: SymbolSpec[];
    try {
      const body = await this.request((requestId) => ({ case: 'symbolListRequest', value: { requestId } }));
      if (body.case !== 'symbolList') throw new Error('unexpected reply');
      specs = body.value.instruments.map(instrumentToSpec);
    } catch (e) {
      // Older gateways do not know SymbolListRequest: fall back to the built-in list.
      this.emitJournal('warn', `Symbol list unavailable (${(e as Error).message}); using defaults`);
      specs = MOCK_SYMBOLS.map(toSpec);
    }
    this.symbols = new Map(specs.map((s) => [s.name, s]));
    return specs;
  }

  async getBars(symbol: string, timeframe: Timeframe, count: number): Promise<Bar[]> {
    try {
      const body = await this.request((requestId) => ({
        case: 'candleRequest',
        value: { requestId, symbol, timeframe: TF_TO_WIRE[timeframe], limit: count },
      }));
      if (body.case !== 'candleResponse') return [];
      const digits = this.digits(symbol);
      const px = (d: Parameters<typeof decimalToBig>[0]) => Number((decimalToBig(d) ?? new Big(0)).toFixed(digits));
      return body.value.candles.map((c) => ({
        time: Number(c.openTimeNs / 1_000_000_000n),
        open: px(c.open),
        high: px(c.high),
        low: px(c.low),
        close: px(c.close),
        volume: Number(c.ticks),
      }));
    } catch {
      // Unknown symbol / not connected: the chart just shows no history.
      return [];
    }
  }

  /** Deal history from the server (protocol v1.2), all pages, oldest first. */
  async getHistory(accountId: string): Promise<Deal[]> {
    if (!this.v12) return [];
    const out: Deal[] = [];
    let cursor = '';
    try {
      for (let page = 0; page < HISTORY_MAX_PAGES; page++) {
        const body = await this.request((requestId) => ({
          case: 'dealHistoryRequest',
          value: { requestId, accountId, limit: HISTORY_PAGE, cursor },
        }));
        if (body.case !== 'dealHistory') break;
        for (const d of body.value.deals) out.push(this.dealFromWire(accountId, d));
        cursor = body.value.nextCursor;
        if (!cursor) break;
      }
    } catch (e) {
      this.emitJournal('warn', `History unavailable: ${(e as Error).message}`);
    }
    return out;
  }

  subscribeQuotes(symbols: string[], onQuotes: (quotes: Quote[]) => void): Unsubscribe {
    const sub = { symbols: new Set(symbols), cb: onQuotes };
    this.quoteSubs.add(sub);
    this.sendSubscribe(symbols);
    return () => {
      this.quoteSubs.delete(sub);
      this.dropUnused(symbols);
    };
  }

  /** The gateway serves top of book only: depth is one level per side, built from quotes. */
  subscribeDepth(symbol: string, onDepth: (depth: Depth) => void): Unsubscribe {
    const sub = { symbol, cb: onDepth };
    this.depthSubs.add(sub);
    this.sendSubscribe([symbol]);
    return () => {
      this.depthSubs.delete(sub);
      this.dropUnused([symbol]);
    };
  }

  onEvent(listener: (event: TradingEvent) => void): Unsubscribe {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async placeOrder(req: OrderRequest): Promise<OrderResult> {
    const spec = this.symbols.get(req.symbol);
    if (!spec) return { ok: false, error: `Unknown symbol ${req.symbol}` };
    const pending = req.type !== 'market';
    if (pending && req.price === undefined) return { ok: false, error: 'Price required' };
    if (req.type === 'stop_limit' && req.limitPrice === undefined) return { ok: false, error: 'Limit price required' };
    if (!this.v12 && (req.type === 'stop' || req.type === 'stop_limit')) return { ok: false, error: `${req.type} orders need a v1.2 gateway` };
    const requestId = req.clientId ?? randomId();
    this.placed.set(requestId, { type: req.type, price: req.price });
    const fill = pending ? undefined : this.waitFill(requestId);
    // Terminal price semantics: `price` = limit price (limit) or stop price (stop, stop-limit).
    const limit = req.type === 'limit' ? req.price : req.type === 'stop_limit' ? req.limitPrice : undefined;
    const stop = req.type === 'stop' || req.type === 'stop_limit' ? req.price : undefined;
    const opt = (v: number | undefined) => (v === undefined ? undefined : toDecimal(v));
    try {
      await this.request(
        () => ({
          case: 'placeOrder',
          value: {
            requestId,
            accountId: req.accountId,
            symbol: req.symbol,
            side: req.side === 'buy' ? WireSide.BUY : WireSide.SELL,
            orderType: WIRE_TYPE[req.type],
            qty: this.volumeToQty(spec, req.volume),
            limitPrice: opt(limit),
            stopPrice: opt(stop),
            sl: opt(req.sl),
            tp: opt(req.tp),
            trailingDistance: opt(req.trailing),
            ocoGroup: pending && req.ocoGroup ? BigInt(req.ocoGroup) : 0n,
            expireAtNs: pending && req.expiry !== undefined ? msToNs(req.expiry) : 0n,
            tif: pending && req.expiry !== undefined ? TimeInForce.GTD : TimeInForce.TIME_IN_FORCE_UNSPECIFIED,
          },
        }),
        requestId,
      );
    } catch (e) {
      this.cancelFillWait(requestId);
      this.placed.delete(requestId);
      return { ok: false, error: (e as Error).message };
    }
    if (!fill) return { ok: true, orderId: requestId };
    return this.fillResult(req.accountId, req.symbol, requestId, await fill);
  }

  /** Market execution outcome from the terminal OrderUpdate (undefined = no report in time). */
  private fillResult(accountId: string, symbol: string, requestId: string, u: OrderUpdate | undefined, positionId?: string): OrderResult {
    // Accepted but no execution report in time: report acceptance, events will follow.
    if (!u) return { ok: true, orderId: requestId };
    const filled = decimalToBig(u.filledQty);
    if (filled && filled.gt(0)) {
      const avg = decimalToBig(u.avgPrice);
      return {
        ok: true,
        orderId: requestId,
        positionId: positionId ?? (u.positionId || u.closePositionId || this.positionId(accountId, symbol)),
        price: avg ? Number(avg.toFixed(this.digits(symbol))) : undefined,
      };
    }
    return { ok: false, error: u.text || OrderStatus[u.status] || 'not filled' };
  }

  async modifyPosition(accountId: string, positionId: string, sl?: number, tp?: number, trailing?: number): Promise<OrderResult> {
    if (!this.v12) return { ok: false, error: 'SL/TP need a v1.2 gateway' };
    const p = this.positions.get(accountId)?.get(positionId);
    if (!p) return { ok: false, error: 'Unknown position' };
    try {
      await this.request((requestId) => ({
        case: 'modifyPosition',
        value: {
          requestId,
          accountId,
          positionId,
          sl: sl === undefined ? undefined : toDecimal(sl),
          tp: tp === undefined ? undefined : toDecimal(tp),
          trailingDistance: trailing === undefined ? undefined : toDecimal(trailing),
        },
      }));
      return { ok: true, positionId };
    } catch (e) {
      return { ok: false, error: (e as Error).message };
    }
  }

  async closePosition(accountId: string, positionId: string, volume?: number): Promise<OrderResult> {
    const p = this.positions.get(accountId)?.get(positionId);
    if (!p) return { ok: false, error: 'Unknown position' };
    const vol = Math.min(volume ?? p.volume, p.volume);
    if (!this.v12) {
      // Pre-v1.2: net positions, closed by an opposite market order.
      return this.placeOrder({ accountId, symbol: p.symbol, side: p.side === 'buy' ? 'sell' : 'buy', type: 'market', volume: vol });
    }
    const spec = this.symbols.get(p.symbol);
    if (!spec) return { ok: false, error: `Unknown symbol ${p.symbol}` };
    const requestId = randomId();
    const fill = this.waitFill(requestId);
    try {
      await this.request(
        () => ({
          case: 'closePosition',
          value: { requestId, accountId, positionId, qty: vol >= p.volume ? undefined : this.volumeToQty(spec, vol) },
        }),
        requestId,
      );
    } catch (e) {
      this.cancelFillWait(requestId);
      return { ok: false, error: (e as Error).message };
    }
    return this.fillResult(accountId, p.symbol, requestId, await fill, positionId);
  }

  async modifyOrder(accountId: string, orderId: string, changes: OrderChanges): Promise<OrderResult> {
    const o = this.orders.get(accountId)?.get(orderId);
    if (!o) return { ok: false, error: 'Unknown order' };
    const spec = this.symbols.get(o.symbol);
    if (!spec) return { ok: false, error: `Unknown symbol ${o.symbol}` };
    if (!this.v12 && Object.keys(changes).some((k) => k !== 'price' && k !== 'volume')) {
      return { ok: false, error: 'Only price and volume can be modified on this gateway' };
    }
    // `price` is the limit price of limits and the stop price of stops / stop-limits.
    const limit = o.type === 'limit' ? changes.price : o.type === 'stop_limit' ? changes.limitPrice : undefined;
    const stop = o.type !== 'limit' ? changes.price : undefined;
    const prot = 'sl' in changes || 'tp' in changes || 'trailing' in changes;
    const merged = { sl: o.sl, tp: o.tp, trailing: o.trailing, ...changes };
    const opt = (v: number | undefined) => (v === undefined ? undefined : toDecimal(v));
    try {
      await this.request((requestId) => ({
        case: 'modifyOrder',
        value: {
          requestId,
          accountId,
          targetRequestId: orderId,
          qty: changes.volume === undefined ? undefined : this.volumeToQty(spec, changes.volume),
          limitPrice: opt(limit),
          stopPrice: opt(stop),
          replaceProtection: prot,
          sl: prot ? opt(merged.sl) : undefined,
          tp: prot ? opt(merged.tp) : undefined,
          trailingDistance: prot ? opt(merged.trailing) : undefined,
          expireAtNs: changes.expiry !== undefined ? msToNs(changes.expiry) : 0n,
          clearExpiry: 'expiry' in changes && changes.expiry === undefined,
        },
      }));
      return { ok: true, orderId };
    } catch (e) {
      return { ok: false, error: (e as Error).message };
    }
  }

  async cancelOrder(accountId: string, orderId: string): Promise<OrderResult> {
    try {
      await this.request((requestId) => ({ case: 'cancelOrder', value: { requestId, accountId, targetRequestId: orderId } }));
      return { ok: true, orderId };
    } catch (e) {
      return { ok: false, error: (e as Error).message };
    }
  }

  // ---- handshake & transport --------------------------------------------
  private async handshake(): Promise<void> {
    const hello = this.awaitBody((b) => b.case === 'hello' || b.case === 'error');
    this.sendBody({
      case: 'hello',
      value: { protocolVersion: PROTOCOL_VERSION, clientName: this.opts.clientName ?? 'fxvps-terminal', maxQuoteHz: this.opts.maxQuoteHz ?? 0 },
    });
    const h = await hello;
    if (h.case === 'error') throw new Error(errorText(h.value.code, h.value.message));
    // One AccountSnapshot per authorized account follows AuthOk (possibly in the same
    // task): track them from the moment AuthOk is decoded, see onFrame().
    const snapshots = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('account snapshot timeout')), REQUEST_TIMEOUT_MS);
      this.resolveSnapshots = () => {
        clearTimeout(timer);
        resolve();
      };
    });
    snapshots.catch(() => undefined);
    const authOk = this.awaitBody((b) => b.case === 'authOk' || b.case === 'error');
    this.sentToken = this.opts.token?.() ?? '';
    this.sendBody({ case: 'auth', value: { token: this.sentToken } });
    const a = await authOk;
    if (a.case === 'error') throw new Error(errorText(a.value.code, a.value.message));
    if (a.case !== 'authOk') throw new Error('unexpected handshake reply');
    await snapshots;
    if (this.v12) await Promise.all(this.accountIds.map((id) => this.loadOrders(id)));
  }

  /** Working orders from the server (authoritative after a (re)connect). */
  private async loadOrders(accountId: string): Promise<void> {
    try {
      const body = await this.request((requestId) => ({ case: 'orderListRequest', value: { requestId, accountId } }));
      if (body.case !== 'orderList') return;
      const book = new Map<string, PendingOrder>();
      for (const u of body.value.orders) {
        const o = this.pendingFromUpdate(u);
        if (o) book.set(o.id, o);
      }
      this.orders.set(accountId, book);
      this.emit({ type: 'orders', accountId, orders: [...book.values()] });
    } catch (e) {
      this.emitJournal('warn', `Order list unavailable: ${(e as Error).message}`);
    }
  }

  private awaitBody(pred: (b: InBody) => boolean): Promise<InBody> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.handshakeWaiter = undefined;
        reject(new Error('handshake timeout'));
      }, REQUEST_TIMEOUT_MS);
      this.handshakeWaiter = {
        pred,
        resolve: (b) => {
          clearTimeout(timer);
          resolve(b);
        },
      };
    });
  }

  private reqId(): string {
    return `t${this.nextReq++}`;
  }

  /** Sends a request and resolves with the correlated reply (Ack or typed response); Error rejects. */
  private request(build: (requestId: string) => Body, requestId = this.reqId()): Promise<InBody> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error('Request timed out'));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(requestId, { resolve, reject, timer });
      if (!this.sendBody(build(requestId))) {
        clearTimeout(timer);
        this.pending.delete(requestId);
        reject(new Error('Not connected'));
      }
    });
  }

  private sendBody(body: Body): boolean {
    const ws = this.ws;
    if (!ws || ws.readyState !== 1) return false;
    this.seq += 1n;
    ws.send(toBinary(EnvelopeSchema, create(EnvelopeSchema, { version: PROTOCOL_VERSION, seq: this.seq, body })));
    return true;
  }

  private sendSubscribe(symbols: string[]): void {
    if (this.state !== 'connected') return; // resubscribe() covers it on (re)connect
    const known = this.symbols.size ? symbols.filter((s) => this.symbols.has(s)) : symbols;
    if (!known.length) return;
    // The Ack/Error is correlated; failures are only journaled.
    this.request((requestId) => ({ case: 'subscribe', value: { requestId, symbols: known } })).catch((e: Error) =>
      this.emitJournal('warn', `Subscribe failed: ${e.message}`),
    );
  }

  private dropUnused(symbols: string[]): void {
    const still = new Set<string>();
    for (const s of this.quoteSubs) s.symbols.forEach((x) => still.add(x));
    for (const s of this.depthSubs) still.add(s.symbol);
    const drop = symbols.filter((s) => !still.has(s));
    if (drop.length) this.sendBody({ case: 'unsubscribe', value: { requestId: this.reqId(), symbols: drop } });
  }

  private resubscribe(): void {
    const all = new Set<string>();
    for (const s of this.quoteSubs) s.symbols.forEach((x) => all.add(x));
    for (const s of this.depthSubs) all.add(s.symbol);
    this.sendSubscribe([...all]);
  }

  private scheduleReconnect(): void {
    this.setState('reconnecting');
    const backoff = Math.min(30_000, 500 * 2 ** this.attempt++);
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => void this.connect().catch(() => undefined), backoff);
  }

  private startPing(): void {
    clearInterval(this.pingTimer);
    this.touch();
    this.pingTimer = setInterval(() => {
      const nonce = ++this.pingNonce;
      this.pingSent.set(nonce, Date.now());
      for (const k of this.pingSent.keys()) if (k < nonce - 16n) this.pingSent.delete(k);
      this.sendBody({ case: 'ping', value: { nonce, tsNs: BigInt(Date.now()) * 1_000_000n } });
    }, PING_MS);
  }

  /** Liveness: any inbound frame (quotes, heartbeats, pongs) resets the idle timer. */
  private touch(): void {
    clearTimeout(this.idleTimer);
    this.idleTimer = setTimeout(() => {
      this.emitJournal('warn', 'Gateway silent; reconnecting');
      this.ws?.close(4000, 'idle');
    }, IDLE_TIMEOUT_MS);
  }

  private onFrame(data: unknown): void {
    this.touch();
    let env: Envelope;
    try {
      // Binary connection: the server answers in protobuf; text frames are not expected.
      const bytes =
        data instanceof ArrayBuffer
          ? new Uint8Array(data)
          : ArrayBuffer.isView(data)
            ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
            : undefined;
      if (!bytes) return;
      env = fromBinary(EnvelopeSchema, bytes);
    } catch {
      return;
    }
    const b = env.body;
    if (b.case === 'authOk') {
      this.accountIds = b.value.accountIds;
      this.v12 = b.value.accounts.length > 0;
      this.infos = new Map(b.value.accounts.map((i) => [i.accountId, i]));
      this.snapshotsPending = new Set(this.accountIds);
      if (!this.snapshotsPending.size) this.resolveSnapshots();
    } else if (b.case === 'error' && !b.value.requestId) {
      this.lastError = errorText(b.value.code, b.value.message);
    }
    if (this.handshakeWaiter?.pred(b)) {
      const w = this.handshakeWaiter;
      this.handshakeWaiter = undefined;
      w.resolve(b);
      return;
    }
    switch (b.case) {
      case 'ack':
      case 'symbolList':
      case 'candleResponse':
      case 'orderList':
      case 'dealHistory':
        this.settle(b.value.requestId, b);
        return;
      case 'error': {
        const p = this.pending.get(b.value.requestId);
        const text = errorText(b.value.code, b.value.message);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(b.value.requestId);
          p.reject(new Error(text));
        } else this.emitJournal('error', `Gateway error ${text}`);
        return;
      }
      case 'quoteBatch':
        this.onQuotes(b.value.quotes);
        return;
      case 'accountSnapshot':
        this.onSnapshot(b.value);
        return;
      case 'positionUpdate':
        if (b.value.position) {
          this.setPosition(b.value.accountId, b.value.position);
          this.emitPositions(b.value.accountId);
        }
        return;
      case 'orderUpdate':
        this.onOrderUpdate(b.value);
        return;
      case 'dealUpdate':
        if (b.value.deal) this.emit({ type: 'deal', deal: this.dealFromWire(b.value.accountId, b.value.deal) });
        return;
      case 'pong': {
        const sent = this.pingSent.get(b.value.nonce);
        if (sent !== undefined) {
          this.pingSent.delete(b.value.nonce);
          this.emit({ type: 'connection', state: 'connected', latencyMs: Date.now() - sent });
        }
        return;
      }
      default:
        // Heartbeat and unknown/future variants: nothing to do (liveness via touch()).
        return;
    }
  }

  private settle(requestId: string, body: InBody): void {
    const p = this.pending.get(requestId);
    if (!p) return;
    clearTimeout(p.timer);
    this.pending.delete(requestId);
    p.resolve(body);
  }

  // ---- state mapping ------------------------------------------------------
  private digits(symbol: string): number {
    return this.symbols.get(symbol)?.digits ?? 5;
  }

  private volumeToQty(spec: SymbolSpec, volume: number) {
    return toDecimal(new Big(volume).times(spec.contractSize).div(100));
  }

  private price(symbol: string, d: Parameters<typeof decimalToBig>[0]): number | undefined {
    const v = decimalToBig(d);
    return v ? Number(v.toFixed(this.digits(symbol))) : undefined;
  }

  private dealFromWire(accountId: string, d: WireDeal): Deal {
    return {
      id: d.dealId,
      accountId,
      positionId: d.positionId,
      symbol: d.symbol,
      side: d.side === WireSide.SELL ? 'sell' : 'buy',
      entry: d.entry === DealEntry.OUT ? 'out' : 'in',
      volume: this.qtyToVolume(d.symbol, decimalToBig(d.qty) ?? new Big(0)),
      price: this.price(d.symbol, d.price) ?? 0,
      time: d.tsNs ? nsToMs(d.tsNs) : Date.now(),
      profit: minor(d.realizedPnl),
      commission: minor(d.commission),
      reason: DEAL_REASON[d.reason as DealReason] ?? 'client',
    };
  }

  private qtyToVolume(symbol: string, qty: Big): number {
    const cs = this.symbols.get(symbol)?.contractSize ?? 100_000;
    return Number(qty.times(100).div(cs).round(0).toFixed(0));
  }

  private positionId(accountId: string, symbol: string): string {
    return `${accountId}:${symbol}`;
  }

  private onQuotes(wire: WireQuote[]): void {
    const out: Quote[] = [];
    for (const q of wire) {
      const bidB = decimalToBig(q.bid);
      const askB = decimalToBig(q.ask);
      if (!bidB || !askB) continue;
      const digits = this.digits(q.symbol);
      const bid = Number(bidB.toFixed(digits));
      const ask = Number(askB.toFixed(digits));
      let d = this.day.get(q.symbol);
      if (!d) {
        d = { open: bid, high: bid, low: bid };
        this.day.set(q.symbol, d);
      }
      d.high = Math.max(d.high, bid);
      d.low = Math.min(d.low, bid);
      const time = q.tsNs ? Number(q.tsNs / 1_000_000n) : Date.now();
      out.push({ symbol: q.symbol, bid, ask, time, dayOpen: d.open, dayHigh: d.high, dayLow: d.low });
      for (const s of this.depthSubs) {
        if (s.symbol !== q.symbol) continue;
        const size = (x: WireQuote['bidSize']) => {
          const v = decimalToBig(x);
          return v ? this.qtyToVolume(q.symbol, v) : 0;
        };
        s.cb({ symbol: q.symbol, bids: [{ price: bid, volume: size(q.bidSize) }], asks: [{ price: ask, volume: size(q.askSize) }], time });
      }
    }
    if (!out.length) return;
    for (const s of this.quoteSubs) {
      const qs = out.filter((q) => s.symbols.has(q.symbol));
      if (qs.length) s.cb(qs);
    }
  }

  private onSnapshot(s: AccountSnapshot): void {
    const balance = Number((decimalToBig(s.balance) ?? new Big(0)).times(100).round(0).toFixed(0));
    const info = this.infos.get(s.accountId);
    const account: Account = {
      id: s.accountId,
      name: s.accountId,
      currency: s.currency || info?.currency || 'USD',
      balance,
      leverage: s.leverage || info?.leverage || 100,
      isDemo: /demo/i.test(s.accountId),
      marginMode: marginModeFromWire(s.marginMode) ?? marginModeFromWire(info?.marginMode ?? WireMarginMode.MARGIN_MODE_UNSPECIFIED),
    };
    const known = this.accounts.has(s.accountId);
    this.accounts.set(s.accountId, account);
    if (known) this.emit({ type: 'account', account });
    // A snapshot is authoritative (also after a reconnect): replace positions.
    this.positions.set(s.accountId, new Map());
    for (const p of s.positions) this.setPosition(s.accountId, p);
    this.emitPositions(s.accountId);
    if (this.snapshotsPending.delete(s.accountId) && !this.snapshotsPending.size) this.resolveSnapshots();
  }

  private setPosition(accountId: string, p: WirePosition): void {
    let m = this.positions.get(accountId);
    if (!m) {
      m = new Map();
      this.positions.set(accountId, m);
    }
    const id = p.positionId || this.positionId(accountId, p.symbol);
    const net = decimalToBig(p.netQty) ?? new Big(0);
    if (net.eq(0)) {
      m.delete(id);
      return;
    }
    const prev = m.get(id);
    const side = net.gt(0) ? 'buy' : 'sell';
    m.set(id, {
      id,
      accountId,
      symbol: p.symbol,
      side,
      volume: this.qtyToVolume(p.symbol, decimalToBig(p.qty) ?? net.abs()),
      openPrice: this.price(p.symbol, p.avgPrice) ?? 0,
      openTime: p.openTimeNs ? nsToMs(p.openTimeNs) : prev && prev.side === side ? prev.openTime : Date.now(),
      sl: this.price(p.symbol, p.sl),
      tp: this.price(p.symbol, p.tp),
      trailing: decimalToBig(p.trailingDistance)?.toNumber(),
      commission: 0,
      swap: 0,
    });
  }

  private emitPositions(accountId: string): void {
    this.emit({ type: 'positions', accountId, positions: [...(this.positions.get(accountId)?.values() ?? [])] });
  }

  /** Pending-order view of an OrderUpdate, or undefined if it is not resting. */
  private pendingFromUpdate(u: OrderUpdate): PendingOrder | undefined {
    const placed = this.placed.get(u.clientRequestId);
    const type = FROM_WIRE_TYPE[u.orderType] ?? (placed && placed.type !== 'market' ? placed.type : undefined);
    const leaves = decimalToBig(u.leavesQty);
    if (!type || !RESTING_STATUSES.has(u.status) || !leaves || !leaves.gt(0)) return undefined;
    const prev = this.orders.get(u.accountId)?.get(u.clientRequestId);
    const limit = this.price(u.symbol, u.limitPrice);
    const stop = this.price(u.symbol, u.stopPrice);
    const price = type === 'limit' ? limit : stop;
    return {
      id: u.clientRequestId,
      accountId: u.accountId,
      symbol: u.symbol,
      side: u.side === WireSide.SELL ? 'sell' : 'buy',
      type,
      volume: this.qtyToVolume(u.symbol, leaves),
      price: price ?? placed?.price ?? prev?.price ?? 0,
      limitPrice: type === 'stop_limit' ? limit : undefined,
      sl: this.price(u.symbol, u.sl),
      tp: this.price(u.symbol, u.tp),
      trailing: decimalToBig(u.trailingDistance)?.toNumber(),
      ocoGroup: u.ocoGroup ? Number(u.ocoGroup) : undefined,
      expiry: u.expireAtNs ? nsToMs(u.expireAtNs) : undefined,
      createdAt: u.createdNs ? nsToMs(u.createdNs) : (prev?.createdAt ?? Date.now()),
      triggered: u.stopTriggered || undefined,
    };
  }

  private onOrderUpdate(u: OrderUpdate): void {
    const side = u.side === WireSide.SELL ? 'sell' : 'buy';
    const lastQty = decimalToBig(u.lastQty);
    const lastPx = decimalToBig(u.lastPrice);
    // v1.2 gateways push real deals (DealUpdate, with P&L); older ones only fills.
    if (!this.v12 && lastQty && lastPx && lastQty.gt(0)) {
      this.emit({
        type: 'deal',
        deal: {
          id: `${u.orderId || u.clientRequestId}-${++this.dealSeq}`,
          accountId: u.accountId,
          positionId: this.positionId(u.accountId, u.symbol),
          symbol: u.symbol,
          side,
          entry: 'in',
          volume: this.qtyToVolume(u.symbol, lastQty),
          price: Number(lastPx.toFixed(this.digits(u.symbol))),
          time: u.tsNs ? nsToMs(u.tsNs) : Date.now(),
          profit: 0,
          commission: 0,
          reason: 'client',
        },
      });
    }
    // Resting orders are shown as pending orders, keyed by PlaceOrder request_id.
    let book = this.orders.get(u.accountId);
    if (!book) {
      book = new Map();
      this.orders.set(u.accountId, book);
    }
    const pending = this.pendingFromUpdate(u);
    if (pending) {
      book.set(u.clientRequestId, pending);
      this.emit({ type: 'orders', accountId: u.accountId, orders: [...book.values()] });
    } else if (book.delete(u.clientRequestId)) {
      this.emit({ type: 'orders', accountId: u.accountId, orders: [...book.values()] });
    }
    if (u.status === OrderStatus.REJECTED) this.emitJournal('error', `Order ${u.clientRequestId} rejected: ${u.text}`);
    if (u.status === OrderStatus.EXPIRED) this.emitJournal('info', `Order ${u.clientRequestId} expired`);
    if (TERMINAL_STATUSES.has(u.status)) {
      this.placed.delete(u.clientRequestId);
      const w = this.fillWaiters.get(u.clientRequestId);
      if (w) {
        clearTimeout(w.timer);
        this.fillWaiters.delete(u.clientRequestId);
        w.resolve(u);
      }
    }
  }

  private waitFill(requestId: string): Promise<OrderUpdate | undefined> {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.fillWaiters.delete(requestId);
        resolve(undefined);
      }, FILL_TIMEOUT_MS);
      this.fillWaiters.set(requestId, { resolve, timer });
    });
  }

  private cancelFillWait(requestId: string): void {
    const w = this.fillWaiters.get(requestId);
    if (w) clearTimeout(w.timer);
    this.fillWaiters.delete(requestId);
  }

  private setState(s: ConnectionState): void {
    this.state = s;
    this.emit({ type: 'connection', state: s });
  }

  private emitJournal(level: 'info' | 'warn' | 'error', message: string): void {
    this.emit({ type: 'journal', entry: { id: randomId(), time: Date.now(), level, message } });
  }

  private emit(e: TradingEvent): void {
    for (const l of this.listeners) l(e);
  }
}
