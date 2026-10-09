import type {
  AdminUser, AuditEntry, BalanceOpRequest, Client, EsmaPreset, FixSession, Group, Order,
  Position, Settings, SymbolSpec, Trade,
} from "../schemas";
import type { Role } from "../rbac";

export interface Actor {
  name: string;
  role: Role;
  /** Stable user id (token `sub`) when known; used for the 4-eyes rule. */
  sub?: string;
}

export interface SymbolExposure {
  symbol: string;
  /** Net client lots (buy positive). */
  netLots: number;
  /** Net notional in base currency minor units (signed). */
  notional: number;
  aBookLots: number;
  bBookLots: number;
  /** LP-side net lots (hedge); mismatch with aBookLots raises an alert. */
  lpLots: number;
  /** Broker hedge of the B-book excess at the LP (stage 7). */
  hedgeLots?: number;
  hedgePendingLots?: number;
  unhedgedBLots?: number;
  limitLots?: number | null;
  /** EWMA daily volatility (%) and 1-day 95% VaR of the B-book net (USD). */
  volDailyPct?: number;
  varUsd?: number | null;
  overLimit?: boolean;
  hedgeRealized?: number;
  hedgeCurrency?: string | null;
}

export type HedgeMode = "switch_to_a_book" | "hedge_excess";
export const HEDGE_MODES: readonly HedgeMode[] = ["switch_to_a_book", "hedge_excess"];

/** B-book exposure limits / auto-hedge (lots as numbers; raw 1e8 on the wire). */
export interface HedgePolicy {
  enabled: boolean;
  mode: HedgeMode;
  defaultSymbolLimit: number | null;
  symbolLimits: Record<string, number>;
  totalLimit: number | null;
  accountLimit: number | null;
  hedgeRatioPct: number;
  releasePct: number;
  /** TWAP: at most this many lots per hedge order, one per interval (null = all at once). */
  sliceLots?: number | null;
  sliceIntervalS?: number;
  /** 1-day 95% VaR cap of the B-book (USD); null = off. */
  varLimitUsd?: number | null;
  /** Live total VaR (USD), read-only. */
  varTotalUsd?: number;
  /** Per-currency B-book caps (USD notional of the leg); flow over a cap goes A-book. */
  currencyLimitsUsd?: Record<string, number>;
  /** News restriction: ± minutes of a high-impact event, and what to do with new flow. */
  newsWindowMin?: number;
  newsAction?: NewsAction;
  /** Read-only: an event is near right now. */
  inNewsWindow?: boolean;
  /** Warehouse volume caps: B-book lots opened in the last N minutes per account / per symbol. */
  burstWindowMin?: number;
  burstAccountLots?: number | null;
  burstSymbolLots?: number | null;
}

export type NewsAction = "none" | "a_book" | "reject";
export const NEWS_ACTIONS: readonly NewsAction[] = ["none", "a_book", "reject"];

/** One currency leg across the open positions. */
export interface CurrencyExposure {
  currency: string;
  aAmount: number;
  bAmount: number;
  netAmount: number;
  aUsd: number | null;
  bUsd: number | null;
  netUsd: number | null;
  limitUsd: number | null;
  overLimit: boolean;
}

export interface ClientFlowRow {
  login: number;
  name: string | null;
  group: string;
  currency: string;
  trades: number;
  fills: number;
  avgHoldSecs: number;
  shortHoldPct: number;
  winRate: number;
  realisedPnl: number;
  brokerPnl: number;
  avgSlipGainPoints: number;
  /** Mid move in the client's favour after fills (points), null until measured. */
  markout1s?: number | null;
  markout5s?: number | null;
  markout60s?: number | null;
  toxicity: number;
}

export interface DashboardStats {
  activeAccounts: number;
  totalAccounts: number;
  depositsToday: number;
  withdrawalsToday: number;
  aBookPnl: number;
  bBookPnl: number;
  openPositions: number;
  lpUp: number;
  lpTotal: number;
  pnlSeries: { t: string; aBook: number; bBook: number }[];
  depositSeries: { day: string; deposits: number; withdrawals: number }[];
}

export type DashboardRange = "today" | "24h" | "7d" | "30d";

export interface DashboardBucket {
  t: string;
  label: string;
  markup: number;
  commission: number;
  bBook: number;
  swap?: number;
  lots: number;
  orders: number;
  rejects: number;
  /** Execution quality per bucket (stage 9). */
  avgSlipPts?: number;
  p95LatencyMs?: number;
  fills?: number;
}

export type AlertSeverity = "info" | "warning" | "critical";
export interface Alert {
  id: string;
  kind: string;
  target: string;
  severity: AlertSeverity;
  title: string;
  detail: string;
  raisedAt: number;
  resolvedAt: number | null;
  acked: boolean;
}
export interface AlertList {
  active: Alert[];
  recent: Alert[];
}

export interface DashboardTotals {
  revenue: number;
  markup: number;
  commission: number;
  bBook: number;
  lots: number;
  orders: number;
  rejects: number;
}

/** Exact per-bucket series from the engine's deals / orders for a range. */
export interface DashboardSeries {
  range: DashboardRange;
  since: string;
  buckets: DashboardBucket[];
  totals: DashboardTotals;
  previous: DashboardTotals;
  topSymbols: { symbol: string; lots: number; revenue: number }[];
  winners: { login: number; name: string; pnl: number; lots: number }[];
  losers: { login: number; name: string; pnl: number; lots: number }[];
  risk: { login: number; name: string; marginLevelPct: number; equity: number; margin: number; marginCall: boolean }[];
  execution: { orders: number; fillRate: number; avgClientSlipPts: number; p95LatencyMs: number; p50LatencyMs: number };
}

