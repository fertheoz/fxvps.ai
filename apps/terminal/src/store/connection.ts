/**
 * Backend selection. Default is the in-browser mock. A gateway connection is chosen by
 *  - URL params `?api=ws&url=ws://host:port/ws&token=<jwt>` (dev login), or
 *  - the connect dialog,
 * and persisted in sessionStorage (per tab, cleared when the tab closes). The token is
 * removed from the address bar after it has been read.
 *
 * Security (finding G3): a `?url=` link can point anywhere, so it is only accepted for
 * allow-listed gateway origins (see `isAllowedWsUrl`), and the identity session token
 * is only ever sent to allow-listed origins (store/api.ts).
 */
export interface GatewayConfig {
  url: string;
  token: string;
}

export const STORAGE_KEY = 'fxvps.gateway';

function storage(): Storage | undefined {
  try {
    return typeof sessionStorage === 'undefined' ? undefined : sessionStorage;
  } catch {
    return undefined;
  }
}

export function loadGateway(): GatewayConfig | null {
  try {
    const raw = storage()?.getItem(STORAGE_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<GatewayConfig>;
    return typeof v.url === 'string' && v.url ? { url: v.url, token: typeof v.token === 'string' ? v.token : '' } : null;
  } catch {
    return null;
  }
}

export function saveGateway(cfg: GatewayConfig | null): void {
  const s = storage();
  if (!s) return;
  if (cfg) s.setItem(STORAGE_KEY, JSON.stringify(cfg));
  else s.removeItem(STORAGE_KEY);
}

export function isValidWsUrl(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === 'ws:' || u.protocol === 'wss:';
  } catch {
    return false;
  }
}

const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]', '::1']);

/** Parses a comma separated origin list (`wss://gw.fxvps.ai,wss://gw2.fxvps.ai`). */
export function parseOrigins(raw: string | undefined): string[] {
  return (raw ?? '')
    .split(',')
    .map((s) => s.trim().replace(/\/+$/, ''))
    .filter(Boolean);
}

export interface WsPolicy {
  /** Build-time `VITE_ALLOWED_WS_ORIGINS`. */
  allowed: string[];
  /** Origin of the page (`https://terminal.fxvps.ai`). */
  pageOrigin: string;
  /** Dev build (`import.meta.env.DEV`). */
  dev: boolean;
}

export function defaultWsPolicy(): WsPolicy {
  return {
    allowed: parseOrigins(import.meta.env.VITE_ALLOWED_WS_ORIGINS),
    pageOrigin: typeof location === 'undefined' ? '' : location.origin,
    dev: Boolean(import.meta.env.DEV),
  };
}

/**
 * May the terminal send credentials to this gateway URL? Allowed: origins listed in
 * `VITE_ALLOWED_WS_ORIGINS`, the page's own origin (`https:` page -> `wss:` same host),
 * and loopback gateways when the build is a dev build or the page itself is served
 * from loopback (local development / e2e). Everything else is denied.
 */
export function isAllowedWsUrl(url: string, policy: WsPolicy = defaultWsPolicy()): boolean {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  if (u.protocol !== 'ws:' && u.protocol !== 'wss:') return false;
  const origin = `${u.protocol}//${u.host}`;
  if (policy.allowed.includes(origin)) return true;
  let page: URL | null;
  try {
    page = policy.pageOrigin ? new URL(policy.pageOrigin) : null;
  } catch {
    page = null;
  }
  if (page) {
    const wsProto = page.protocol === 'https:' ? 'wss:' : 'ws:';
    if (u.protocol === wsProto && u.host === page.host) return true;
  }
  const pageIsLoopback = page !== null && LOOPBACK.has(page.hostname);
  return (policy.dev || pageIsLoopback) && LOOPBACK.has(u.hostname);
}

/**
 * Resolves the gateway config from the query string (which wins and is persisted) or
 * sessionStorage. `?api=mock` forces the mock and forgets a stored gateway.
 * Returns the config and the query string with `token` stripped.
 */
export function resolveGateway(
  search: string,
  policy: WsPolicy = defaultWsPolicy(),
): { gateway: GatewayConfig | null; cleanedSearch: string } {
  const params = new URLSearchParams(search);
  const api = params.get('api');
  const url = params.get('url');
  let gateway: GatewayConfig | null;
  if (api === 'mock') {
    saveGateway(null);
    gateway = null;
  } else if (api === 'ws' && url && isValidWsUrl(url)) {
    if (isAllowedWsUrl(url, policy)) {
      gateway = { url, token: params.get('token') ?? loadGateway()?.token ?? '' };
      saveGateway(gateway);
    } else {
      // A link must not redirect the terminal (and its credentials) to a foreign gateway.
      console.warn(`[fxvps] gateway ${url} is not in VITE_ALLOWED_WS_ORIGINS; ignored`);
      gateway = loadGateway();
    }
  } else {
    gateway = loadGateway();
  }
  params.delete('token');
  const q = params.toString();
  return { gateway, cleanedSearch: q ? `?${q}` : '' };
}
