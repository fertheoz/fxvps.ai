"use client";
import * as React from "react";
import { Badge, Button, Label } from "@/components/ui/primitives";
import { api, useApiMutation } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { AlgoTestRow } from "@/lib/api";
import type { Group } from "@/lib/schemas";

/** Parça 14: a group's bid / ask offset formulas with a live sandbox. */
export function AlgoFields({ draft, set }: { draft: Group; set: <K extends keyof Group>(k: K, v: Group[K]) => void }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const algo = draft.algo ?? { bid: "", ask: "" };
  const upd = (patch: Partial<{ bid: string; ask: string }>) => {
    const next = { ...algo, ...patch };
    set("algo", next.bid.trim() || next.ask.trim() ? next : null);
  };
  const [rows, setRows] = React.useState<AlgoTestRow[] | null>(null);
  const [err, setErr] = React.useState<string | null>(null);
  const test = useApiMutation((v: { bid: string; ask: string; group: string }) => api().testAlgo(v, actor), (r) => { setRows(r.rows); setErr(r.error ?? null); });
  return (
    <div className="col-span-full grid gap-2 rounded-md border border-border p-3" data-testid="algo-fields">
      <div className="text-sm font-medium">{t("groups.algo")}</div>
      <p className="text-xs text-muted-foreground">{t("groups.algoHint")}</p>
      <div className="grid gap-2 sm:grid-cols-2">
        <Label>{t("groups.algoBid")}<input className="rounded-md border border-border bg-background p-2 font-mono text-xs" value={algo.bid} onChange={(e) => upd({ bid: e.target.value })} placeholder="if(net < -2, 2, 0)" data-testid="algo-bid" /></Label>
        <Label>{t("groups.algoAsk")}<input className="rounded-md border border-border bg-background p-2 font-mono text-xs" value={algo.ask} onChange={(e) => upd({ ask: e.target.value })} placeholder="clamp(net * 1.5, 0, 20) + if(news, 5, 0)" data-testid="algo-ask" /></Label>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" onClick={() => test.mutate({ bid: algo.bid, ask: algo.ask, group: draft.name })} disabled={test.isPending || (!algo.bid.trim() && !algo.ask.trim())} data-testid="algo-test">{t("groups.algoTest")}</Button>
        <span className="text-xs text-muted-foreground">{t("groups.algoVars")}</span>
      </div>
      {err && <div className="text-xs text-destructive">{err}</div>}
      {test.error && <div className="text-xs text-destructive">{String(test.error instanceof Error ? test.error.message : test.error)}</div>}
      {rows && rows.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-xs tabular-nums">
            <thead><tr>{[t("positions.symbol"), "spread", "net", "vol%", "hour", "news", t("groups.algoBidPts"), t("groups.algoAskPts"), t("groups.algoNow"), t("groups.algoResult")].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.symbol} className="border-t border-border/60">
                  <td className="px-2 py-0.5 font-medium">{r.symbol}</td>
                  <td className="px-2 py-0.5">{f.num(r.vars.spread, 1)}</td>
                  <td className="px-2 py-0.5">{f.num(r.vars.net)}</td>
                  <td className="px-2 py-0.5">{f.num(r.vars.vol)}</td>
                  <td className="px-2 py-0.5">{r.vars.hour}</td>
                  <td className="px-2 py-0.5">{r.vars.news ? "1" : "0"}</td>
                  <td className="px-2 py-0.5">{f.num(r.bidPoints, 1)}</td>
                  <td className="px-2 py-0.5">{f.num(r.askPoints, 1)}</td>
                  <td className="px-2 py-0.5">{r.bidNow} / {r.askNow}</td>
                  <td className="px-2 py-0.5">{r.bid} / {r.ask} {r.crossed && <Badge tone="danger">{t("groups.algoCrossed")}</Badge>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {rows && rows.length === 0 && !err && <div className="text-xs text-muted-foreground">{t("common.noResults")}</div>}
    </div>
  );
}