/** Engine routing rule (camelCase mirror of risk::RoutingRule). */
export interface RoutingRule {
  id: string;
  name: string;
  enabled: boolean;
  groups: string[];
  accounts: number[];
  symbols: string[];
  /** Lots (converted to centi-lots on the wire). */
  minLots: number | null;
  maxLots: number | null;
  kind: "any" | "market" | "pending";
  hoursUtc: [number, number] | null;
  routing: "ABook" | "BBook" | null;
  aBookPct: number | null;
  markupPoints: number | null;
  maxSlippagePoints: number | null;
  partialFill: "CancelRemainder" | "AllOrNone" | { Retry: { max_attempts: number } } | null;
  /** Client toxicity window 0..100 (stage 7 flow profile); null = any. */
  minToxicity: number | null;
  maxToxicity: number | null;
  /** Order sources; empty = any. */
  platforms: RulePlatform[];
  /** Client IP prefixes / IPv4 CIDRs; empty = any. */
  ipPrefixes: string[];
  /** |net open position| of the account on the symbol, lots; null = any. */
  minNopLots: number | null;
  maxNopLots: number | null;
  /** Burst: at least minWindowLots opened in the last windowMinutes. */
  windowMinutes: number | null;
  minWindowLots: number | null;
  /** Scalper profile; null = any. */
  scalper: boolean | null;
  /** Herd: at least this many accounts sent the same symbol and side within herdWindowS (default 60). */
  herdAccounts: number | null;
  herdWindowS: number | null;
  /** Within ± minutes of a high-impact calendar event; null = any. */
  newsWindowMin: number | null;
  /** Daily window in minutes of the UTC day [from, to), wraps past midnight; null = any. */
  minutesUtc: [number, number] | null;
  /** 0 = Monday .. 6 = Sunday; empty = every day. */
  weekdays: number[];
  /** Volatility: raw LP spread at least this many points; null = any. */
  minSpreadPoints: number | null;
}

export type RulePlatform = "unknown" | "terminal" | "mobile" | "api" | "bridge" | "copy";
export const RULE_PLATFORMS: RulePlatform[] = ["terminal", "mobile", "api", "bridge", "copy", "unknown"];

export interface RulesDryRun {
  since: string;
  rules: { id: string; name: string; enabled: boolean; orders: number; lots: number }[];
  unmatched: { orders: number; lots: number };
  samples: { orderId: string; login: number; name: string | null; symbol: string; lots: number; rule: string | null; routing: "A" | "B" }[];
}

export interface BalanceOpResult {
  id: string;
  status: "applied" | "pending_approval";
  newBalance: number;
  newCredit: number;
}

/** A balance operation awaiting (or after) the second approval (4-eyes). */
export interface ApprovalRequest {
  id: string;
  clientId: string;
  login: number;
  type: "deposit" | "withdraw" | "credit";
  amount: number;
  currency: string;
  reason: string;
  requestedBy: string;
  requestedByRole: string;
  /** Stable subject of the requester (token `sub`); the approver must differ. */
  requestedBySub?: string;
  requestedAt: string;
  status: "pending_approval" | "applied" | "rejected";
  decidedBy: string | null;
  decidedAt: string | null;
  note: string | null;
}

export type ApprovalStatusFilter = "pending_approval" | "all";

/** Live-update subscription: topics are AdminApi method names to refetch ("*" = all). */
export type Unsubscribe = () => void;

export interface ListQuery {
  search?: string;
}

export interface MarginCallRow {
  client: Client;
  marginLevel: number;
  state: "margin_call" | "stop_out";
}

export interface Statement {
  login: number;
  name: string;
  currency: string;
  opening: number;
  deposits: number;
  withdrawals: number;
  pnl: number;
  commission: number;
  swap: number;
  closing: number;
}

/** An order sent to the LP, its executions and the client orders it was allocated to. */
export interface LpExecution {
  id: string;
  /** LP that took the order (multi-LP); null in journals from before aggregation. */
  lp?: string | null;
  symbol: string;
  side: "buy" | "sell";
  lots: number;
  filledLots: number;
  avgPrice: number;
  status: "working" | "partial" | "filled" | "rejected";
  reason: string | null;
  createdAt: string;
  fills: { execId: string; lots: number; price: number; at: string }[];
  clients: { orderId: string; login: number; lots: number; price: number; detail?: ClientOrderDetail }[];
  /** Full LP-side detail (absent on older servers / the mock). */
  detail?: LpOrderDetail;
  /** Optional columns (hidden by default). */
  clOrdId?: string;
  kind?: string;
  attempt?: number;
  attempts?: number;
  firstFillMs?: number | null;
  lastFillMs?: number | null;
  lpSlipPts?: number | null;
  sentBid?: number | null;
  sentAsk?: number | null;
  limit?: number | null;
  stop?: number | null;
  revision?: number;
  login?: number | null;
  orderIds?: string;
  fillCount?: number;
}

/** One raw FIX frame of the trading session, SOH shown as `|`. */
export interface FixMessage {
  at: string;
  lp: string;
  dir: "in" | "out";
  msgType: string;
  clOrdId: string | null;
  origClOrdId: string | null;
  orderId: string | null;
  execId: string | null;
  raw: string;
}

/** One LP order as the bridge saw it: what went out and every report back. */
export interface LpOrderDetail {
  lpOrderId?: string;
  /** ClOrdID of the first revision at the LP (`LP-<id>`). */
  clOrdId?: string;
  lp?: string | null;
  lots?: number;
  side?: "buy" | "sell";
  kind: string;
  limit: number | null;
  stop: number | null;
  resting: boolean;
  revision: number;
  hedge: boolean;
  sentBid: number | null;
  sentAsk: number | null;
  sentAt: string;
  /** 1-based attempt index in the client order's chain, and the chain length. */
  attempt: number;
  attempts: number;
  firstFillMs: number | null;
  lastFillMs: number | null;
  /** Fill vs the LP quote we saw when sending, points, + = worse for us. */
  lpSlipPts: number | null;
  fills: { execId: string; lots: number; price: number; at: string; latencyMs: number }[];
  reason: string | null;
  done: boolean;
}

