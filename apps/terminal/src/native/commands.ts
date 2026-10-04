import type { ConnectionState, Timeframe } from '../api/types';
import { invokeNative } from './tauri';

/** Mirrors `NotifyEvent` in apps/desktop/src-tauri/src/commands.rs (serde tag = kind). */
export type NotifyEvent =
  | { kind: 'fill'; symbol: string; side: string; lots: string; price: string }
  | { kind: 'price_alert'; symbol: string; price: string; message?: string }
  | { kind: 'position_closed'; symbol: string; reason: 'sl' | 'tp' | 'stop_out'; lots: string; price: string; profit: string };

export const openChartWindow = (symbol: string, timeframe?: Timeframe) =>
  invokeNative<string>('open_chart_window', { symbol, timeframe });

export const setConnectionStatus = (status: ConnectionState, latencyMs?: number) =>
  invokeNative<void>('set_connection_status', { status, latencyMs: latencyMs == null ? null : Math.round(latencyMs) });

export const notifyEvent = (event: NotifyEvent) => invokeNative<void>('notify_event', { event });

/** OS keychain access (main window only; chart windows are not granted these). */
export const credentials = {
  set: (account: string, secret: string) => invokeNative<void>('credential_set', { account, secret }, true),
  get: async (account: string) => (await invokeNative<string | null>('credential_get', { account }, true)) ?? null,
  delete: (account: string) => invokeNative<void>('credential_delete', { account }, true),
};
