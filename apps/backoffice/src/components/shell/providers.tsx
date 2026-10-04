"use client";
import * as React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

interface Toast { id: number; text: string; tone: "ok" | "error" }
const ToastCtx = React.createContext<(text: string, tone?: Toast["tone"]) => void>(() => {});
export const useToast = () => React.useContext(ToastCtx);

export function Providers({ children }: { children: React.ReactNode }) {
  const [client] = React.useState(() => new QueryClient({ defaultOptions: { queries: { staleTime: 2_000, refetchOnWindowFocus: false } } }));
  const [toasts, setToasts] = React.useState<Toast[]>([]);
  const push = React.useCallback((text: string, tone: Toast["tone"] = "ok") => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, tone }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
  }, []);
  return (
    <QueryClientProvider client={client}>
      <ToastCtx.Provider value={push}>
        {children}
        <div className="fixed bottom-4 right-4 z-[60] grid gap-2" aria-live="polite">
          {toasts.map((t) => (
            <div key={t.id} role="status" className={`rounded-md border px-4 py-2 text-sm shadow-lg ${t.tone === "error" ? "border-red-500/40 bg-red-950 text-red-100" : "border-border bg-card"}`}>
              {t.text}
            </div>
          ))}
        </div>
      </ToastCtx.Provider>
    </QueryClientProvider>
  );
}
