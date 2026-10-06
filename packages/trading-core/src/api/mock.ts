import type {
  Account,
  Bar,
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
  OrderHistoryEntry,
} from './types';
import { TIMEFRAME_SECONDS } from './types';
import { MOCK_SYMBOLS, toSpec } from './symbols';
import {
  buildRates,
  closePrice,
  commissionMinor,
  computeAccountMetrics,
  marginMinor,
  openPrice,
  profitMinor,
} from '../lib/money';
import { validateTicket } from '../lib/validation';
import { gaussian, hashString, mulberry32 } from '../lib/random';
import { bucketStart } from '../lib/bars';

export interface MockOptions {
  seed?: number;
  /** Simulated request latency in ms (0 in tests). */
  latencyMs?: number;
  /** Tick interval in ms; 0 disables the timer (drive with `tick()` manually). */
  tickIntervalMs?: number;
  now?: () => number;
}

interface SymState {
  spec: SymbolSpec;
  /** Mid price in integer points. */
  midPts: number;
  spreadPts: number;
  volPts: number;
  quote: Quote;
}

const STOP_OUT_LEVEL = 50; // percent

/**
 * Fully in-browser trading backend: random-walk quotes, order matching,
 * pending order triggers, SL/TP, stop-out, hedging accounts and sub-accounts.
 */
export class MockTradingApi implements TradingApi {
  private rand: () => number;
  private readonly latency: number;
  private readonly tickInterval: number;
  private readonly now: () => number;
  private syms = new Map<string, SymState>();
  private accounts = new Map<string, Account>();
  private positions = new Map<string, Position[]>();
  private orders = new Map<string, PendingOrder[]>();
  private deals = new Map<string, Deal[]>();
  private prefs = new Map<string, string>();
  private listeners = new Set<(e: TradingEvent) => void>();
  private quoteSubs = new Set<{ symbols: Set<string>; cb: (q: Quote[]) => void }>();
  private depthSubs = new Set<{ symbol: string; cb: (d: Depth) => void }>();
  private timer: ReturnType<typeof setInterval> | undefined;
  private depthTimer: ReturnType<typeof setInterval> | undefined;
  private seq = 1000;

  constructor(opts: MockOptions = {}) {
    this.rand = mulberry32(opts.seed ?? 42);
    this.latency = opts.latencyMs ?? 40;
    this.tickInterval = opts.tickIntervalMs ?? 120;
    this.now = opts.now ?? Date.now;
    for (const s of MOCK_SYMBOLS) {
      const spec = toSpec(s);
      const scale = 10 ** s.digits;
      const midPts = Math.round(s.start * scale);
      const dayOpenPts = midPts + Math.round(gaussian(this.rand) * s.volPoints * 40);
      const st: SymState = {
        spec,
        midPts,
        spreadPts: s.spreadPoints,
        volPts: s.volPoints,
        quote: { symbol: s.name, bid: 0, ask: 0, time: 0, dayOpen: dayOpenPts / scale, dayHigh: 0, dayLow: 0 },
      };
      this.refreshQuote(st);
      st.quote.dayHigh = Math.max(st.quote.dayOpen, st.quote.bid);
      st.quote.dayLow = Math.min(st.quote.dayOpen, st.quote.bid);
      this.syms.set(s.name, st);
    }
    const mk = (a: Account) => {
      this.accounts.set(a.id, a);
      this.positions.set(a.id, []);
      this.orders.set(a.id, []);
      this.deals.set(a.id, []);
    };
    mk({ id: '100001', name: 'Main', currency: 'USD', balance: 10_000_000, leverage: 100, isDemo: true, marginMode: 'hedging' });
    mk({ id: '100002', name: 'Scalping', parentId: '100001', currency: 'USD', balance: 1_000_000, leverage: 200, isDemo: true, marginMode: 'hedging' });
    mk({ id: '100003', name: 'Swing', parentId: '100001', currency: 'USD', balance: 2_500_000, leverage: 30, isDemo: true, marginMode: 'hedging' });
  }

