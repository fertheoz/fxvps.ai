import type {
  Account,
  Bar,
  Deal,
  Depth,
  OrderRequest,
  OrderResult,
  PendingOrder,
  Quote,
  SymbolSpec,
  Timeframe,
  TradingApi,
  TradingEvent,
  Unsubscribe,
} from './types';

/**
 * WebSocket adapter for the future fxvps.ai gateway (price-edge + client-api).
 * Wire protocol: JSON text frames, documented in apps/terminal/PROTOCOL.md.
 * This is a thin, untested-against-a-real-server stub: request/response
 * correlation, subscriptions and reconnect with backoff are implemented.
 */

type ServerMessage =
  | { type: 'response'; id: number; ok: true; result: unknown }
  | { type: 'response'; id: number; ok: false; error: { code: string; message: string } }
  | { type: 'quotes'; quotes: Quote[] }
  | { type: 'depth'; depth: Depth }
  | { type: 'event'; event: TradingEvent }
  | { type: 'pong'; t: number };

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
}

export interface WsOptions {
  url: string;
  /** Bearer token obtained from OIDC; sent in the `hello` frame, never in the URL. */
  token?: () => string | undefined;
  WebSocketImpl?: typeof WebSocket;
}

export class WsTradingApi implements TradingApi {
  private ws: WebSocket | undefined;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private listeners = new Set<(e: TradingEvent) => void>();
  private quoteSubs = new Set<{ symbols: Set<string>; cb: (q: Quote[]) => void }>();
  private depthSubs = new Set<{ symbol: string; cb: (d: Depth) => void }>();
  private closedByUser = false;
  private attempt = 0;
  private pingTimer: ReturnType<typeof setInterval> | undefined;

  constructor(private readonly opts: WsOptions) {}

  connect(): Promise<void> {
    this.closedByUser = false;
    this.emit({ type: 'connection', state: this.attempt ? 'reconnecting' : 'connecting' });
    const Impl = this.opts.WebSocketImpl ?? WebSocket;
    return new Promise((resolve, reject) => {
      const ws = new Impl(this.opts.url);
      this.ws = ws;
      ws.onopen = async () => {
        try {
          await this.request('hello', { protocol: 1, token: this.opts.token?.() });
          this.attempt = 0;
          this.emit({ type: 'connection', state: 'connected' });
          this.resubscribe();
          this.startPing();
          resolve();
        } catch (e) {
          reject(e as Error);
        }
      };
      ws.onmessage = (ev) => this.onMessage(String(ev.data));
      ws.onerror = () => reject(new Error('WebSocket error'));
      ws.onclose = () => {
        clearInterval(this.pingTimer);
        for (const p of this.pending.values()) p.reject(new Error('Connection closed'));
        this.pending.clear();
        if (this.closedByUser) {
          this.emit({ type: 'connection', state: 'disconnected' });
          return;
        }
        this.emit({ type: 'connection', state: 'reconnecting' });
        const backoff = Math.min(30_000, 500 * 2 ** this.attempt++);
        setTimeout(() => void this.connect().catch(() => undefined), backoff);
      };
    });
  }

  disconnect(): void {
    this.closedByUser = true;
    this.ws?.close();
  }

  getAccounts(): Promise<Account[]> {
    return this.request('accounts.list', {});
  }
  getSymbols(): Promise<SymbolSpec[]> {
    return this.request('symbols.list', {});
  }
  getBars(symbol: string, timeframe: Timeframe, count: number): Promise<Bar[]> {
    return this.request('bars.get', { symbol, timeframe, count });
  }
  getHistory(accountId: string): Promise<Deal[]> {
    return this.request('history.deals', { accountId });
  }

  subscribeQuotes(symbols: string[], onQuotes: (quotes: Quote[]) => void): Unsubscribe {
    const sub = { symbols: new Set(symbols), cb: onQuotes };
    this.quoteSubs.add(sub);
    this.send({ type: 'subscribe', channel: 'quotes', symbols });
    return () => {
      this.quoteSubs.delete(sub);
      this.send({ type: 'unsubscribe', channel: 'quotes', symbols });
    };
  }

