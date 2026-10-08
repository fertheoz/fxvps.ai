/**
 * Client self-service REST (funding requests, KYC documents) served by the
 * gateway at `/api/client/*` next to `/ws`, authenticated with the same
 * identity token the WebSocket uses. Mock mode (no gateway) has no API.
 */
import { isGatewayApi } from '../store/api';
import { defaultGateway, loadGateway } from '../store/connection';
import { sessionToken } from '../store/session';

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

async function call<T>(path: string, init: RequestInit = {}): Promise<T> {
  const base = clientApiBase();
  if (!base) throw new Error('no gateway');
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
    throw new Error(message);
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

export const clientApi = {
  copy: () => call<CopyOverview>('/copy'),
  copySubscribe: (r: { account: string; provider: number; ratioBps: number; equityStopPct: number }) => call<{ ok: boolean }>('/copy/subscribe', json(r)),
  copyUnsubscribe: (r: { account: string; provider: number; close: boolean }) => call<{ ok: boolean }>('/copy/unsubscribe', json(r)),
  me: () => call<ClientMe>('/me'),
  requestFunding: (req: { account: string; kind: FundingKind; method: FundingMethod; amount: number; details: string }) =>
    call<FundingRequest>('/funding', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(req) }),
  uploadDocument: (account: string, kind: string, file: File) =>
    call<KycDocument>(`/kyc/documents?account=${encodeURIComponent(account)}`, {
      method: 'POST',
      headers: { 'content-type': file.type || 'application/octet-stream', 'x-filename': file.name, 'x-doc-kind': kind },
      body: file,
    }),
};