  // ---- lifecycle -------------------------------------------------------
  async connect(): Promise<void> {
    this.emit({ type: 'connection', state: 'connecting' });
    await this.delay();
    this.emit({ type: 'connection', state: 'connected', latencyMs: this.latency });
    this.journal('info', 'Connected to mock trade server (demo)');
    if (this.tickInterval > 0 && !this.timer) {
      this.timer = setInterval(() => this.tick(), this.tickInterval);
      this.depthTimer = setInterval(() => this.pushDepth(), 300);
    }
  }

  disconnect(): void {
    clearInterval(this.timer);
    clearInterval(this.depthTimer);
    this.timer = undefined;
    this.depthTimer = undefined;
    this.emit({ type: 'connection', state: 'disconnected' });
  }

  // ---- reference data --------------------------------------------------
  async getAccounts(): Promise<Account[]> {
    await this.delay();
    return [...this.accounts.values()].map((a) => ({ ...a }));
  }

  async getSymbols(): Promise<SymbolSpec[]> {
    await this.delay();
    return [...this.syms.values()].map((s) => ({ ...s.spec }));
  }

  async getHistory(accountId: string): Promise<Deal[]> {
    await this.delay();
    return [...(this.deals.get(accountId) ?? [])];
  }

  async getPrefs(accountId: string): Promise<string | null> {
    await this.delay();
    return this.prefs.get(accountId) ?? null;
  }

  async setPrefs(accountId: string, json: string): Promise<void> {
    await this.delay();
    this.prefs.set(accountId, json);
  }

  /** The simulator keeps no order history: every deal stands for one filled market order. */
  async getOrderHistory(accountId: string): Promise<OrderHistoryEntry[]> {
    await this.delay();
    return [...(this.deals.get(accountId) ?? [])].reverse().map((d) => ({
      id: d.id,
      accountId,
      symbol: d.symbol,
      side: d.side,
      type: 'market' as const,
      volume: d.volume,
      filled: d.volume,
      avgPrice: d.price,
      status: 'filled' as const,
      time: d.time,
    }));
  }

  async getBars(symbol: string, timeframe: Timeframe, count: number): Promise<Bar[]> {
    await this.delay();
    const st = this.syms.get(symbol);
    if (!st) return [];
    const tf = TIMEFRAME_SECONDS[timeframe];
    const rnd = mulberry32(hashString(symbol + timeframe));
    const scale = 10 ** st.spec.digits;
    const stepVol = st.volPts * Math.sqrt(tf / 2) * 0.6;
    const bars: Bar[] = [];
    let close = st.midPts;
    const lastStart = bucketStart(Math.floor(this.now() / 1000), tf);
    // Walk backwards from the current price so the history joins the live feed.
    for (let i = 0; i < count; i++) {
      const open = Math.max(1, Math.round(close - gaussian(rnd) * stepVol));
      const hi = Math.max(open, close) + Math.round(Math.abs(gaussian(rnd)) * stepVol * 0.5);
      const lo = Math.max(1, Math.min(open, close) - Math.round(Math.abs(gaussian(rnd)) * stepVol * 0.5));
      bars.push({
        time: lastStart - i * tf,
        open: open / scale,
        high: hi / scale,
        low: lo / scale,
        close: close / scale,
        volume: Math.round(50 + rnd() * 500 * Math.sqrt(tf / 60)),
      });
      close = open;
    }
    return bars.reverse();
  }

  // ---- streaming -------------------------------------------------------
  subscribeQuotes(symbols: string[], onQuotes: (quotes: Quote[]) => void): Unsubscribe {
    const sub = { symbols: new Set(symbols), cb: onQuotes };
    this.quoteSubs.add(sub);
    onQuotes(symbols.map((s) => this.syms.get(s)?.quote).filter((q): q is Quote => !!q).map((q) => ({ ...q })));
    return () => this.quoteSubs.delete(sub);
  }

  subscribeDepth(symbol: string, onDepth: (depth: Depth) => void): Unsubscribe {
    const sub = { symbol, cb: onDepth };
    this.depthSubs.add(sub);
    onDepth(this.makeDepth(symbol));
    return () => this.depthSubs.delete(sub);
  }

