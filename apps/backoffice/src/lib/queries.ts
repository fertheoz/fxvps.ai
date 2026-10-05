"use client";
import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, getApi, type AdminApi } from "./api";
import { isLive } from "./auth";
import { useT } from "./hooks";
import type { MessageKey } from "./i18n";
import { useToast } from "@/components/shell/providers";

export const api = () => getApi();

/** AdminApi methods usable as queries (everything except the push channel). */
export type ApiMethod = Exclude<keyof AdminApi, "subscribe">;
type Fn<K extends ApiMethod> = AdminApi[K];

// --- live (SSE) connection state ------------------------------------------
let liveConnected = false;
const liveListeners = new Set<() => void>();
function setLiveConnected(v: boolean) {
  if (v === liveConnected) return;
  liveConnected = v;
  liveListeners.forEach((l) => l());
}
export function useLiveConnected(): boolean {
  return React.useSyncExternalStore(
    (cb) => {
      liveListeners.add(cb);
      return () => liveListeners.delete(cb);
    },
    () => liveConnected,
    () => false,
  );
}

/**
 * `live` is a polling interval used only while no server push is connected;
 * with the HTTP adapter's SSE stream, queries refetch on invalidation instead.
 */
export function useApiQuery<K extends ApiMethod>(key: K, args: Parameters<Fn<K>> = [] as unknown as Parameters<Fn<K>>, opts: { live?: number; enabled?: boolean } = {}) {
  const pushed = useLiveConnected();
  return useQuery({
    queryKey: [key, ...args],
    queryFn: () => (getApi()[key] as (...a: unknown[]) => Promise<Awaited<ReturnType<Fn<K>>>>)(...args),
    refetchInterval: pushed ? false : opts.live,
    enabled: opts.enabled,
  });
}

/** Subscribes to the adapter's push channel and invalidates queries by topic. */
export function LiveUpdates({ enabled = true }: { enabled?: boolean }) {
  const qc = useQueryClient();
  React.useEffect(() => {
    const sub = getApi().subscribe;
    if (!enabled || !sub) return;
    return sub(
      (topics) => {
        if (topics.includes("*")) void qc.invalidateQueries();
        else for (const t of topics) void qc.invalidateQueries({ queryKey: [t] });
      },
      setLiveConnected,
    );
  }, [qc, enabled]);
  return null;
}

/** Mutation that invalidates all queries and reports errors as toasts. */
/** API errors in plain words (no permission ids). */
export function humanError(e: unknown, t: (k: MessageKey) => string): string {
  if (e instanceof ApiError) {
    if (e.code === "mfa_required") return t("mfa.error");
    if (e.code === "forbidden") return t("err.forbidden");
  }
  return e instanceof Error ? e.message : String(e);
}

/** false while the session lacks the 2FA login that mutating calls need (live mode only). */
export function useMfaOk(): boolean {
  const live = isLive();
  const me = useApiQuery("getMe", [], { enabled: live });
  return !live || me.data?.mfaOk !== false;
}

export function useApiMutation<A, R>(fn: (a: A) => Promise<R>, onOk?: (r: R) => void) {
  const qc = useQueryClient();
  const toast = useToast();
  const t = useT();
  return useMutation({
    mutationFn: fn,
    onSuccess: (r) => {
      void qc.invalidateQueries();
      onOk?.(r);
    },
    onError: (e) => toast(humanError(e, t), "error"),
  });
}