/** One client order: what was asked, what was given. */
export interface ClientOrderDetail {
  orderId: string;
  clientOrderId: string;
  login: number;
  side: "buy" | "sell";
  kind: string;
  origin: string;
  platform: string;
  ip: string | null;
  lots: number;
  filledLots: number;
  requested: number | null;
  price: number | null;
  /** Fill vs requested, points, + = worse for the client. */
  clientSlipPts: number | null;
  status: string;
  reason: string | null;
  lpAttempts: number;
  createdAt: string;
  rule: string | null;
  book: "A" | "B";
  maxDeviationPts: number | null;
  markupOverridePts: number | null;
  lpOrders?: LpOrderDetail[];
}

/** One client order in the execution-quality report. Points: positive slippage = worse for the client. */
export interface ExecutionRow {
  id: string;
  at: string;
  login: number;
  name: string | null;
  symbol: string;
  side: "buy" | "sell";
  type: "market" | "limit" | "stop" | "stop_limit";
  lots: number;
  filledLots: number;
  status: "filled" | "partial" | "rejected";
  reason: string | null;
  book: "A" | "B";
  requested: number | null;
  fill: number | null;
  clientSlipPts: number | null;
  lpPrice: number | null;
  /** LP quote (our side) when the LP order went out; `lpSlipPts` = fill vs this. */
  lpSentPrice: number | null;
  lpSlipPts: number | null;
  /** LP orders this client order took part in (re-arms send again). */
  attempts: number;
  capturePts: number | null;
  lpLatencyMs: number | null;
  lpFills: number;
  lpStatus: "working" | "filled" | "rejected" | null;
  rearmed: boolean;
}

export interface ExecutionSummary {
  symbol: string;
  orders: number;
  fillRate: number;
  partialRate: number;
  rejectRate: number;
  avgSlipPts: number;
  p95SlipPts: number;
  improvedRate: number;
  avgCapturePts: number;
  avgLpSlipPts: number;
  avgLatencyMs: number;
  p50LatencyMs: number;
  p95LatencyMs: number;
  avgAttempts: number;
}

export interface ExecutionReport {
  rows: ExecutionRow[];
  bySymbol: ExecutionSummary[];
}

/** Broker revenue legs (minor units). `lp` is our own result at the LP. */
export interface RevenueTotals {
  markup: number;
  bBook: number;
  commission: number;
  /** Swap kept by the broker (B-book) - stage 8. */
  swap?: number;
  lp: number;
  total: number;
}

/** Rollover schedule (stage 8). */
export interface SwapConfig {
  enabled: boolean;
  rolloverHourUtc: number;
  skipWeekend: boolean;
  lastRolloverAt?: string | null;
}

export interface RolloverResult {
  applied: boolean;
  positions: number;
  reason: string;
}

export interface PerfReport {
  engine: { samples: number; totalCommands: number; p50Us: number; p95Us: number; p99Us: number; maxUs: number; uptimeS: number };
  writerSeq: number;
  replica: { seq: number; lagCommands: number; reloads: number; applied: number } | null;
  budget: { p99Us: number; ok: boolean };
  /** Last nightly load test (yuk-sinavi.sh). */
  loadtest?: { at: string; ok: boolean; clients: number; connected: number; ordersPerMin: number; targetPerMin: number; ackP99Ms: number; rejectPct: number; quotesPerS: number } | null;
}
export interface Tenant { id: string; name: string; groups: string[]; hostnames: string[]; brandColor?: string; logoUrl?: string; supportEmail?: string }

/** Account-behaviour thresholds (0 = flag off). */
export interface BehaviorThresholds {
  windowH: number;
  scalperHoldS: number;
  scalperMinCloses: number;
  scalperPct: number;
  burstPerMin: number;
  churnConnects: number;
  authFails: number;
  ipCount: number;
  herdAccounts: number;
  herdWindowS: number;
}
export const DEFAULT_BEHAVIOR: BehaviorThresholds = { windowH: 24, scalperHoldS: 60, scalperMinCloses: 10, scalperPct: 50, burstPerMin: 30, churnConnects: 30, authFails: 10, ipCount: 5, herdAccounts: 5, herdWindowS: 60 };
export interface HerdSignal { symbol: string; side: "Buy" | "Sell"; accounts: number; lots: number; firstTs: number; lastTs: number }

export const RULE_METRICS = ["exposure_net_lots", "unhedged_b_lots", "b_book_net_lots", "var_total_usd", "var_symbol_usd", "currency_exposure_usd", "margin_calls", "stop_outs", "orders_per_min", "open_positions", "lp_latency_ms"] as const;
export type RuleMetric = (typeof RULE_METRICS)[number];
export interface AlertRule { id: string; enabled: boolean; metric: RuleMetric; target: string; op: "gt" | "lt"; threshold: number; severity: AlertSeverity; title: string }
export interface RuleEval { id: string; value: number | null; fired: boolean }
export interface RuleFiring { id: string; severity: AlertSeverity; title: string; detail: string }
export interface AlertRulesView { rules: AlertRule[]; metrics: string[]; evals: RuleEval[]; firing: RuleFiring[] }
export interface HedgePreviewRow { symbol: string; bBookNetLots: number; hedgeLots: number; targetLots: number; deltaLots: number; firstOrder: { side: "buy" | "sell"; lots: number } | null; limitLots: number | null; varUsd: number | null }
export interface HedgePreview { symbols: HedgePreviewRow[]; varTotalUsd: number; varLimitUsd: number | null; varOver: boolean; currency: { currency: string; usd: number; limitUsd: number | null; over: boolean }[] }

