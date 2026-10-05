import { create } from 'zustand';
import { IdentityClient, IdentityError, type TokenResponse } from '../auth/identity';

/**
 * Identity session (services/identity). The access token is kept in memory only and
 * refreshed silently before it expires; the refresh token is an HttpOnly cookie the
 * page never sees. Only active when `VITE_IDENTITY_URL` is configured; the mock
 * backend and the dev `?token=` flow do not need it.
 */

export interface AccessClaims {
  sub: string;
  email: string;
  exp: number;
  iat?: number;
  accounts: string[];
  roles: string[];
  amr: string[];
}

/** Reads (does not verify) the JWT payload; the gateway verifies the signature. */
export function decodeJwt(token: string): AccessClaims | null {
  const part = token.split('.')[1];
  if (!part) return null;
  try {
    const bin = atob(part.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((part.length + 3) % 4));
    const bytes = Uint8Array.from(bin, (ch) => ch.charCodeAt(0));
    const c = JSON.parse(new TextDecoder().decode(bytes)) as Partial<AccessClaims>;
    if (typeof c.sub !== 'string' || typeof c.exp !== 'number') return null;
    return {
      sub: c.sub,
      email: typeof c.email === 'string' ? c.email : '',
      exp: c.exp,
      iat: c.iat,
      accounts: Array.isArray(c.accounts) ? c.accounts.map(String) : [],
      roles: Array.isArray(c.roles) ? c.roles.map(String) : [],
      amr: Array.isArray(c.amr) ? c.amr.map(String) : [],
    };
  } catch {
    return null;
  }
}

export const ACCOUNT_KEY = 'fxvps.account';
/** Refresh this many seconds before expiry (and never later than 80% of the lifetime). */
export const REFRESH_MARGIN_S = 60;

export type SessionStatus = 'idle' | 'checking' | 'anonymous' | 'authenticated';

export interface SessionState {
  client: IdentityClient | null;
  status: SessionStatus;
  accessToken: string | null;
  claims: AccessClaims | null;
  /** Trading account chosen in the account picker. */
  account: string | null;
  /** Last refresh failure (session ended). */
  error: string | null;
  configure(baseUrl: string | null, fetchImpl?: typeof fetch): void;
  /** Silent sign-in from the refresh cookie (page load). */
  restore(): Promise<boolean>;
  accept(r: TokenResponse): void;
  refresh(): Promise<boolean>;
  /** Revokes the session. `keepState` leaves the UI as is (caller reloads the page next). */
  logout(opts?: { keepState?: boolean }): Promise<void>;
  selectAccount(id: string): void;
}

function store(): Storage | undefined {
  try {
    return typeof sessionStorage === 'undefined' ? undefined : sessionStorage;
  } catch {
    return undefined;
  }
}

let timer: ReturnType<typeof setTimeout> | undefined;
let inflight: Promise<boolean> | null = null;

function delayMs(c: AccessClaims, nowMs: number): number {
  const now = nowMs / 1000;
  const lifetime = c.iat ? c.exp - c.iat : c.exp - now;
  // Short-lived (privileged, 60 s) tokens: the margin must not swallow the whole lifetime.
  const margin = Math.min(REFRESH_MARGIN_S, lifetime * 0.25);
  const at = Math.min(c.exp - margin, (c.iat ?? now) + lifetime * 0.8);
  return Math.max(5_000, (at - now) * 1000);
}

export const useSession = create<SessionState>((set, get) => ({
  client: null,
  status: 'idle',
  accessToken: null,
  claims: null,
  account: null,
  error: null,

  configure(baseUrl, fetchImpl) {
    clearTimeout(timer);
    inflight = null;
    set({
      client: baseUrl ? new IdentityClient(baseUrl, fetchImpl) : null,
      status: baseUrl ? 'anonymous' : 'idle',
      accessToken: null,
      claims: null,
      account: store()?.getItem(ACCOUNT_KEY) ?? null,
      error: null,
    });
  },

  async restore() {
    if (!get().client) return false;
    set({ status: 'checking' });
    const ok = await get().refresh();
    if (!ok) set({ status: 'anonymous', error: null });
    return ok;
  },

  accept(r) {
    const claims = decodeJwt(r.access_token);
    if (!claims) {
      set({ status: 'anonymous', accessToken: null, claims: null, error: 'invalid token' });
      return;
    }
    const prev = get().account;
    const account = prev && claims.accounts.includes(prev) ? prev : claims.accounts.length === 1 ? claims.accounts[0]! : null;
    set({ status: 'authenticated', accessToken: r.access_token, claims, account, error: null });
    clearTimeout(timer);
    timer = setTimeout(() => void get().refresh(), delayMs(claims, Date.now()));
  },

  refresh() {
    const c = get().client;
    if (!c) return Promise.resolve(false);
    // Single flight: concurrent callers share one rotation (a second rotation with
    // the already-rotated cookie would look like token theft and end the session).
    inflight ??= c
      .refresh()
      .then((r) => {
        get().accept(r);
        return get().status === 'authenticated';
      })
      .catch((e: unknown) => {
        const was = get().status === 'authenticated';
        clearTimeout(timer);
        // Network failure: keep the current token and retry shortly.
        if (!(e instanceof IdentityError) && was) {
          timer = setTimeout(() => void get().refresh(), 10_000);
          return true;
        }
        set({
          status: 'anonymous',
          accessToken: null,
          claims: null,
          error: was ? (e instanceof Error ? e.message : String(e)) : null,
        });
        return false;
      })
      .finally(() => {
        inflight = null;
      });
    return inflight;
  },

  async logout(opts) {
    clearTimeout(timer);
    const c = get().client;
    if (!opts?.keepState) set({ status: 'anonymous', accessToken: null, claims: null, error: null });
    store()?.removeItem(ACCOUNT_KEY);
    try {
      await c?.logout();
    } catch {
      /* cookie cleared server side or already gone */
    }
  },

  selectAccount(id) {
    store()?.setItem(ACCOUNT_KEY, id);
    set({ account: id });
  },
}));

/** Current access token for the gateway `Auth` frame. */
export function sessionToken(): string | undefined {
  return useSession.getState().accessToken ?? undefined;
}

/** `VITE_IDENTITY_URL` (build time). Empty → identity login disabled. */
export function identityUrl(): string | null {
  const v = (import.meta.env.VITE_IDENTITY_URL as string | undefined)?.trim();
  return v ? v : null;
}