  subscribeDepth(symbol: string, onDepth: (depth: Depth) => void): Unsubscribe {
    const sub = { symbol, cb: onDepth };
    this.depthSubs.add(sub);
    this.send({ type: 'subscribe', channel: 'depth', symbols: [symbol] });
    return () => {
      this.depthSubs.delete(sub);
      this.send({ type: 'unsubscribe', channel: 'depth', symbols: [symbol] });
    };
  }

  onEvent(listener: (event: TradingEvent) => void): Unsubscribe {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  placeOrder(req: OrderRequest): Promise<OrderResult> {
    return this.trade('order.place', { ...req, clientId: req.clientId ?? crypto.randomUUID() });
  }
  modifyPosition(accountId: string, positionId: string, sl?: number, tp?: number): Promise<OrderResult> {
    return this.trade('position.modify', { accountId, positionId, sl, tp });
  }
  closePosition(accountId: string, positionId: string, volume?: number): Promise<OrderResult> {
    return this.trade('position.close', { accountId, positionId, volume });
  }
  modifyOrder(
    accountId: string,
    orderId: string,
    changes: Partial<Pick<PendingOrder, 'price' | 'limitPrice' | 'sl' | 'tp' | 'expiry'>>,
  ): Promise<OrderResult> {
    return this.trade('order.modify', { accountId, orderId, ...changes });
  }
  cancelOrder(accountId: string, orderId: string): Promise<OrderResult> {
    return this.trade('order.cancel', { accountId, orderId });
  }

  // ---- internals ---------------------------------------------------------
  private async trade(method: string, params: object): Promise<OrderResult> {
    try {
      return await this.request<OrderResult>(method, params);
    } catch (e) {
      return { ok: false, error: (e as Error).message };
    }
  }

  private request<T>(method: string, params: object): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      if (!this.send({ type: 'request', id, method, params })) {
        this.pending.delete(id);
        reject(new Error('Not connected'));
      }
    });
  }

  private send(msg: object): boolean {
    if (!this.ws || this.ws.readyState !== 1) return false;
    this.ws.send(JSON.stringify(msg));
    return true;
  }

  private onMessage(raw: string): void {
    let msg: ServerMessage;
    try {
      msg = JSON.parse(raw) as ServerMessage;
    } catch {
      return;
    }
    switch (msg.type) {
      case 'response': {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        if (msg.ok) p.resolve(msg.result);
        else p.reject(new Error(`${msg.error.code}: ${msg.error.message}`));
        return;
      }
      case 'quotes':
        for (const s of this.quoteSubs) {
          const qs = msg.quotes.filter((q) => s.symbols.has(q.symbol));
          if (qs.length) s.cb(qs);
        }
        return;
      case 'depth':
        for (const s of this.depthSubs) if (s.symbol === msg.depth.symbol) s.cb(msg.depth);
        return;
      case 'event':
        this.emit(msg.event);
        return;
      case 'pong':
        this.emit({ type: 'connection', state: 'connected', latencyMs: Date.now() - msg.t });
        return;
    }
  }

  private resubscribe(): void {
    const q = new Set<string>();
    for (const s of this.quoteSubs) s.symbols.forEach((x) => q.add(x));
    if (q.size) this.send({ type: 'subscribe', channel: 'quotes', symbols: [...q] });
    for (const s of this.depthSubs) this.send({ type: 'subscribe', channel: 'depth', symbols: [s.symbol] });
  }

  private startPing(): void {
    clearInterval(this.pingTimer);
    this.pingTimer = setInterval(() => this.send({ type: 'ping', t: Date.now() }), 5000);
  }

  private emit(e: TradingEvent): void {
    for (const l of this.listeners) l(e);
  }
}
