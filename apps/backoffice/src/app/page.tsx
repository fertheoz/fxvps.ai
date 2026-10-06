"use client";
import * as React from "react";
import { AlertTriangle } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle, PageHeader, Pnl, Stat, Tabs } from "@/components/ui/primitives";
import { ExecutionChart, ExposureChart, FlowChart, RevenueChart, VolumeChart } from "@/components/charts";
import { AlertsCard } from "@/components/alerts";
import { FixBadge } from "@/components/badges";
import { useApiQuery } from "@/lib/queries";
import { useFormat, useT } from "@/lib/hooks";
import type { DashboardRange } from "@/lib/api";

const RANGES: DashboardRange[] = ["today", "24h", "7d", "30d"];

/** Change vs the previous period, as a signed percentage string (or null when the base is 0). */
function delta(now: number, prev: number): string | null {
  if (!prev) return null;
  const p = ((now - prev) / Math.abs(prev)) * 100;
  return `${p >= 0 ? "+" : ""}${p.toFixed(0)}%`;
}

export default function DashboardPage() {
  const t = useT();
  const f = useFormat();
  const [range, setRange] = React.useState<DashboardRange>("24h");
  const stats = useApiQuery("dashboard", [], { live: 5000 });
  const series = useApiQuery("dashboardSeries", [range], { live: 5000 });
  const exp = useApiQuery("exposure", [], { live: 5000 });
  const fix = useApiQuery("listFixSessions", [], { live: 5000 });
  const lpx = useApiQuery("listLpExecutions", [], { live: 5000 });
  const compact = (v: number) => f.money(v, "USD", { compact: true });
  const s = stats.data;
  const d = series.data;
  const mismatches = (exp.data ?? []).filter((e) => Math.abs(e.aBookLots - e.lpLots) > 0.001);
  const sub = (now: number | undefined, prev: number | undefined) => {
    if (now === undefined || prev === undefined) return undefined;
    const x = delta(now, prev);
    return x ? `${x} ${t("dash.vsPrev")}` : t("dash.noPrev");
  };
  const tone = (v: number | undefined): "up" | "down" | undefined => (v === undefined ? undefined : v < 0 ? "down" : "up");

  return (
    <div data-testid="page-dashboard">
      <PageHeader title={t("nav.dashboard")}>
        <Tabs value={range} onChange={setRange} items={RANGES.map((r) => ({ value: r, label: t(`dash.range.${r}`) }))} />
      </PageHeader>

      <div className="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-8">
        <Stat label={t("dash.revenue")} value={d ? f.money(d.totals.revenue) : "…"} sub={sub(d?.totals.revenue, d?.previous.revenue)} tone={tone(d?.totals.revenue)} />
        <Stat label={t("reports.markup")} value={d ? f.money(d.totals.markup) : "…"} sub={sub(d?.totals.markup, d?.previous.markup)} tone={tone(d?.totals.markup)} />
        <Stat label={t("reports.commission")} value={d ? f.money(d.totals.commission) : "…"} sub={sub(d?.totals.commission, d?.previous.commission)} />
        <Stat label={t("reports.bBook")} value={d ? f.money(d.totals.bBook) : "…"} sub={s ? `${t("dash.floating")} ${f.money(s.bBookPnl)}` : undefined} tone={tone(d?.totals.bBook)} />
        <Stat label={t("dash.volume")} value={d ? `${d.totals.lots.toFixed(2)} lot` : "…"} sub={sub(d?.totals.lots, d?.previous.lots)} />
        <Stat label={t("dash.orders")} value={d ? `${d.totals.orders}` : "…"} sub={d ? `${d.totals.rejects} ${t("reports.rejectRate").toLowerCase()} · ${(d.execution.fillRate * 100).toFixed(0)}% ${t("reports.fillRate").toLowerCase()}` : undefined} tone={d && d.totals.rejects > 0 ? "down" : undefined} />
        <Stat label={t("dash.activeAccounts")} value={s ? `${s.activeAccounts}` : "…"} sub={s ? `/ ${s.totalAccounts} · ${s.openPositions} ${t("dash.openPositions")}` : undefined} />
        <Stat label={t("dash.lpHealth")} value={s ? `${s.lpUp} / ${s.lpTotal}` : "…"} sub={d ? `p95 ${Math.round(d.execution.p95LatencyMs)} ms · ${t("reports.avgSlip")} ${d.execution.avgClientSlipPts.toFixed(2)}` : undefined} tone={s && s.lpUp < s.lpTotal ? "down" : "up"} />
      </div>

      <div className="mt-4 grid min-w-0 grid-cols-1 gap-4 xl:grid-cols-3">
        <Card className="min-w-0 overflow-hidden xl:col-span-2">
          <CardHeader><CardTitle>{t("dash.revenueChart")}</CardTitle></CardHeader>
          <CardContent>{d ? <RevenueChart data={d.buckets} fmt={compact} labels={[t("reports.markup"), t("reports.commission"), t("reports.bBook")]} /> : t("common.loading")}</CardContent>
        </Card>
        <Card className="min-w-0 overflow-hidden">
          <CardHeader><CardTitle>{t("dash.volumeChart")}</CardTitle></CardHeader>
          <CardContent>{d ? <VolumeChart data={d.buckets} labels={[t("positions.lots"), t("dash.orders"), t("reports.rejectRate")]} /> : t("common.loading")}</CardContent>
        </Card>

        <Card className="min-w-0 overflow-hidden xl:col-span-2">
          <CardHeader><CardTitle>{t("dash.execChart")}</CardTitle></CardHeader>
          <CardContent>{d ? <ExecutionChart data={d.buckets} labels={[t("reports.avgSlip"), t("reports.p95Latency")]} /> : t("common.loading")}</CardContent>
        </Card>
        <AlertsCard />

        <Card className="min-w-0 overflow-hidden xl:col-span-2">
          <CardHeader><CardTitle>{t("dash.exposure")}</CardTitle></CardHeader>
          <CardContent>{exp.data ? <ExposureChart data={exp.data} fmt={compact} /> : t("common.loading")}</CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("nav.lp")}</CardTitle></CardHeader>
          <CardContent className="grid gap-2">
            {fix.data?.map((x) => (
              <div key={x.id} className="flex items-center justify-between gap-2 text-sm">
                <span className="truncate">{x.lp} <span className="text-muted-foreground">{x.kind}</span></span>
                <FixBadge status={x.status} />
              </div>
            ))}
            {mismatches.length > 0 && (
              <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
                <div className="flex items-center gap-1.5 font-medium"><AlertTriangle className="h-3.5 w-3.5" /> {t("dash.lpMismatch")}</div>
                {mismatches.map((m) => <div key={m.symbol} className="tabular-nums">{m.symbol}: A-book {m.aBookLots} / LP {m.lpLots}</div>)}
              </div>
            )}
            {d && (
              <div className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1 text-xs tabular-nums">
                <span className="text-muted-foreground">{t("reports.p50Latency")}</span><span className="text-right">{Math.round(d.execution.p50LatencyMs)} ms</span>
                <span className="text-muted-foreground">{t("reports.p95Latency")}</span><span className="text-right">{Math.round(d.execution.p95LatencyMs)} ms</span>
                <span className="text-muted-foreground">{t("reports.fillRate")}</span><span className="text-right">{(d.execution.fillRate * 100).toFixed(0)}%</span>
                <span className="text-muted-foreground">{t("reports.avgSlip")}</span><span className="text-right">{d.execution.avgClientSlipPts.toFixed(2)}</span>
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader><CardTitle>{t("dash.topSymbols")}</CardTitle></CardHeader>
          <CardContent>
            {d && d.topSymbols.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noData")}</div>}
            <table className="w-full text-sm tabular-nums">
              <tbody>
                {d?.topSymbols.map((x) => (
                  <tr key={x.symbol} className="border-t border-border/60 first:border-0">
                    <td className="py-1 font-medium">{x.symbol}</td>
                    <td className="py-1 text-right text-muted-foreground">{x.lots.toFixed(2)} lot</td>
                    <td className="py-1 text-right"><Pnl value={x.revenue}>{f.money(x.revenue)}</Pnl></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("dash.clients")}</CardTitle></CardHeader>
          <CardContent>
            {d && d.winners.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noData")}</div>}
            <table className="w-full text-sm tabular-nums">
              <tbody>
                {d?.winners.map((x) => (
                  <tr key={`w${x.login}`} className="border-t border-border/60 first:border-0">
                    <td className="py-1"><span className="font-medium">{x.login}</span> <span className="text-muted-foreground">{x.name}</span></td>
                    <td className="py-1 text-right text-muted-foreground">{x.lots.toFixed(2)} lot</td>
                    <td className="py-1 text-right"><Pnl value={x.pnl}>{f.money(x.pnl)}</Pnl></td>
                  </tr>
                ))}
                {d?.losers.map((x) => (
                  <tr key={`l${x.login}`} className="border-t border-border/60">
                    <td className="py-1"><span className="font-medium">{x.login}</span> <span className="text-muted-foreground">{x.name}</span></td>
                    <td className="py-1 text-right text-muted-foreground">{x.lots.toFixed(2)} lot</td>
                    <td className="py-1 text-right"><Pnl value={x.pnl}>{f.money(x.pnl)}</Pnl></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("dash.risk")}</CardTitle></CardHeader>
          <CardContent>
            {d && d.risk.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noRisk")}</div>}
            <table className="w-full text-sm tabular-nums">
              <tbody>
                {d?.risk.map((x) => (
                  <tr key={x.login} className="border-t border-border/60 first:border-0">
                    <td className="py-1"><span className="font-medium">{x.login}</span> <span className="text-muted-foreground">{x.name}</span></td>
                    <td className={`py-1 text-right ${x.marginLevelPct < 100 ? "text-red-600 dark:text-red-400" : "text-amber-600 dark:text-amber-400"}`}>{x.marginLevelPct.toFixed(0)}%</td>
                    <td className="py-1 text-right text-muted-foreground">{f.money(x.equity)} / {f.money(x.margin)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>

        <Card className="xl:col-span-2" data-testid="dash-lp-executions">
          <CardHeader><CardTitle>{t("dash.lpExecutions")}</CardTitle></CardHeader>
          <CardContent className="overflow-x-auto">
            {lpx.data && lpx.data.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noExecutions")}</div>}
            {lpx.data && lpx.data.length > 0 && (
              <table className="w-full text-sm tabular-nums">
                <thead className="text-left text-xs text-muted-foreground">
                  <tr>
                    <th className="py-1 pr-3 font-normal">{t("audit.at")}</th>
                    <th className="pr-3 font-normal">{t("positions.symbol")}</th>
                    <th className="pr-3 font-normal">{t("positions.side")}</th>
                    <th className="pr-3 font-normal">{t("positions.lots")}</th>
                    <th className="pr-3 font-normal">{t("reports.lpPrice")}</th>
                    <th className="pr-3 font-normal">{t("reports.clientPrice")}</th>
                    <th className="pr-3 font-normal">{t("clients.login")}</th>
                    <th className="font-normal">{t("reports.status")}</th>
                  </tr>
                </thead>
                <tbody>
                  {lpx.data.slice(0, 8).map((x) => (
                    <tr key={x.id} className="border-t border-border/60">
                      <td className="py-1.5 pr-3 text-muted-foreground">{f.date(x.createdAt)}</td>
                      <td className="pr-3 font-medium">{x.symbol}</td>
                      <td className={`pr-3 ${x.side === "buy" ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400"}`}>{x.side}</td>
                      <td className="pr-3">{x.filledLots} / {x.lots}</td>
                      <td className="pr-3">{x.avgPrice || "—"}</td>
                      <td className="pr-3">{x.clients.map((c) => c.price).join(", ") || "—"}</td>
                      <td className="pr-3">{x.clients.map((c) => c.login).join(", ")}</td>
                      <td className={x.status === "rejected" ? "text-red-600 dark:text-red-400" : ""} title={x.reason ?? undefined}>{x.status}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("dash.depositChart")}</CardTitle></CardHeader>
          <CardContent>{s ? <FlowChart data={s.depositSeries} fmt={compact} labels={[t("reports.deposits"), t("reports.withdrawals")]} /> : t("common.loading")}</CardContent>
        </Card>
      </div>
    </div>
  );
}
