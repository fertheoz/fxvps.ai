"use client";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getApi, type AdminApi } from "./api";
import { useToast } from "@/components/shell/providers";

export const api = () => getApi();

type Fn<K extends keyof AdminApi> = AdminApi[K];

export function useApiQuery<K extends keyof AdminApi>(key: K, args: Parameters<Fn<K>> = [] as unknown as Parameters<Fn<K>>, opts: { live?: number } = {}) {
  return useQuery({
    queryKey: [key, ...args],
    queryFn: () => (getApi()[key] as (...a: unknown[]) => Promise<Awaited<ReturnType<Fn<K>>>>)(...args),
    refetchInterval: opts.live,
  });
}

/** Mutation that invalidates all queries and reports errors as toasts. */
export function useApiMutation<A, R>(fn: (a: A) => Promise<R>, onOk?: (r: R) => void) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: fn,
    onSuccess: (r) => {
      void qc.invalidateQueries();
      onOk?.(r);
    },
    onError: (e) => toast(e instanceof Error ? e.message : String(e), "error"),
  });
}
