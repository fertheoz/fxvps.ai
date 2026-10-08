/**
 * Client self-service REST (funding requests, KYC documents) served by the
 * gateway at `/api/client/*` next to `/ws`, authenticated with the same
 * identity token the WebSocket uses. Mock mode (no gateway) has no API.
 */
import { isGatewayApi } from '../store/api';
import { defaultGateway, loadGateway } from '../store/connection';
import { sessionToken } from '../store/session';
import type { EconEvent } from '../lib/econCalendar';

export interface ClientAccount {
  externalId: string;
  login: number;
  name: string;
  group: string;
  currency: string;
  balance: number;
  equity: number;
  margin: number;
  kyc: 'none' | 'pending' | 'approved' | 'rejected';
}
export type FundingKind = 'deposit' | 'withdraw';
export type FundingMethod = 'usdt_trc20' | 'bank';
export interface FundingRequest {
  id: string;
  account: number;
  kind: FundingKind;
  method: FundingMethod;
  amount: number;
  currency: string;
  details: string;
  requestedAt: number;
  status: 'requested' | 'approved' | 'rejected' | 'paid';
  note: string | null;
}
export interface KycDocument {
  id: string;
  account: number;
  kind: string;
  filename: string;
  size: number;
  uploadedAt: string;
}
export interface FundingInstructions {
  usdtTrc20Address: string;
  bankDetails: string;
  minDepositMinor: number;
  minWithdrawMinor: number;
}
export interface ClientMe {
  subject: string;
  name: string;
  accounts: ClientAccount[];
  unknownAccounts: string[];
  funding: FundingRequest[];
  documents: KycDocument[];
  instructions: FundingInstructions;
}

/** `wss://trade.fxvps.ai/ws` -> `https://trade.fxvps.ai/api/client`; null without a gateway. */
export function clientApiBase(): string | null {
  if (!isGatewayApi()) return null;
  const gw = loadGateway() ?? defaultGateway();
  if (!gw) return null;
  try {
    const u = new URL(gw.url);
    u.protocol = u.protocol === 'wss:' ? 'https:' : 'http:';
    u.pathname = '/api/client';
    u.search = '';
    return u.toString().replace(/\/$/, '');
  } catch {
    return null;
  }
}

/** Error answer of the client API (`status` 0 = no answer). */
export class ClientApiError extends Error {
  readonly status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = 'ClientApiError';
    this.status = status;
  }
}

async function call<T>(path: string, init: RequestInit = {}): Promise<T> {
  const base = clientApiBase();
  if (!base) throw new ClientApiError(0, 'no gateway');
  const token = (loadGateway() ?? defaultGateway())?.token || sessionToken();
  const res = await fetch(`${base}${path}`, {
    ...init,
    headers: { accept: 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}), ...(init.headers ?? {}) },
  });
  if (!res.ok) {
    let message = `${res.status}`;
    try {
      const e = (await res.json()) as { error?: { message?: string } };
      if (e.error?.message) message = e.error.message;
    } catch {
      /* non-JSON */
    }
    throw new ClientApiError(res.status, message);
  }
  return (await res.json()) as T;
}

export interface CopyStrategy {
  account: number;
  name: string;
  description: string;
  perfFeeBps: number;
  public: boolean;
  pnl30d: number;
  return30dPct: number | null;
  deals30d: number;
  winRate: number | null;
  followers: number;
  /** Largest fall from a peak over 30 days (closed deals), % of that peak. */
  maxDrawdownPct?: number | null;
  /** Cumulative return % at the end of each of the last 30 days (closed deals). */
  returnCurvePct?: number[];
}
export interface CopySubscription {
  follower: number;
  provider: number;
  ratioBps: number;
  equityStopPct: number;
  active: boolean;
  stoppedReason: string | null;
  /** Minor units of the account currency. */
  realized: number;
  feesPaid: number;
}
export interface CopyOverview {
  strategies: CopyStrategy[];
  subscriptions: CopySubscription[];
}

