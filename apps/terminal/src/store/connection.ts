/**
 * Backend selection. Default is the in-browser mock. A gateway connection is chosen by
 *  - URL params `?api=ws&url=ws://host:port/ws&token=<jwt>` (dev login), or
 *  - the connect dialog,
 * and persisted in sessionStorage (per tab, cleared when the tab closes). The token is
 * removed from the address bar after it has been read.
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

/**
 * Resolves the gateway config from the query string (which wins and is persisted) or
 * sessionStorage. `?api=mock` forces the mock and forgets a stored gateway.
 * Returns the config and the query string with `token` stripped.
 */
export function resolveGateway(search: string): { gateway: GatewayConfig | null; cleanedSearch: string } {
  const params = new URLSearchParams(search);
  const api = params.get('api');
  const url = params.get('url');
  let gateway: GatewayConfig | null;
  if (api === 'mock') {
    saveGateway(null);
    gateway = null;
  } else if (api === 'ws' && url && isValidWsUrl(url)) {
    gateway = { url, token: params.get('token') ?? loadGateway()?.token ?? '' };
    saveGateway(gateway);
  } else {
    gateway = loadGateway();
  }
  params.delete('token');
  const q = params.toString();
  return { gateway, cleanedSearch: q ? `?${q}` : '' };
}
