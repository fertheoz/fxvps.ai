import type { TradingApi } from '../api/types';
import { MockTradingApi } from '../api/mock';
import { WsTradingApi } from '../api/ws';
import { resolveGateway } from './connection';
import { sessionToken } from './session';
import { RafBatcher } from '../lib/rafBatcher';
import type { Quote } from '../api/types';
import { useTerminal } from './terminal';
import { translate } from '../i18n';
import { volumeToLots } from '../lib/money';

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
  // the access token; it is read on every (re)connect.
  if (gateway) return new WsTradingApi({ url: gateway.url, token: () => gateway.token || sessionToken() });
  return new MockTradingApi();
}

export function isGatewayApi(a: TradingApi | null = api): boolean {
  return a instanceof WsTradingApi;
}

export function getApi(): TradingApi {
  if (!api) throw new Error('TradingApi not initialised');
  return api;
}

/** Wire an API into the store. Returns a teardown function. */
export async function bootstrap(instance: TradingApi): Promise<() => void> {
  api = instance;
  const store = useTerminal.getState();
  const offEvents = instance.onEvent((e) => useTerminal.getState().applyEvent(e));
  await instance.connect();
  const [symbols, accounts] = await Promise.all([instance.getSymbols(), instance.getAccounts()]);
  store.setReference(symbols, accounts);
  await Promise.all(
    accounts.map(async (a) => useTerminal.getState().setHistory(a.id, await instance.getHistory(a.id))),
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
    instance.disconnect();
    api = null;
  };
}

/** Helpers that call the API and surface the result as a toast. */
export const trade = {
  async market(symbol: string, side: 'buy' | 'sell', volume: number) {
    const s = useTerminal.getState();
    if (!s.activeAccountId) return;
    const r = await getApi().placeOrder({ accountId: s.activeAccountId, symbol, side, type: 'market', volume });
    const t = (k: Parameters<typeof translate>[1], v?: Record<string, string | number>) => translate(useTerminal.getState().lang, k, v);
    if (r.ok)
      s.toast('ok', t('toast.filled', { side: side.toUpperCase(), lots: volumeToLots(volume), symbol, price: r.price ?? '' }));
    else s.toast('error', t('toast.rejected', { error: r.error }));
    return r;
  },
};
