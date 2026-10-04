/**
 * Thin wrapper over the Tauri IPC. In a plain browser (or tests without a mock)
 * every call is a no-op so the terminal stays a normal web app.
 */
type InvokeFn = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

let override: InvokeFn | null = null;

/** True inside the fxvps desktop shell (Tauri 2 injects `__TAURI_INTERNALS__`). */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** Test hook: replace the IPC transport. Pass `null` to restore. */
export function setInvokeForTests(fn: InvokeFn | null): void {
  override = fn;
}

/**
 * Invokes a desktop command. Resolves `undefined` outside Tauri. Errors are
 * swallowed (logged) unless `rethrow` is set: native extras must never break trading.
 */
export async function invokeNative<T>(cmd: string, args?: Record<string, unknown>, rethrow = false): Promise<T | undefined> {
  if (!override && !isTauri()) return undefined;
  try {
    if (override) return await override<T>(cmd, args);
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (rethrow) throw e;
    console.warn(`[native] ${cmd} failed`, e);
    return undefined;
  }
}