/** Tick-warehouse analytics (parça 13). */
export interface LiquidityHour { hour: number; ticks: number; avgSpreadPoints: number; minSpreadPoints: number; maxSpreadPoints: number; avgBidLots: number; avgAskLots: number }
export interface LiquidityMap { symbol: string; day: string; ticks: number; hours: LiquidityHour[]; days: string[]; symbols: string[] }
export interface MarkoutRow { id: string; at: string; login: number; symbol: string; side: "buy" | "sell"; entry: "in" | "out"; lots: number; price: number; m1: number | null; m5: number | null; m30: number | null }
export interface MarkoutReport { rows: MarkoutRow[]; summary: { symbol: string; deals: number; m1: number; m5: number; m30: number }[] }
export interface WhatIfRow { symbol: string; legs: number; lots: number; currency: string; delta: number; deltaUsd: number | null }
export interface WhatIfReport { deltaPoints: number; group: string | null; rows: WhatIfRow[]; totalUsd: number }

/** Algorithmic pricing sandbox (parça 14). */
export interface AlgoVars { spread: number; net: number; vol: number; hour: number; news: number; markup: number }
export interface AlgoTestRow { symbol: string; vars: AlgoVars; bidPoints: number; askPoints: number; bidNow: number; askNow: number; bid: number; ask: number; crossed: boolean }

/** Temporary markup set through the pricing API (parça 12). */
export interface TempMarkup { id: string; group: string; symbol: string | null; points: number; from: string; until: string; reason: string; active: boolean }

export type BehaviorFlag = "scalper" | "burst" | "churn" | "brute_force" | "ip_hopping" | "flood";
export interface AccountActivity {
  login: number;
  name: string;
  group: string;
  platforms: string[];
  ips: string[];
  orders: number;
  cancels: number;
  maxPerMin: number;
  closes: number;
  medianHoldS: number | null;
  scalpPct: number;
  connects: number;
  disconnects: number;
  authFails: number;
  flags: BehaviorFlag[];
  score: number;
}
export interface IpActivity { ip: string; authFails: number; keyFails: number; connRejects: number; accounts: number[]; flags: BehaviorFlag[] }
export interface ActivityReport { windowH: number; accounts: AccountActivity[]; ips: IpActivity[]; herd?: HerdSignal[] }
export type ActivityKind = "connect" | "disconnect" | "auth_fail" | "key_fail" | "conn_reject";
export interface ActivityEvent { tsMs: number; kind: ActivityKind; account: number | null; names: string[]; platform: string; ip: string | null; detail: string }

/** A user of a connected trading platform (MT5 server behind the bridge). */
export interface PlatformUser {
  institution: string;
  login: number;
  name: string;
  email: string;
  phone: string;
  country: string;
  city: string;
  address: string;
  group: string;
  server: string;
  leverage: number;
  balance: number;
  currency: string;
  registeredAt: string;
  lastLoginAt: string;
  lastIp: string;
  status: string;
  comment: string;
  extra: Record<string, string>;
  updatedMs: number;
}
export interface PlatformUserDetail { user: PlatformUser; institution: { id: string; account: string; name: string } | null; events: ActivityEvent[] }

export interface AlertSettings {
  lpDownGraceS: number;
  /** Feed QoS: warn when an LP's market-data latency stays above this (ms, 0 = off). */
  lpSlowMs?: number;
  behavior?: BehaviorThresholds;
  fillRateMinOrders: number;
  fillRateFloorPct: number;
  latencyFloorMs: number;
  latencyMultiplier: number;
  webhookUrl: string;
  /** Write-only: empty keeps the stored token. */
  telegramToken: string;
  telegramTokenSet?: boolean;
  telegramChatId: string;
  quietHoursUtc: [number, number] | null;
  dailyReportHourUtc: number | null;
}
export interface TradingCalendar { holidays: string[] }
/** Economic calendar (plan item 10): release events, `time` in epoch ms (UTC). */
export type EconImpact = "low" | "medium" | "high";
export interface EconEvent { id: string; time: number; at: string; currency: string; title: string; impact: EconImpact; actual: string | null; forecast: string | null; previous: string | null }
export interface EconEventInput { time: number; currency: string; title: string; impact: EconImpact; actual?: string | null; forecast?: string | null; previous?: string | null }
export interface EconEventList { from: number; to: number; events: EconEvent[] }
export interface EconImportResult { total: number; added: number; updated: number; unchanged: number; skipped: number }
export interface StatementMailStatus {
  /** `CORE_STATEMENT_EMAIL=1` on the core-engine. */
  enabled: boolean;
  /** An SMTP channel is configured (CORE_SMTP_URL or IDENTITY_SMTP_URL). */
  configured: boolean;
  from: string | null;
  /** Statements go out on days 1..runDays of the month (UTC). */
  runDays: number;
  /** Last month (YYYY-MM): the period the next run covers. */
  month: string;
  /** Client accounts with a deliverable e-mail address. */
  recipients: number;
  run: { sent: number; passes: number; failed: number; done: boolean } | null;
}
export interface StatementTestResult { ok: boolean; to: string; account: number; month: string }
export interface RuleVersionMeta { id: string; at: string; actor: string; count: number }
export interface SimState { scenario: { rejectPct: number; latencyMs: number }; instruments: { securityId: string; mid: string | null }[] }

