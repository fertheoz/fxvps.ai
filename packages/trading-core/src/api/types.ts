/**
 * Domain types shared by the UI, the mock backend and the WebSocket adapter.
 *
 * Money conventions (no float bugs for money):
 *  - Account money (balance, equity, margin, P/L, commission) is an integer
 *    number of **minor units** (cents for USD). Safe up to 2^53.
 *  - Volume is an integer number of **centi-lots** (1 = 0.01 lot).
 *  - Prices are decimal numbers already rounded to the symbol's `digits`
 *    (as delivered by the feed). Any arithmetic on them goes through
 *    big.js in `lib/money.ts`, never through raw JS float math.
 */

export type Side = 'buy' | 'sell';
export type OrderType = 'market' | 'limit' | 'stop' | 'stop_limit';
export type Timeframe = 'M1' | 'M5' | 'M15' | 'M30' | 'H1' | 'H4' | 'D1' | 'W1' | 'MN';
export type ConnectionState = 'connecting' | 'connected' | 'reconnecting' | 'disconnected';

export const TIMEFRAMES: readonly Timeframe[] = ['M1', 'M5', 'M15', 'M30', 'H1', 'H4', 'D1', 'W1', 'MN'];
export const TIMEFRAME_SECONDS: Record<Timeframe, number> = {
  M1: 60,
  M5: 300,
  M15: 900,
  M30: 1800,
  H1: 3600,
  H4: 14400,
  D1: 86400,
  W1: 604800,
  MN: 2592000,
};

export interface SymbolSpec {
  name: string;
  description: string;
  base: string;
  quote: string;
  digits: number;
  /** Size of one pip in price units, as decimal string (e.g. "0.0001"). */
  pipSize: string;
  /** Units of base per 1.00 lot. */
  contractSize: number;
  /** Volume constraints in centi-lots. */
  minVolume: number;
  maxVolume: number;
  volumeStep: number;
  /** Margin rate multiplier applied on top of account leverage (1 = plain leverage). */
  marginRate: number;
  category: 'fx' | 'metal' | 'index' | 'crypto';
}

export interface Quote {
  symbol: string;
  bid: number;
  ask: number;
  /** Unix ms. */
  time: number;
  /** Session open (for daily change). */
  dayOpen: number;
  dayHigh: number;
  dayLow: number;
}

export interface Bar {
  /** Unix seconds (bar open). */
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
}

export interface DepthLevel {
  price: number;
  /** Centi-lots available. */
  volume: number;
}

export interface Depth {
  symbol: string;
  bids: DepthLevel[];
  asks: DepthLevel[];
  time: number;
}

export interface Account {
  id: string;
  name: string;
  /** Parent account id for sub-accounts. */
  parentId?: string;
  currency: string;
  /** Minor units. */
  balance: number;
  /** e.g. 100 for 1:100. */
  leverage: number;
  isDemo: boolean;
  /** Netting: one position per symbol; hedging: several (both sides). */
  marginMode?: MarginMode;
}

export type MarginMode = 'netting' | 'hedging';

export interface Position {
  id: string;
  accountId: string;
  symbol: string;
  side: Side;
  volume: number;
  openPrice: number;
  openTime: number;
  sl?: number;
  tp?: number;
  /** Server-side trailing stop distance in price units. */
  trailing?: number;
  /** Minor units, negative = cost. */
  commission: number;
  swap: number;
}

export interface PendingOrder {
  id: string;
  accountId: string;
  symbol: string;
  side: Side;
  type: Exclude<OrderType, 'market'>;
  volume: number;
  /** Trigger price (limit price for limit, stop price for stop/stop-limit). */
  price: number;
  /** Limit price for stop-limit orders. */
  limitPrice?: number;
  sl?: number;
  tp?: number;
  /** Trailing stop distance (price units) for the resulting position. */
  trailing?: number;
  /** One-cancels-other group. */
  ocoGroup?: number;
  /** Unix ms, undefined = GTC. */
  expiry?: number;
  createdAt: number;
  /** Stop-limit becomes a limit after trigger. */
  triggered?: boolean;
}

export interface Deal {
  id: string;
  accountId: string;
  positionId: string;
  symbol: string;
  side: Side;
  entry: 'in' | 'out';
  volume: number;
  price: number;
  time: number;
  /** Realized P/L in minor units (0 for entry deals). */
  profit: number;
  commission: number;
  reason: 'client' | 'sl' | 'tp' | 'stop_out' | 'order';
}

export interface OrderRequest {
  accountId: string;
  symbol: string;
  side: Side;
  type: OrderType;
  volume: number;
  price?: number;
  limitPrice?: number;
  sl?: number;
  tp?: number;
  /** Trailing stop distance in price units. */
  trailing?: number;
  /** One-cancels-other group (pending orders). */
  ocoGroup?: number;
  expiry?: number;
  /** Client-generated id for idempotency. */
  clientId?: string;
}

export type OrderChanges = Partial<Pick<PendingOrder, 'volume' | 'price' | 'limitPrice' | 'sl' | 'tp' | 'trailing' | 'expiry'>>;

export type OrderResult =
  | { ok: true; positionId?: string; orderId?: string; price?: number }
  | { ok: false; error: string };

export interface JournalEntry {
  id: string;
  time: number;
  level: 'info' | 'warn' | 'error';
  message: string;
}

export type TradingEvent =
  | { type: 'connection'; state: ConnectionState; latencyMs?: number }
  | { type: 'account'; account: Account }
  | { type: 'positions'; accountId: string; positions: Position[] }
  | { type: 'orders'; accountId: string; orders: PendingOrder[] }
  | { type: 'deal'; deal: Deal }
  | { type: 'journal'; entry: JournalEntry };

export type Unsubscribe = () => void;

/**
 * Pluggable backend. The UI only talks to this interface; implementations:
 *  - MockTradingApi: in-browser simulation (default)
 *  - WsTradingApi: WebSocket gateway adapter (see PROTOCOL.md)
 */
export interface TradingApi {
  connect(): Promise<void>;
  disconnect(): void;
  getAccounts(): Promise<Account[]>;
  getSymbols(): Promise<SymbolSpec[]>;
  getBars(symbol: string, timeframe: Timeframe, count: number): Promise<Bar[]>;
  getHistory(accountId: string): Promise<Deal[]>;
  subscribeQuotes(symbols: string[], onQuotes: (quotes: Quote[]) => void): Unsubscribe;
  subscribeDepth(symbol: string, onDepth: (depth: Depth) => void): Unsubscribe;
  onEvent(listener: (event: TradingEvent) => void): Unsubscribe;
  placeOrder(req: OrderRequest): Promise<OrderResult>;
  /** Sets SL / TP / trailing distance (full replacement: undefined = none). */
  modifyPosition(accountId: string, positionId: string, sl?: number, tp?: number, trailing?: number): Promise<OrderResult>;
  closePosition(accountId: string, positionId: string, volume?: number): Promise<OrderResult>;
  /** Changes a pending order in place; keys present with `undefined` remove sl/tp/trailing/expiry. */
  modifyOrder(accountId: string, orderId: string, changes: OrderChanges): Promise<OrderResult>;
  cancelOrder(accountId: string, orderId: string): Promise<OrderResult>;
}