const json = (body: unknown): RequestInit => ({ method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

export interface Sentiment {
  symbol: string;
  /** Share of open lots that are long (0..100). */
  longPct: number;
  traders: number;
}

export interface IbPeriod {
  clients: number;
  deals: number;
  lots: number;
  payout: number;
  share?: number;
  rebate?: number;
  override?: number;
  currency: string | null;
}
export interface IbDashboard {
  ib: number;
  code: string;
  sharePct: number;
  perLotCents: number;
  overridePct: number;
  clients: number;
  /** Last UTC day paid out (YYYY-MM-DD). */
  paidThrough?: string | null;
  thisMonth: IbPeriod | null;
  lastMonth: IbPeriod | null;
  payouts: { amount: number; currency: string; reason: string; status: string; requestedAt: string }[];
}

export interface Statement {
  account: number;
  name: string;
  broker: string;
  group: string;
  currency: string;
  from: string;
  to: string | null;
  generatedAt: string;
  balance: number;
  equity: number;
  margin: number;
  trades: { at: string; symbol: string; side: 'buy' | 'sell'; lots: number; price: number; pnl: number; commission: number; swap: number; position: number }[];
  /** `commission` covers every deal in the period (both sides); `commissionOpen` is its opening-side part. */
  totals: { trades: number; lots: number; pnl: number; commission: number; commissionOpen?: number; swap: number };
  positions: { id: number; symbol: string; side: 'buy' | 'sell'; lots: number; openPrice: number; openedAt: string; swap: number }[];
  /** `amount` is a magnitude; the kind gives the direction (see `cashSigned`). */
  cash: { at: string; kind: StatementCashKind; amount: number; reason: string }[];
}

export type StatementCashKind = 'deposit' | 'withdraw' | 'copy_fee' | 'copy_fee_income' | 'nbp';

/** Signed balance effect of a statement cash row (credit > 0). */
export const cashSigned = (c: { kind: StatementCashKind; amount: number }) =>
  c.kind === 'deposit' || c.kind === 'copy_fee_income' || c.kind === 'nbp' ? c.amount : -c.amount;

export const clientApi = {
  sentiment: () => call<Sentiment[]>('/sentiment'),
  ib: () => call<{ ibs: IbDashboard[] }>('/ib'),
  ibLink: (account: string, code: string) =>
    call<{ ok: boolean; already?: boolean }>('/ib/link', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ account, code }) }),
  statement: (account: string, from: string, to: string) =>
    call<Statement>(`/statement?account=${encodeURIComponent(account)}${from ? `&from=${from}` : ''}${to ? `&to=${to}` : ''}`),
  copy: () => call<CopyOverview>('/copy'),
  copySubscribe: (r: { account: string; provider: number; ratioBps: number; equityStopPct: number }) => call<{ ok: boolean }>('/copy/subscribe', json(r)),
  copyUnsubscribe: (r: { account: string; provider: number; close: boolean }) => call<{ ok: boolean }>('/copy/unsubscribe', json(r)),
  me: () => call<ClientMe>('/me'),
  /** Economic calendar between `from` and `to` (epoch ms), all currencies. */
  calendar: (from: number, to: number) => call<{ from: number; to: number; events: EconEvent[] }>(`/calendar?from=${Math.floor(from)}&to=${Math.floor(to)}`),
  requestFunding: (req: { account: string; kind: FundingKind; method: FundingMethod; amount: number; details: string }) =>
    call<FundingRequest>('/funding', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(req) }),
  uploadDocument: (account: string, kind: string, file: File) =>
    call<KycDocument>(`/kyc/documents?account=${encodeURIComponent(account)}`, {
      method: 'POST',
      headers: { 'content-type': file.type || 'application/octet-stream', 'x-filename': file.name, 'x-doc-kind': kind },
      body: file,
    }),
};