export interface KycDocMeta {
  id: string;
  account: number;
  kind: string;
  filename: string;
  contentType: string;
  size: number;
  sha256: string;
  uploadedBy: string;
  uploadedAt: string;
}
export interface FundingRequest {
  id: string;
  account: number;
  clientName?: string | null;
  kind: "deposit" | "withdraw";
  method: "usdt_trc20" | "bank";
  amount: number;
  currency: string;
  details: string;
  requestedBy: string;
  requestedAt: number;
  requestedAtIso?: string;
  status: "requested" | "approved" | "rejected" | "paid";
  decidedBy: string | null;
  decidedAt: number | null;
  decidedAtIso?: string | null;
  note: string | null;
  /** USDT deposits: exact amount the client was told to send (micro-USDT). */
  expectedMicro?: number | null;
  /** On-chain transfer the watcher matched to this request. */
  txHash?: string | null;
  /** hazine.io payment card of a USDT deposit. */
  payUrl?: string | null;
  /** What actually arrived on chain (micro-USDT); only an exact match skips 4-eyes. */
  receivedMicro?: number | null;
  /** hazine reported the payment after the request was decided; needs review. */
  paidAfterDecision?: boolean;
  /** Staff member who reviewed such a late payment. */
  lateHandledBy?: string | null;
  opId: string | null;
}
export type FundingDecision = "approve" | "reject" | "paid" | "handled";
export interface IbRow {
  ib: number; name: string | null; currency: string | null; sharePct: number; clients: number; deals: number; lots: number; commission: number; markup: number; payout: number;
  /** Rebate per closed lot (minor), override on sub-IBs (%), referral code, parent IB. */
  perLotCents?: number; overridePct?: number; code?: string; parent?: number | null;
  share?: number; rebate?: number; override?: number;
  /** Last UTC day paid out (YYYY-MM-DD) and the payout op awaiting approval. */
  paidThrough?: string | null; payoutPending?: string | null;
}
export interface IbReport { from: string; to: string | null; rows: IbRow[] }
/** IB payout worked out by the server: from the day after `paidThrough` (null = from the start) to `to`, inclusive. */
export interface IbPayoutPreview {
  ib: number;
  from: string | null;
  to: string;
  paidThrough: string | null;
  amount: number;
  currency: string;
  /** Payout op of this IB awaiting approval (no new payout until it is decided). */
  pending: string | null;
}
export interface IbPayoutResult extends IbPayoutPreview { op: BalanceOpResult }

export interface TransactionRow {
  txId: string;
  tradingDateTime: string;
  executingEntity: string;
  buyerId: string;
  sellerId: string;
  clientLogin: number;
  clientName: string | null;
  instrument: string;
  assetClass: string | null;
  isin: string;
  side: "buy" | "sell";
  entry: "open" | "close";
  price: number;
  priceCurrency: string | null;
  quantityLots: number;
  quantityUnits: number;
  notional: number;
  tradingCapacity: "DEAL" | "MTCH";
  venue: string;
  executionLp: string | null;
  book: "A" | "B";
  commission: number;
  swap: number;
  realisedPnl: number;
  reason: string;
}
export interface TransactionReport { from: string; to: string | null; rows: TransactionRow[] }

export interface BestExecutionRow {
  venue: string;
  assetClass: string;
  orders: number;
  filled: number;
  rejected: number;
  fillRate: number;
  lots: number;
  volumeSharePct: number;
  avgClientSlipPts: number;
  p95ClientSlipPts: number;
  priceImprovementPct: number;
  p50LatencyMs: number;
  p95LatencyMs: number;
}
export interface BestExecutionReport { from: string; to: string | null; rows: BestExecutionRow[] }

export interface AuditChain {
  count: number;
  chained: number;
  verified: boolean;
  headHash: string;
  brokenAt: string | null;
  lastAt: string | null;
}

export interface RevenueRow {
  id: string;
  at: string;
  kind: "pnl" | "commission";
  ref: string;
  book: "A" | "B";
  login: number;
  symbol: string;
  lots: number;
  price: number;
  lpPrice: number | null;
  client: number;
  broker: number;
  lp: number;
}

export interface RevenueReport {
  total: RevenueTotals;
  last24h: RevenueTotals;
  rows: RevenueRow[];
}

/** One closed deal in the reconciliation report: client side vs LP side, the broker's legs in minor units. */
export interface ReconciliationRow {
  id: string;
  at: string;
  login: number;
  position: string;
  symbol: string;
  side: "buy" | "sell";
  lots: number;
  book: "A" | "B";
  reason: string;
  openClient: number;
  openLp: number | null;
  closeClient: number;
  closeLp: number | null;
  clientPnl: number;
  lpPnl: number;
  markup: number;
  commission: number;
  swapFee: number;
  broker: number;
  /** client P&L + markup = LP P&L (A-book) held for this deal. */
  ok: boolean;
  /** Optional columns (hidden by default). */
  openAt?: string | null;
  holdSecs?: number | null;
  orderId?: string;
  platform?: string | null;
  origin?: string | null;
  rule?: string | null;
  clientSlipPts?: number | null;
  attempts?: number;
  latencyMs?: number | null;
  lpSlipPts?: number | null;
  lpKind?: string | null;
  lpOrderIds?: string;
  swap?: number;
  /** Everything behind the deal (absent on the mock). */
  detail?: {
    deals: { dealId: string; orderId: string; at: string; entry: string; side: "buy" | "sell"; lots: number; price: number; lpPrice: number | null; reason: string; pnl: number; lpPnl: number; markup: number; commission: number; swap: number; swapFee: number }[];
    orders: ClientOrderDetail[];
  };
}

/**
 * The back-office contract. The mock adapter implements it in-browser;
 * the HTTP adapter maps 1:1 to the future backoffice-api REST endpoints
 * (see API.md).
 */
export interface AdminApi {
  dashboard(): Promise<DashboardStats>;
  dashboardSeries(range: DashboardRange): Promise<DashboardSeries>;
  exposure(): Promise<SymbolExposure[]>;
  currencyExposure(): Promise<CurrencyExposure[]>;
  listAlerts(): Promise<AlertList>;
  ackAlert(id: string, actor: Actor): Promise<AlertList>;
  hedgePolicy(): Promise<HedgePolicy>;
  saveHedgePolicy(p: HedgePolicy, actor: Actor): Promise<HedgePolicy>;
  /** Parça 10b: manual hedge, policy change preview, dealer-defined alert rules. */
  manualHedge(req: { symbol: string; side: "buy" | "sell"; lots: number }, actor: Actor): Promise<{ ok: boolean }>;
  /** Parça 12: temporary markup (real-time pricing API). */
  tempMarkups(): Promise<TempMarkup[]>;
  /** Parça 14: evaluate pricing formulas against the live book (nothing saved). */
  testAlgo(req: { bid: string; ask: string; group: string; symbols?: string[] }, actor: Actor): Promise<{ rows: AlgoTestRow[]; error?: string }>;
  setTempMarkup(req: { group: string; symbol: string | null; points: number; ttlS: number; reason: string }, actor: Actor): Promise<TempMarkup[]>;
  clearTempMarkup(id: string, actor: Actor): Promise<TempMarkup[]>;
  previewHedgePolicy(p: HedgePolicy, actor: Actor): Promise<HedgePreview>;
  alertRules(): Promise<AlertRulesView>;
  saveAlertRules(rules: AlertRule[], actor: Actor): Promise<AlertRulesView>;
  previewAlertRules(rules: AlertRule[], actor: Actor): Promise<Pick<AlertRulesView, "evals" | "firing">>;
  /** Per-client flow profile with the toxicity score the rules use. */
  clientFlow(): Promise<ClientFlowRow[]>;