  onEvent(listener: (event: TradingEvent) => void): Unsubscribe {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  // ---- trading ---------------------------------------------------------
  async placeOrder(req: OrderRequest): Promise<OrderResult> {
    await this.delay();
    const acc = this.accounts.get(req.accountId);
    const st = this.syms.get(req.symbol);
    if (!acc || !st) return this.reject('Unknown account or symbol');
    const metrics = this.metrics(acc.id);
    const rates = this.rates();
    const entry = openPrice(req.side, st.quote);
    const requiredMargin =
      req.type === 'market' ? marginMinor(st.spec, req.volume, entry, acc.leverage, acc.currency, rates) : undefined;
    const errors = validateTicket(
      { ...req },
      { spec: st.spec, quote: st.quote, now: this.now(), requiredMargin, freeMargin: metrics.freeMargin },
    );
    if (errors.length) return this.reject(`Order rejected: ${errors.map((e) => e.key).join(', ')}`);

    if (req.type === 'market') {
      const pos = this.openPosition(acc, req, entry, 'client');
      return { ok: true, positionId: pos.id, price: pos.openPrice };
    }
    const order: PendingOrder = {
      id: this.id(),
      accountId: acc.id,
      symbol: req.symbol,
      side: req.side,
      type: req.type,
      volume: req.volume,
      price: req.price!,
      limitPrice: req.limitPrice,
      sl: req.sl,
      tp: req.tp,
      trailing: req.trailing,
      ocoGroup: req.ocoGroup,
      expiry: req.expiry,
      createdAt: this.now(),
    };
    this.orders.get(acc.id)!.push(order);
    this.journal('info', `#${order.id} ${req.side} ${req.type} ${req.volume / 100} ${req.symbol} at ${req.price} placed`);
    this.emitOrders(acc.id);
    return { ok: true, orderId: order.id };
  }

  async modifyPosition(accountId: string, positionId: string, sl?: number, tp?: number, trailing?: number): Promise<OrderResult> {
    await this.delay();
    const pos = this.positions.get(accountId)?.find((p) => p.id === positionId);
    if (!pos) return this.reject('Position not found');
    const q = this.syms.get(pos.symbol)!.quote;
    const cp = closePrice(pos.side, q);
    const buy = pos.side === 'buy';
    if (sl !== undefined && (buy ? sl >= cp : sl <= cp)) return this.reject('Invalid SL');
    if (tp !== undefined && (buy ? tp <= cp : tp >= cp)) return this.reject('Invalid TP');
    if (trailing !== undefined && !(trailing > 0)) return this.reject('Invalid trailing distance');
    pos.sl = sl;
    pos.tp = tp;
    pos.trailing = trailing;
    this.journal('info', `#${pos.id} modified sl: ${sl ?? '-'} tp: ${tp ?? '-'}${trailing ? ` trailing: ${trailing}` : ''}`);
    this.emitPositions(accountId);
    return { ok: true, positionId };
  }

  async closePosition(accountId: string, positionId: string, volume?: number): Promise<OrderResult> {
    await this.delay();
    const pos = this.positions.get(accountId)?.find((p) => p.id === positionId);
    if (!pos) return this.reject('Position not found');
    const spec = this.syms.get(pos.symbol)!.spec;
    const vol = volume ?? pos.volume;
    if (vol <= 0 || vol > pos.volume || vol % spec.volumeStep !== 0) return this.reject('Invalid close volume');
    const price = this.closeVolume(pos, vol, 'client');
    return { ok: true, positionId, price };
  }

  async modifyOrder(accountId: string, orderId: string, changes: OrderChanges): Promise<OrderResult> {
    await this.delay();
    const o = this.orders.get(accountId)?.find((x) => x.id === orderId);
    if (!o) return this.reject('Order not found');
    const spec = this.syms.get(o.symbol)!.spec;
    const v = changes.volume;
    if (v !== undefined && (v < spec.minVolume || v > spec.maxVolume || v % spec.volumeStep !== 0)) return this.reject('Invalid volume');
    if ('price' in changes && !(changes.price! > 0)) return this.reject('Invalid price');
    Object.assign(o, changes);
    if (o.volume === undefined) o.volume = v ?? spec.minVolume;
    this.emitOrders(accountId);
    return { ok: true, orderId };
  }

  async cancelOrder(accountId: string, orderId: string): Promise<OrderResult> {
    await this.delay();
    const list = this.orders.get(accountId);
    const idx = list?.findIndex((o) => o.id === orderId) ?? -1;
    if (!list || idx < 0) return this.reject('Order not found');
    list.splice(idx, 1);
    this.journal('info', `#${orderId} cancelled`);
    this.emitOrders(accountId);
    return { ok: true, orderId };
  }

  // ---- simulation ------------------------------------------------------
  /** Advance the random walk for a few symbols and run the matching engine. */
  tick(symbols?: string[]): void {
    const all = [...this.syms.values()];
    const chosen = symbols
      ? all.filter((s) => symbols.includes(s.spec.name))
      : all.filter(() => this.rand() < 0.45);
    if (!chosen.length) return;
    for (const st of chosen) {
      const drift = Math.round(gaussian(this.rand) * st.volPts);
      st.midPts = Math.max(st.spreadPts * 4, st.midPts + drift);
      this.refreshQuote(st);
    }
    for (const sub of this.quoteSubs) {
      const qs = chosen.filter((s) => sub.symbols.has(s.spec.name)).map((s) => ({ ...s.quote }));
      if (qs.length) sub.cb(qs);
    }
    for (const acc of this.accounts.values()) this.match(acc, new Set(chosen.map((c) => c.spec.name)));
  }

  /** Test helper: set an exact mid price (in price units). */
  setMid(symbol: string, price: number): void {
    const st = this.syms.get(symbol)!;
    st.midPts = Math.round(price * 10 ** st.spec.digits);
    this.refreshQuote(st);
    for (const sub of this.quoteSubs) if (sub.symbols.has(symbol)) sub.cb([{ ...st.quote }]);
    for (const acc of this.accounts.values()) this.match(acc, new Set([symbol]));
  }

  getQuote(symbol: string): Quote | undefined {
    const q = this.syms.get(symbol)?.quote;
    return q ? { ...q } : undefined;
  }

  // ---- internals -------------------------------------------------------
  private refreshQuote(st: SymState): void {
    const scale = 10 ** st.spec.digits;
    const half = Math.floor(st.spreadPts / 2);
    const bidPts = st.midPts - half;
    const askPts = bidPts + st.spreadPts;
    const q = st.quote;
    q.bid = bidPts / scale;
    q.ask = askPts / scale;
    q.time = this.now();
    q.dayHigh = Math.max(q.dayHigh || q.bid, q.bid);
    q.dayLow = Math.min(q.dayLow || q.bid, q.bid);
  }

  private match(acc: Account, changed: Set<string>): void {
    const now = this.now();
    const orders = this.orders.get(acc.id)!;
    let ordersChanged = false;
    for (const o of [...orders]) {
      if (!changed.has(o.symbol) || !orders.includes(o)) continue; // gone (OCO sibling)
      if (o.expiry !== undefined && o.expiry <= now) {
        orders.splice(orders.indexOf(o), 1);
        this.journal('info', `#${o.id} expired`);
        ordersChanged = true;
        continue;
      }
      const q = this.syms.get(o.symbol)!.quote;
      const px = o.side === 'buy' ? q.ask : q.bid;
      const buy = o.side === 'buy';
      if (o.type === 'stop_limit' && !o.triggered) {
        if (buy ? px >= o.price : px <= o.price) {
          o.triggered = true;
          this.journal('info', `#${o.id} stop-limit triggered, limit ${o.limitPrice}`);
          ordersChanged = true;
        }
        continue;
      }
      const limitLike = o.type === 'limit' || o.type === 'stop_limit';
      const target = o.type === 'stop_limit' ? o.limitPrice! : o.price;
      const hit = limitLike ? (buy ? px <= target : px >= target) : buy ? px >= target : px <= target;
      if (!hit) continue;
      orders.splice(orders.indexOf(o), 1);
      ordersChanged = true;
      const rates = this.rates();
      const req = marginMinor(this.syms.get(o.symbol)!.spec, o.volume, px, acc.leverage, acc.currency, rates);
      if (req > this.metrics(acc.id).freeMargin) {
        this.journal('warn', `#${o.id} cancelled: not enough money`);
        continue;
      }
      // Limits fill at the better of market and limit price; stops at market.
      const fill = limitLike ? (buy ? Math.min(px, target) : Math.max(px, target)) : px;
      this.openPosition(acc, o, fill, 'order');
      if (o.ocoGroup) {
        for (const x of [...orders]) {
          if (x.ocoGroup !== o.ocoGroup) continue;
          orders.splice(orders.indexOf(x), 1);
          this.journal('info', `#${x.id} cancelled (OCO with #${o.id})`);
        }
      }
    }
    if (ordersChanged) this.emitOrders(acc.id);

    const positions = this.positions.get(acc.id)!;
    for (const p of [...positions]) {
      if (!changed.has(p.symbol)) continue;
      const q = this.syms.get(p.symbol)!.quote;
      const cp = closePrice(p.side, q);
      const buy = p.side === 'buy';
      if (p.trailing) {
        // Server-side style trailing stop: follows once `trailing` in profit, never moves back.
        const digits = this.syms.get(p.symbol)!.spec.digits;
        const cand = Number((buy ? cp - p.trailing : cp + p.trailing).toFixed(digits));
        const inProfit = buy ? cand > p.openPrice : cand < p.openPrice;
        const better = p.sl === undefined || (buy ? cand > p.sl : cand < p.sl);
        if (inProfit && better) {
          p.sl = cand;
          this.emitPositions(acc.id);
        }
      }
      if (p.sl !== undefined && (buy ? cp <= p.sl : cp >= p.sl)) this.closeVolume(p, p.volume, 'sl');
      else if (p.tp !== undefined && (buy ? cp >= p.tp : cp <= p.tp)) this.closeVolume(p, p.volume, 'tp');
    }
    if (positions.length) {
      // Stop-out: close the worst position while margin level < STOP_OUT_LEVEL.
      for (let guard = 0; guard < positions.length + 1; guard++) {
        const m = this.metrics(acc.id);
        if (m.marginLevel === null || Number(m.marginLevel) >= STOP_OUT_LEVEL) break;
        const rates = this.rates();
        const worst = [...positions].sort(
          (a, b) => this.pnl(a, acc, rates) - this.pnl(b, acc, rates),
        )[0];
        if (!worst) break;
        this.journal('error', `Stop out: margin level ${m.marginLevel}%`);
        this.closeVolume(worst, worst.volume, 'stop_out');
      }
    }
  }

  private pnl(p: Position, acc: Account, rates: ReturnType<typeof buildRates>): number {
    const st = this.syms.get(p.symbol)!;
    return profitMinor(st.spec, p.side, p.volume, p.openPrice, closePrice(p.side, st.quote), acc.currency, rates);
  }

  private openPosition(
    acc: Account,
    req: Pick<OrderRequest, 'symbol' | 'side' | 'volume' | 'sl' | 'tp' | 'trailing'>,
    price: number,
    reason: Deal['reason'],
  ): Position {
    const pos: Position = {
      id: this.id(),
      accountId: acc.id,
      symbol: req.symbol,
      side: req.side,
      volume: req.volume,
      openPrice: price,
      openTime: this.now(),
      sl: req.sl,
      tp: req.tp,
      trailing: req.trailing,
      commission: commissionMinor(req.volume),
      swap: 0,
    };
    this.positions.get(acc.id)!.push(pos);
    this.addDeal({
      id: this.id(), accountId: acc.id, positionId: pos.id, symbol: pos.symbol, side: pos.side, entry: 'in',
      volume: pos.volume, price, time: pos.openTime, profit: 0, commission: pos.commission, reason,
    });
    this.journal('info', `#${pos.id} ${pos.side} ${pos.volume / 100} ${pos.symbol} at ${price} filled`);
    this.emitPositions(acc.id);
    return pos;
  }

  /** Close `vol` of a position, realize P/L into balance. Returns exit price. */
  private closeVolume(pos: Position, vol: number, reason: Deal['reason']): number {
    const acc = this.accounts.get(pos.accountId)!;
    const st = this.syms.get(pos.symbol)!;
    const exit = closePrice(pos.side, st.quote);
    const profit = profitMinor(st.spec, pos.side, vol, pos.openPrice, exit, acc.currency, this.rates());
    // Entry commission is realized pro-rata (integer math), exit commission charged now.
    const entryComm = vol === pos.volume ? pos.commission : Math.round((pos.commission * vol) / pos.volume);
    const exitComm = commissionMinor(vol);
    acc.balance += profit + entryComm + exitComm + (vol === pos.volume ? pos.swap : 0);
    pos.commission -= entryComm;
    pos.volume -= vol;
    const list = this.positions.get(acc.id)!;
    if (pos.volume === 0) list.splice(list.indexOf(pos), 1);
    this.addDeal({
      id: this.id(), accountId: acc.id, positionId: pos.id, symbol: pos.symbol, side: pos.side === 'buy' ? 'sell' : 'buy',
      entry: 'out', volume: vol, price: exit, time: this.now(), profit, commission: entryComm + exitComm, reason,
    });
    this.journal(reason === 'stop_out' ? 'error' : 'info', `#${pos.id} closed ${vol / 100} at ${exit} (${reason})`);
    this.emit({ type: 'account', account: { ...acc } });
    this.emitPositions(acc.id);
    return exit;
  }

  private makeDepth(symbol: string): Depth {
    const st = this.syms.get(symbol);
    if (!st) return { symbol, bids: [], asks: [], time: this.now() };
    const scale = 10 ** st.spec.digits;
    const bidPts = Math.round(st.quote.bid * scale);
    const askPts = Math.round(st.quote.ask * scale);
    const step = Math.max(1, Math.round(st.spreadPts / 3));
    const lvl = (i: number) => Math.round((st.spec.minVolume * 50 + this.rand() * 400 * st.spec.minVolume) * (1 + i * 0.4));
    const bids = Array.from({ length: 10 }, (_, i) => ({ price: (bidPts - i * step) / scale, volume: lvl(i) }));
    const asks = Array.from({ length: 10 }, (_, i) => ({ price: (askPts + i * step) / scale, volume: lvl(i) }));
    return { symbol, bids, asks, time: this.now() };
  }

  private pushDepth(): void {
    for (const sub of this.depthSubs) sub.cb(this.makeDepth(sub.symbol));
  }

  private quotesRecord(): Record<string, Quote> {
    const r: Record<string, Quote> = {};
    for (const [k, v] of this.syms) r[k] = v.quote;
    return r;
  }
  private specsRecord(): Record<string, SymbolSpec> {
    const r: Record<string, SymbolSpec> = {};
    for (const [k, v] of this.syms) r[k] = v.spec;
    return r;
  }
  private rates() {
    return buildRates(this.specsRecord(), this.quotesRecord());
  }
  private metrics(accountId: string) {
    return computeAccountMetrics(
      this.accounts.get(accountId)!,
      this.positions.get(accountId)!,
      this.quotesRecord(),
      this.specsRecord(),
    );
  }

  private addDeal(d: Deal): void {
    this.deals.get(d.accountId)!.push(d);
    this.emit({ type: 'deal', deal: d });
  }
  private emitPositions(accountId: string): void {
    this.emit({ type: 'positions', accountId, positions: this.positions.get(accountId)!.map((p) => ({ ...p })) });
  }
  private emitOrders(accountId: string): void {
    this.emit({ type: 'orders', accountId, orders: this.orders.get(accountId)!.map((o) => ({ ...o })) });
  }
  private journal(level: 'info' | 'warn' | 'error', message: string): void {
    this.emit({ type: 'journal', entry: { id: this.id(), time: this.now(), level, message } });
  }
  private reject(error: string): OrderResult {
    this.journal('warn', error);
    return { ok: false, error };
  }
  private emit(e: TradingEvent): void {
    for (const l of this.listeners) l(e);
  }
  private id(): string {
    return String(++this.seq);
  }
  private delay(): Promise<void> {
    return this.latency > 0 ? new Promise((r) => setTimeout(r, this.latency)) : Promise.resolve();
  }
}
