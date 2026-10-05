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

/**
 * The back-office contract. The mock adapter implements it in-browser;
 * the HTTP adapter maps 1:1 to the future backoffice-api REST endpoints
 * (see API.md).
 */
export interface AdminApi {
  dashboard(): Promise<DashboardStats>;
  exposure(): Promise<SymbolExposure[]>;

  listClients(q?: ListQuery): Promise<Client[]>;
  getClient(id: string): Promise<Client | null>;
  /** Opens an account in `group` (funding is a separate deposit). */
  openAccount(req: { name: string; email: string; group: string }, actor: Actor): Promise<Client>;
  balanceOp(req: BalanceOpRequest, actor: Actor): Promise<BalanceOpResult>;
  setKyc(id: string, kyc: Client["kyc"], actor: Actor): Promise<Client>;

  listGroups(): Promise<Group[]>;
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
  /** Empty/absent password keeps the stored one. Restarts the FIX sessions. */
  saveLpConfig(c: LpConfig, actor: Actor): Promise<LpConfig>;

  listTrades(): Promise<Trade[]>;
  statements(): Promise<Statement[]>;

  listAudit(): Promise<AuditEntry[]>;

  listApprovals(status?: ApprovalStatusFilter): Promise<ApprovalRequest[]>;
  approve(id: string, actor: Actor): Promise<ApprovalRequest>;
  reject(id: string, reason: string, actor: Actor): Promise<ApprovalRequest>;

  listUsers(): Promise<AdminUser[]>;
  saveUser(u: AdminUser, actor: Actor): Promise<AdminUser>;

  getSettings(): Promise<Settings>;
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
}