  listClients(q?: ListQuery): Promise<Client[]>;
  getClient(id: string): Promise<Client | null>;
  /** Opens an account in `group` (funding is a separate deposit). */
  openAccount(req: { name: string; email: string; group: string }, actor: Actor): Promise<Client>;
  balanceOp(req: BalanceOpRequest, actor: Actor): Promise<BalanceOpResult>;
  setKyc(id: string, kyc: Client["kyc"], actor: Actor): Promise<Client>;
  /** Moves the account to another group (refused while it has positions or orders). */
  setGroup(id: string, group: string, actor: Actor): Promise<Client>;
  /** Reporting identity (LEI) of a client; null clears it. */
  setProfile(id: string, p: { lei: string | null }, actor: Actor): Promise<Client>;
  /** Introducing broker: this account's share and/or the IB it belongs to. */
  setIb(id: string, p: { sharePct?: number; ibAccount?: number | null; perLotCents?: number; overridePct?: number; code?: string }, actor: Actor): Promise<Client>;
  listKycDocs(id: string): Promise<KycDocMeta[]>;
  kycDocBlob(id: string, doc: string): Promise<Blob>;
  listFunding(status?: string): Promise<FundingRequest[]>;
  decideFunding(id: string, decision: FundingDecision, note: string | undefined, actor: Actor): Promise<FundingRequest>;
  ibReport(from?: string, to?: string): Promise<IbReport>;
  /** What an IB payout up to `to` (a closed day) would book; the amount comes from the server. */
  ibPayoutPreview(ib: number, to: string): Promise<IbPayoutPreview>;
  /** Books it as a deposit through the four-eyes flow. */
  ibPayout(ib: number, to: string, actor: Actor): Promise<IbPayoutResult>;
  /** MiFIR-style transaction report rows for [from, to] (YYYY-MM-DD). */
  transactions(from?: string, to?: string): Promise<TransactionReport>;
  bestExecution(from?: string, to?: string): Promise<BestExecutionReport>;
  auditChain(): Promise<AuditChain>;
  /** Stage 13: operations settings & automation. */
  getAlertSettings(): Promise<AlertSettings>;
  saveAlertSettings(s: AlertSettings, actor: Actor): Promise<AlertSettings>;
  /** Monthly statement e-mail: env flag, SMTP channel and last month's run. */
  getStatementMail(): Promise<StatementMailStatus>;
  /** Mails last month's statement of `account` (null: the first client) to the caller. */
  sendTestStatement(account: number | null, actor: Actor): Promise<StatementTestResult>;
  getCalendar(): Promise<TradingCalendar>;
  saveCalendar(c: TradingCalendar, actor: Actor): Promise<TradingCalendar>;
  /** Economic calendar: `from` / `to` are epoch ms or YYYY-MM-DD (default: last 7 days to 14 days ahead). */
  listEconEvents(from?: string, to?: string): Promise<EconEventList>;
  createEconEvent(e: EconEventInput, actor: Actor): Promise<EconEvent>;
  updateEconEvent(id: string, e: EconEventInput, actor: Actor): Promise<EconEvent>;
  deleteEconEvent(id: string, actor: Actor): Promise<{ ok: boolean }>;
  /** Fetches this week's ForexFactory feed server side; idempotent by (title, currency, time). */
  importEconWeek(actor: Actor): Promise<EconImportResult>;
  ruleVersions(): Promise<RuleVersionMeta[]>;
  restoreRuleVersion(id: string, actor: Actor): Promise<RoutingRule[]>;
  simState(): Promise<SimState>;
  simShock(symbol: string, pct: number, actor: Actor): Promise<unknown>;
  simScenario(s: { rejectPct: number; latencyMs: number }, actor: Actor): Promise<{ rejectPct: number; latencyMs: number }>;
  /** Stage 14: engine latency budget / replica lag and tenants. */
  perf(): Promise<PerfReport>;
  listTenants(): Promise<Tenant[]>;
  saveTenants(ts: Tenant[], actor: Actor): Promise<Tenant[]>;
  /** Parça 10a: account behaviour and platform (MT5) users. */
  activityAccounts(q?: { hours?: number }): Promise<ActivityReport>;
  activityEvents(q?: { hours?: number; login?: number; ip?: string; limit?: number }): Promise<ActivityEvent[]>;
  listPlatformUsers(q?: { q?: string; institution?: string }): Promise<PlatformUser[]>;
  getPlatformUser(id: string): Promise<PlatformUserDetail>;
  upsertPlatformUsers(users: Partial<PlatformUser>[], actor: Actor): Promise<{ upserted: number }>;

  listGroups(): Promise<Group[]>;
  listRules(): Promise<RoutingRule[]>;
  saveRules(rules: RoutingRule[], actor: Actor): Promise<RoutingRule[]>;
  rulesDryRun(): Promise<RulesDryRun>;
  saveGroup(g: Group, actor: Actor): Promise<Group>;

  listSymbols(): Promise<SymbolSpec[]>;
  saveSymbol(s: SymbolSpec, actor: Actor): Promise<SymbolSpec>;

  listPositions(): Promise<Position[]>;
  listOrders(): Promise<Order[]>;
  forceClose(positionIds: string[], actor: Actor): Promise<{ closed: number }>;

