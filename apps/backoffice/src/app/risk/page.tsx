"use client";
import * as React from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, PageHeader, Select } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { MarginCallRow } from "@/lib/api";

export default function RiskPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const exp = useApiQuery("exposure", [], { live: 5000 });
  const mc = useApiQuery("marginCalls", [], { live: 5000 });
  const presets = useApiQuery("esmaPresets");
  const groups = useApiQuery("listGroups");
  const [target, setTarget] = React.useState<Record<string, string>>({});
  const apply = useApiMutation((v: { preset: string; group: string }) => api().applyPreset(v.preset, v.group, actor), (g) => toast(`${g.name}: 1:${g.leverage}`));

  const stopOuts = (mc.data ?? []).filter((r) => r.state === "stop_out");
  const calls = (mc.data ?? []).filter((r) => r.state === "margin_call");

  const list = (rows: MarginCallRow[], testId: string) => (
    <div className="grid gap-1" data-testid={testId}>
      {rows.length === 0 && <p className="text-sm text-muted-foreground">{t("common.noResults")}</p>}
      {rows.map((r) => (
        <div key={r.client.id} className="flex items-center justify-between gap-2 border-b border-border py-1.5 text-sm last:border-0">
          <span><span className="font-medium tabular-nums">#{r.client.login}</span> <span className="text-muted-foreground">{r.client.name}</span></span>
          <span className="flex items-center gap-2 tabular-nums">
            <span className="text-xs text-muted-foreground">{f.money(r.client.equity)} / {f.money(r.client.margin)}</span>
            <Badge tone={r.state === "stop_out" ? "danger" : "warning"}>{f.num(r.marginLevel, 1)}%</Badge>
          </span>
        </div>
      ))}
    </div>
  );

  return (
    <div data-testid="page-risk">
      <PageHeader title={t("risk.title")} />
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>{t("risk.topExposure")}</CardTitle></CardHeader>
          <CardContent>
            <table className="w-full text-sm">
              <thead className="text-xs text-muted-foreground"><tr><th className="text-left">{t("positions.symbol")}</th><th className="text-right">Net lots</th><th className="text-right">A</th><th className="text-right">B</th><th className="text-right">LP</th><th className="text-right">Notional</th></tr></thead>
              <tbody>
                {(exp.data ?? []).slice(0, 10).map((e) => (
                  <tr key={e.symbol} className="border-t border-border tabular-nums">
                    <td className="py-1 font-medium">{e.symbol}</td>
                    <td className="text-right">{e.netLots}</td>
                    <td className="text-right">{e.aBookLots}</td>
                    <td className="text-right">{e.bBookLots}</td>
                    <td className={`text-right ${Math.abs(e.lpLots - e.aBookLots) > 0.001 ? "text-amber-600 dark:text-amber-400 font-semibold" : ""}`}>{e.lpLots}</td>
                    <td className="text-right">{f.money(e.notional, "USD", { compact: true })}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("risk.stopOutQueue")} ({stopOuts.length})</CardTitle></CardHeader>
          <CardContent>{list(stopOuts, "stopout-queue")}</CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("risk.marginCalls")} ({calls.length})</CardTitle></CardHeader>
          <CardContent>{list(calls, "margin-calls")}</CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("risk.presets")}</CardTitle></CardHeader>
          <CardContent className="grid gap-3">
            {(presets.data ?? []).map((p) => (
              <div key={p.id} className="grid gap-2 rounded-md border border-border p-2 text-sm">
                <div className="font-medium">{p.label}</div>
                <div className="flex flex-wrap gap-1.5 text-xs">
                  <Badge>1:{p.maxLeverage}</Badge><Badge>MC {p.marginCallPct}%</Badge><Badge>SO {p.stopOutPct}%</Badge>
                  {p.negativeBalanceProtection && <Badge tone="success">{t("risk.nbp")}</Badge>}
                </div>
                {actor.can("risk.edit") && (
                  <div className="flex gap-2">
                    <Select value={target[p.id] ?? ""} onChange={(e) => setTarget({ ...target, [p.id]: e.target.value })} aria-label="group" className="flex-1">
                      <option value="">—</option>
                      {(groups.data ?? []).map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
                    </Select>
                    <Button size="sm" disabled={!target[p.id] || apply.isPending} onClick={() => apply.mutate({ preset: p.id, group: target[p.id]! })}>{t("risk.apply")}</Button>
                  </div>
                )}
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
