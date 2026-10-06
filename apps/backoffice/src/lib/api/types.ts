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
  lots: number;
  orders: number;
  rejects: number;
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
}

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
  clients: { orderId: string; login: number; lots: number; price: number }[];
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
  lp: number;
  total: number;
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

/**
 * The back-office contract. The mock adapter implements it in-browser;
 * the HTTP adapter maps 1:1 to the future backoffice-api REST endpoints
 * (see API.md).
 */
export interface AdminApi {
  dashboard(): Promise<DashboardStats>;
  dashboardSeries(range: DashboardRange): Promise<DashboardSeries>;
  exposure(): Promise<SymbolExposure[]>;

  listClients(q?: ListQuery): Promise<Client[]>;
  getClient(id: string): Promise<Client | null>;
  /** Opens an account in `group` (funding is a separate deposit). */
  openAccount(req: { name: string; email: string; group: string }, actor: Actor): Promise<Client>;
  balanceOp(req: BalanceOpRequest, actor: Actor): Promise<BalanceOpResult>;
  setKyc(id: string, kyc: Client["kyc"], actor: Actor): Promise<Client>;
  /** Moves the account to another group (refused while it has positions or orders). */
  setGroup(id: string, group: string, actor: Actor): Promise<Client>;

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

  listTrades(): Promise<Trade[]>;
  statements(): Promise<Statement[]>;
  /** Orders routed to the LP with their fills, newest first. */
  listLpExecutions(): Promise<LpExecution[]>;
  /** Execution quality: client slippage, LP leg, latency, per-symbol summary. */
  execution(): Promise<ExecutionReport>;
  /** Realized broker revenue from the ledger (markup, B-book, commission). */
  revenue(): Promise<RevenueReport>;

  listAudit(): Promise<AuditEntry[]>;

  listApprovals(status?: ApprovalStatusFilter): Promise<ApprovalRequest[]>;
  approve(id: string, actor: Actor): Promise<ApprovalRequest>;
  reject(id: string, reason: string, actor: Actor): Promise<ApprovalRequest>;

  listUsers(): Promise<AdminUser[]>;
  saveUser(u: AdminUser, actor: Actor): Promise<AdminUser>;

  getSettings(): Promise<Settings>;
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
  mdUp: boolean;
  tradeUp: boolean;
}

export interface LpAggregation {
  mode: AggMode;
  maxDeviationPoints: number;
  lps: LpPolicyRuntime[];
}

export interface LpAggregationInput {
  mode: AggMode;
  maxDeviationPoints: number;
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