  marginCalls(): Promise<MarginCallRow[]>;
  esmaPresets(): Promise<EsmaPreset[]>;
  applyPreset(presetId: string, groupId: string, actor: Actor): Promise<Group>;

  listFixSessions(): Promise<FixSession[]>;
  /** Raw FIX frames of one LP order (ClOrdID), oldest first; read-only. */
  fixMessages(clOrdId: string): Promise<{ clOrdId: string; messages: FixMessage[] }>;
  /** MT5 plugin bridge institutions (+ live sessions). */
  listInstitutions(): Promise<InstitutionList>;
  /** Denetçi: LP/core reconciliation status, settings and corrections. */
  getAudit(): Promise<AuditView>;
  saveAuditSettings(s: AuditSettings, actor: Actor): Promise<AuditSettings>;
  /** Institution staff: own institutions, accounts, positions, deals. */
  getPartnerOverview(): Promise<PartnerOverview>;
  /** Copy trading: strategies (30-day stats) and follower subscriptions. */
  getCopyOverview(): Promise<CopyOverview>;
  saveCopyStrategy(account: number, s: { name: string; description: string; perfFeeBps: number; public: boolean }, actor: Actor): Promise<CopyStrategy>;
  copyUnsubscribe(r: { follower: number; provider: number; close: boolean }, actor: Actor): Promise<{ ok: boolean }>;
  /** Charges performance fees above the high-water mark now (also runs daily at rollover). */
  copySettle(provider: number, actor: Actor): Promise<{ ok: boolean; fees: number }>;
  /** Zero point: the auditor closes its history now. */
  resetAudit(actor: Actor): Promise<{ resetAt: number }>;
  /** Creates an institution; the key is returned once. */
  createInstitution(req: InstitutionReq, actor: Actor): Promise<{ institution: Institution; key: string }>;
  updateInstitution(id: string, req: InstitutionReq, actor: Actor): Promise<Institution>;
  rotateInstitutionKey(id: string, actor: Actor): Promise<{ key: string }>;
  deleteInstitution(id: string, actor: Actor): Promise<{ ok: boolean }>;
  reconnect(sessionId: string, actor: Actor): Promise<FixSession>;
  /** Managed fix-gateway config; passwords are never returned (`password_set` instead). */
  getLpConfig(): Promise<LpConfig | null>;
  /** Multi-LP aggregation policy + runtime per LP (404 when no in-process stack). */
  getLpAggregation(): Promise<LpAggregation>;
  saveLpAggregation(c: LpAggregationInput, actor: Actor): Promise<LpAggregation>;
  /** Per-LP execution quality (fill rate, rejects, slippage, latency). */
  lpReport(): Promise<LpReportRow[]>;
  /** Empty/absent password keeps the stored one. Restarts the FIX sessions. */
  saveLpConfig(c: LpConfig, actor: Actor): Promise<LpConfig>;

  /** Without a range: the newest rows; with one: everything inside it (server-capped). */
  listTrades(from?: string, to?: string): Promise<Trade[]>;
  statements(): Promise<Statement[]>;
  /** Orders routed to the LP with their fills, newest first. */
  listLpExecutions(from?: string, to?: string): Promise<LpExecution[]>;
  /** Execution quality: client slippage, LP leg, latency, per-symbol summary. */
  execution(): Promise<ExecutionReport>;
  /** Realized broker revenue from the ledger (markup, B-book, commission). */
  revenue(): Promise<RevenueReport>;
  /** Parça 13: tick warehouse analytics. */
  liquidity(q?: { symbol?: string; day?: string }): Promise<LiquidityMap>;
  markout(q?: { hours?: number; limit?: number }): Promise<MarkoutReport>;
  whatIf(req: { hours?: number; deltaPoints: number; group?: string | null }, actor: Actor): Promise<WhatIfReport>;
  reconciliation(from?: string, to?: string): Promise<ReconciliationRow[]>;

  listAudit(): Promise<AuditEntry[]>;

  listApprovals(status?: ApprovalStatusFilter): Promise<ApprovalRequest[]>;
  approve(id: string, actor: Actor): Promise<ApprovalRequest>;
  reject(id: string, reason: string, actor: Actor): Promise<ApprovalRequest>;

  listUsers(): Promise<AdminUser[]>;
  saveUser(u: AdminUser, actor: Actor): Promise<AdminUser>;

  getSettings(): Promise<Settings>;
  getSwapConfig(): Promise<SwapConfig>;
  saveSwapConfig(c: SwapConfig, actor: Actor): Promise<SwapConfig>;
  /** Runs today's rollover now (idempotent per UTC day). */
  runRollover(actor: Actor): Promise<RolloverResult>;
  /** Session: `mfaOk` false = mutating calls answer `mfa_required` until a 2FA login. */
  getMe(): Promise<{ sub: string; name: string; role: string; permissions: string[]; mfaOk: boolean }>;
  saveSettings(s: Settings, actor: Actor): Promise<Settings>;

  /** Server push (SSE). Adapters without push omit it and the UI polls. */
  subscribe?(onTopics: (topics: string[]) => void, onStatus?: (connected: boolean) => void): Unsubscribe;
}

/** One FIX session of the LP (fix-gateway `SessionEndpoint`, snake_case as on the wire). */
export interface LpEndpoint {
  addr: string;
  sender_comp_id: string;
  target_comp_id: string;
  username: string | null;
  password: string | null;
  password_set?: boolean;
  reset_on_logon: boolean;
  tls: { server_name?: string | null; ca_file?: string | null; client_cert_file?: string | null; client_key_file?: string | null } | null;
}

export interface LpInstrument {
  symbol: string;
  security_id: string;
  tick_size: string;
  qty_step?: string;
  /** Units per LP OrderQty: 1 = units, 10000 = LMAX FX contracts. */
  contract_size?: number;
}

