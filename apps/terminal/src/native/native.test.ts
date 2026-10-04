import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Deal, TradingApi, TradingEvent } from '../api/types';
import {
  credentials,
  dealNotification,
  invokeNative,
  isTauri,
  openChartWindow,
  parseChartView,
  setInvokeForTests,
  startNativeBridge,
} from '.';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async (cmd: string) => `real:${cmd}`) }));

const deal = (over: Partial<Deal>): Deal => ({
  id: 'd1',
  accountId: 'a',
  positionId: 'p',
  symbol: 'EURUSD',
  side: 'buy',
  entry: 'in',
  volume: 10,
  price: 1.1,
  time: 0,
  profit: 0,
  commission: 0,
  reason: 'client',
  ...over,
});

function fakeApi() {
  let listener: ((e: TradingEvent) => void) | null = null;
  const api = {
    onEvent: (l: (e: TradingEvent) => void) => {
      listener = l;
      return () => (listener = null);
    },
  } as unknown as TradingApi;
  return { api, emit: (e: TradingEvent) => listener?.(e), active: () => listener !== null };
}

afterEach(() => {
  setInvokeForTests(null);
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

describe('native bridge', () => {
  it('is a no-op in the browser', async () => {
    expect(isTauri()).toBe(false);
    expect(await openChartWindow('EURUSD', 'H1')).toBeUndefined();
    const { api, active } = fakeApi();
    startNativeBridge(api);
    expect(active()).toBe(false);
  });

  it('uses @tauri-apps/api when __TAURI_INTERNALS__ is present', async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(isTauri()).toBe(true);
    expect(await openChartWindow('EURUSD')).toBe('real:open_chart_window');
  });

  it('passes command arguments', async () => {
    const calls: [string, unknown][] = [];
    setInvokeForTests(async (cmd, args) => {
      calls.push([cmd, args]);
      return 'chart-EURUSD' as never;
    });
    expect(await openChartWindow('EURUSD', 'H1')).toBe('chart-EURUSD');
    expect(calls[0]).toEqual(['open_chart_window', { symbol: 'EURUSD', timeframe: 'H1' }]);
  });

  it('swallows errors unless asked to rethrow', async () => {
    setInvokeForTests(async () => {
      throw new Error('denied');
    });
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
    expect(await invokeNative('notify_event')).toBeUndefined();
    await expect(credentials.get('refresh')).rejects.toThrow('denied');
  });

  it('credential helpers map to keychain commands', async () => {
    const store = new Map<string, string>();
    setInvokeForTests(async (cmd, a) => {
      const args = a as { account: string; secret?: string };
      if (cmd === 'credential_set') store.set(args.account, args.secret!);
      if (cmd === 'credential_delete') store.delete(args.account);
      return (cmd === 'credential_get' ? (store.get(args.account) ?? null) : undefined) as never;
    });
    await credentials.set('refresh', 's3');
    expect(await credentials.get('refresh')).toBe('s3');
    await credentials.delete('refresh');
    expect(await credentials.get('refresh')).toBeNull();
  });

  it('forwards connection changes and trade notifications', () => {
    const calls: [string, unknown][] = [];
    setInvokeForTests(async (cmd, args) => {
      calls.push([cmd, args]);
      return undefined as never;
    });
    const { api, emit } = fakeApi();
    const off = startNativeBridge(api, true);
    emit({ type: 'connection', state: 'connected', latencyMs: 41 });
    emit({ type: 'connection', state: 'connected', latencyMs: 42 }); // same bucket: skipped
    emit({ type: 'connection', state: 'reconnecting' });
    emit({ type: 'deal', deal: deal({}) });
    emit({ type: 'deal', deal: deal({ entry: 'out', reason: 'client' }) }); // manual close: skipped
    emit({ type: 'deal', deal: deal({ entry: 'out', reason: 'stop_out', profit: -12345 }) });
    off();
    expect(calls.map((c) => c[0])).toEqual(['set_connection_status', 'set_connection_status', 'notify_event', 'notify_event']);
    expect(calls[0]![1]).toEqual({ status: 'connected', latencyMs: 41 });
    expect(calls[3]![1]).toMatchObject({ event: { kind: 'position_closed', reason: 'stop_out', profit: '-123.45' } });
  });

  it('maps deals to notifications', () => {
    expect(dealNotification(deal({}))).toMatchObject({ kind: 'fill', side: 'buy', lots: '0.10' });
    expect(dealNotification(deal({ entry: 'out', reason: 'tp', profit: 500 }))).toMatchObject({ kind: 'position_closed', reason: 'tp' });
    expect(dealNotification(deal({ entry: 'out', reason: 'client' }))).toBeNull();
  });

  it('parses the detached chart view', () => {
    expect(parseChartView('?view=chart&symbol=eurusd&tf=H1')).toEqual({ symbol: 'EURUSD', timeframe: 'H1' });
    expect(parseChartView('?view=chart&symbol=XAUUSD&tf=bogus')).toEqual({ symbol: 'XAUUSD', timeframe: 'M5' });
    expect(parseChartView('?view=chart&symbol=<x>')).toBeNull();
    expect(parseChartView('?symbol=EURUSD')).toBeNull();
  });
});
