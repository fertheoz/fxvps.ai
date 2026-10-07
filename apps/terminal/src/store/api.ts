import type { TradingApi } from '@fxvps/trading-core';
import { MockTradingApi } from '@fxvps/trading-core';
import { WsTradingApi } from '../api/ws';
import { isAllowedWsUrl, resolveGateway } from './connection';
import { sessionToken, useSession } from './session';
import { RafBatcher } from '@fxvps/trading-core';
import type { ChartObjects, Quote } from '@fxvps/trading-core';
import { setObjectsSaver, useTerminal } from './terminal';
import { translate } from '../i18n';
import { pipsToDistance, volumeToLots } from '@fxvps/trading-core';

let api: TradingApi | null = null;

/**
 * `?api=ws&url=ws://host:port/ws&token=<jwt>` or a gateway saved by the connect dialog
 * (sessionStorage) selects the gateway adapter; default is the in-browser mock.
 * See store/connection.ts.
 */
export function createApiFromLocation(search: string): TradingApi {
  const { gateway, cleanedSearch } = resolveGateway(search);
  if (cleanedSearch !== search && typeof history !== 'undefined' && typeof location !== 'undefined') {
    // Do not leave the bearer token in the address bar / history.
    history.replaceState(history.state, '', `${location.pathname}${cleanedSearch}${location.hash}`);
  }
  // Without a pasted dev token the identity session supplies (and silently refreshes)
  // the access token; it is read on every (re)connect, and only ever sent to an
  // allow-listed gateway origin (a manually entered foreign gateway gets no token).
  if (gateway) {
    const trusted = isAllowedWsUrl(gateway.url);
    if (!trusted && !gateway.token) console.warn(`[fxvps] gateway ${gateway.url} is not allow-listed; session token withheld`);
    const ws = new WsTradingApi({ url: gateway.url, token: () => gateway.token || (trusted ? sessionToken() : '') });
    // A silently refreshed session token is pushed to the gateway in place (no reconnect).
    if (!gateway.token && trusted) {
      useSession.subscribe((s, prev) => {
        if (s.accessToken && s.accessToken !== prev.accessToken) ws.renewToken();
      });
    }
    return ws;
  }
  return new MockTradingApi();
}

export function isGatewayApi(a: TradingApi | null = api): boolean {
  return a instanceof WsTradingApi;
}

/** Lightning button / back online / tab visible: reconnect the gateway now. */
export function reconnectNow(): void {
  if (api instanceof WsTradingApi) api.reconnectNow();
}

let wired = false;
/** Keeps the gateway's retry policy in sync with the store and reconnects on
 *  network / visibility recovery (no page refresh needed). */
function wireReconnect(instance: TradingApi): void {
  if (!(instance instanceof WsTradingApi)) return;
  instance.setRetryEvery(useTerminal.getState().reconnectEveryMs);
  if (wired || typeof window === 'undefined') return;
  wired = true;
  useTerminal.subscribe((s, prev) => {
    if (s.reconnectEveryMs !== prev.reconnectEveryMs && api instanceof WsTradingApi) api.setRetryEvery(s.reconnectEveryMs);
  });
  window.addEventListener('online', reconnectNow);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') reconnectNow();
  });
}

export function getApi(): TradingApi {
  if (!api) throw new Error('TradingApi not initialised');
  return api;
}

/** Wire an API into the store. Returns a teardown function. */
export async function bootstrap(instance: TradingApi): Promise<() => void> {
  api = instance;
  wireReconnect(instance);
  const store = useTerminal.getState();
  const offEvents = instance.onEvent((e) => useTerminal.getState().applyEvent(e));
  await instance.connect();
  const [symbols, accounts] = await Promise.all([instance.getSymbols(), instance.getAccounts()]);
  store.setReference(symbols, accounts);
  await Promise.all(
    accounts.map(async (a) => useTerminal.getState().setHistory(a.id, await instance.getHistory(a.id))),
  );
  // Chart objects (lines, alerts) come from the server-side preferences of each account.
  setObjectsSaver((accountId, json) => instance.setPrefs(accountId, json));
  await Promise.all(
    accounts.map(async (a) => {
      const raw = await instance.getPrefs(a.id).catch(() => null);
      const parsed = parseObjects(raw);
      if (parsed) useTerminal.getState().setObjects(a.id, parsed, false);
    }),
  );
  // Ticks are conflated per symbol and flushed once per animation frame.
  const batcher = new RafBatcher<Quote>((qs) => useTerminal.getState().applyQuotes(qs));
  const offQuotes = instance.subscribeQuotes(
    symbols.map((s) => s.name),
    (qs) => qs.forEach((q) => batcher.push(q.symbol, q)),
  );
  return () => {
    offQuotes();
    offEvents();
    setObjectsSaver(null);
    instance.disconnect();
    api = null;
  };
}

/** Stored preferences -> chart objects (unknown fields are ignored, bad JSON = nothing). */
export function parseObjects(raw: string | null): ChartObjects | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<ChartObjects>;
    return {
      lines: Array.isArray(v.lines) ? v.lines.filter((l) => l && typeof l.symbol === 'string' && typeof l.price === 'number') : [],
      alerts: Array.isArray(v.alerts) ? v.alerts.filter((a) => a && typeof a.symbol === 'string' && typeof a.price === 'number') : [],
      shapes: Array.isArray(v.shapes) ? v.shapes.filter((x) => x && typeof x.symbol === 'string' && x.a && x.b && (x.kind === 'trend' || x.kind === 'rect' || x.kind === 'fib')) : [],
      templates: Array.isArray(v.templates) ? v.templates.filter((x) => x && typeof x.name === 'string' && x.indicators && x.settings) : [],
    };
  } catch {
    return null;
  }
}

/** Helpers that call the API and surface the result as a toast. */
export const trade = {
  async market(symbol: string, side: 'buy' | 'sell', volume: number) {
    const s = useTerminal.getState();
    if (!s.activeAccountId) return;
    const spec = s.symbols[symbol];
    const maxDeviation = spec && s.maxDeviationPips > 0 ? pipsToDistance(s.maxDeviationPips, spec) : undefined;
    const r = await getApi().placeOrder({ accountId: s.activeAccountId, symbol, side, type: 'market', volume, maxDeviation });
    const t = (k: Parameters<typeof translate>[1], v?: Record<string, string | number>) => translate(useTerminal.getState().lang, k, v);
    if (r.ok)
      s.toast('ok', t('toast.filled', { side: side.toUpperCase(), lots: volumeToLots(volume), symbol, price: r.price ?? '' }));
    else s.toast('error', t('toast.rejected', { error: r.error }));
    return r;
  },
};
