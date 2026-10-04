import type {
  AdminUser, AuditEntry, BalanceOpRequest, Client, EsmaPreset, FixSession, Group, Order,
  Position, Settings, SymbolSpec, Trade,
} from "../schemas";
import type { Role } from "../rbac";

export interface Actor {
  name: string;
  role: Role;
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

  listTrades(): Promise<Trade[]>;
  statements(): Promise<Statement[]>;

  listAudit(): Promise<AuditEntry[]>;

  listUsers(): Promise<AdminUser[]>;
  saveUser(u: AdminUser, actor: Actor): Promise<AdminUser>;

  getSettings(): Promise<Settings>;
  saveSettings(s: Settings, actor: Actor): Promise<Settings>;
}