export type AggMode = "best_price" | "vwap" | "priority" | "round_robin";
export const AGG_MODES: readonly AggMode[] = ["best_price", "vwap", "priority", "round_robin"];

/** One LP's policy as edited in the console (lots as decimal strings, null = no limit). */
export interface LpPolicy {
  name: string;
  enabled: boolean;
  /** false = quote-only: prices, never an order. */
  orders: boolean;
  priority: number;
  minLots: string | null;
  maxLots: string | null;
  /** Core symbols (EURUSD); empty = all the LP quotes. */
  symbols: string[];
}

export interface LpPolicyRuntime extends Omit<LpPolicy, "minLots" | "maxLots"> {
  minLots: number | null;
  maxLots: number | null;
  /** Symbols with a current book. */
  quoting: number;
  /** Symbols excluded by the deviation guard right now. */
  deviating: string[];
  lastQuoteAt: string | null;
  /** Market-data latency measured by the gateway (ms, 0 = unknown). */
  latencyMs?: number;
  /** Suspended by the feed-QoS guard right now. */
  slow?: boolean;
  mdUp: boolean;
  tradeUp: boolean;
}

export interface LpAggregation {
  mode: AggMode;
  maxDeviationPoints: number;
  /** Silent-LP guard (ms, 0 = off). */
  maxQuoteAgeMs?: number;
  /** Feed-QoS guard (ms, 0 = off). */
  maxLatencyMs?: number;
  lps: LpPolicyRuntime[];
}

export interface LpAggregationInput {
  mode: AggMode;
  maxDeviationPoints: number;
  /** Silent-LP guard (ms, 0 = off). */
  maxQuoteAgeMs?: number;
  /** Feed-QoS guard (ms, 0 = off). */
  maxLatencyMs?: number;
  lps: LpPolicy[];
}

export interface LpReportRow {
  lp: string;
  orders: number;
  filled: number;
  partial: number;
  rejected: number;
  working: number;
  requestedLots: number;
  filledLots: number;
  fillRate: number;
  rejectRate: number;
  avgSlipPoints: number;
  p95SlipPoints: number;
  p50LatencyMs: number;
  p95LatencyMs: number;
  lastFillAt: string | null;
  symbols: number;
}

/** fix-gateway `GatewayConfig`. */
export interface LpConfig {
  lp: string;
  heartbeat_secs: number;
  market_depth: number;
  reconnect_delay_ms: number;
  security_id_source: string;
  store_dir: string | null;
  md: LpEndpoint;
  trade: LpEndpoint;
  instruments: LpInstrument[];
  nats: unknown;
  /** false = paused (Stop): no connection attempts. */
  enabled?: boolean;
  /** Failed logons in a row before a session stops retrying. */
  max_logon_failures?: number;
}

export interface BridgeSession {
  institution: string;
  server: string;
  plugin: string;
  ip: string | null;
  since_ms: number;
  last_ms: number;
  orders: number;
  fills: number;
  rejects: number;
  reconcile_ok: boolean | null;
  fill_ms_p50?: number;
  fill_ms_p99?: number;
}
export interface InstitutionActivity { deals: number; lots: number; revenue?: number; clientPnl: number }
export interface Institution {
  id: string;
  name: string;
  account: string;
  ordersPerSec: number;
  ips: string[];
  createdNs: number;
  sessions: BridgeSession[];
  activity?: { h24: InstitutionActivity; d7: InstitutionActivity } | null;
}
export interface InstitutionList {
  institutions: Institution[];
  endpoint: string;
  statusAt: number | null;
}
export interface InstitutionReq {
  id?: string;
  name?: string;
  account: string;
  ordersPerSec?: number;
  ips?: string[];
}

export interface AuditSettings { autoheal: boolean; maxLots: number; maxPerMin: number }
export interface AuditLpStats { orders: number; fills: number; rejects: number; partial: number; ack_ms_p50: number; ack_ms_p99: number; fill_ms_p50: number; fill_ms_p99: number }
export interface AuditIncident { ts_ms: number; kind: string; lp: string; symbol: string; cl_ord_id: string | null; detail: string }
export interface AuditStatus {
  at: number; startedMs: number; ok: boolean; inFlight: number; autoheal: boolean;
  lps: Record<string, AuditLpStats>;
  net: { lp: string; symbol: string; qty: string }[];
  openMismatches: { lp: string; symbol: string; diff: string }[];
  incidentsTotal: number; correctionsSent: number; resetMs: number | null; incidents: AuditIncident[];
}
export interface AuditView { status: AuditStatus | null; settings: AuditSettings; corrections: Record<string, unknown>[]; ageMs?: number | null }

export interface CopyStrategy {
  account: number;
  name: string;
  description: string;
  perfFeeBps: number;
  public: boolean;
  /** Minor units of the account currency. */
  pnl30d: number;
  return30dPct: number | null;
  deals30d: number;
  winRate: number | null;
  followers: number;
  /** Largest peak-to-trough fall over 30 days (closed deals), minor units. */
  maxDrawdown: number;
  /** The same fall as % of its peak; null without a positive balance. */
  maxDrawdownPct: number | null;
  /** Closed-deal equity at the end of each of the last 30 days, minor units. */
  equityCurve: number[];
  /** Cumulative return % per day (same 30 points). */
  returnCurvePct: number[];
}

export interface CopySubscription {
  follower: number;
  provider: number;
  ratioBps: number;
  equityStopPct: number;
  perfFeeBps: number;
  since: string;
  active: boolean;
  stoppedReason: string | null;
  /** Minor units. */
  realized: number;
  hwm: number;
  feesPaid: number;
  openCopies: number;
}

export interface CopyOverview {
  strategies: CopyStrategy[];
  subscriptions: CopySubscription[];
  /** Groups whose accounts may subscribe (server gate). */
  groups: string[];
}

export interface PartnerOverview {
  institutions: Institution[];
  accounts: Client[];
  positions: Position[];
  deals: Record<string, unknown>[];
  endpoint: string;
  statusAt: number | null;
}
