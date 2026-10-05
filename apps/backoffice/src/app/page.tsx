"use client";
import { AlertTriangle } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle, PageHeader, Stat } from "@/components/ui/primitives";
import { ExposureChart, FlowChart, PnlChart } from "@/components/charts";
import { FixBadge } from "@/components/badges";
import { useApiQuery } from "@/lib/queries";
import { useFormat, useT } from "@/lib/hooks";

export default function DashboardPage() {
  const t = useT();
  const f = useFormat();
  const stats = useApiQuery("dashboard", [], { live: 5000 });
  const exp = useApiQuery("exposure", [], { live: 5000 });
  const fix = useApiQuery("listFixSessions", [], { live: 5000 });
  const lpx = useApiQuery("listLpExecutions", [], { live: 5000 });
  const compact = (v: number) => f.money(v, "USD", { compact: true });
  const s = stats.data;
  const mismatches = (exp.data ?? []).filter((e) => Math.abs(e.aBookLots - e.lpLots) > 0.001);

  return (
    <div data-testid="page-dashboard">
      <PageHeader title={t("nav.dashboard")} />
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
        <Stat label={t("dash.activeAccounts")} value={s ? `${s.activeAccounts}` : "…"} sub={s ? `/ ${s.totalAccounts}` : undefined} />
        <Stat label={t("dash.depositsToday")} value={s ? f.money(s.depositsToday) : "…"} tone="up" />
        <Stat label={t("dash.withdrawalsToday")} value={s ? f.money(s.withdrawalsToday) : "…"} tone="down" />
        <Stat label={t("dash.aBookPnl")} value={s ? f.money(s.aBookPnl) : "…"} tone={s && s.aBookPnl < 0 ? "down" : "up"} />
        <Stat label={t("dash.bBookPnl")} value={s ? f.money(s.bBookPnl) : "…"} tone={s && s.bBookPnl < 0 ? "down" : "up"} />
        <Stat label={t("dash.lpHealth")} value={s ? `${s.lpUp} / ${s.lpTotal}` : "…"} tone={s && s.lpUp < s.lpTotal ? "down" : "up"} />
      </div>

      <div className="mt-4 grid gap-4 xl:grid-cols-3">
        <Card className="xl:col-span-2">
          <CardHeader><CardTitle>{t("dash.exposure")}</CardTitle></CardHeader>
          <CardContent>{exp.data ? <ExposureChart data={exp.data} fmt={compact} /> : t("common.loading")}</CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("nav.lp")}</CardTitle></CardHeader>
          <CardContent className="grid gap-2">
            {fix.data?.map((x) => (
              <div key={x.id} className="flex items-center justify-between gap-2 text-sm">
                <span className="truncate">{x.lp} <span className="text-muted-foreground">{x.kind}</span></span>
                <span className="flex items-center gap-2 tabular-nums text-xs text-muted-foreground">
                  {x.status === "logged_on" ? `${f.num(x.latencyMs)} ms` : ""}
                  <FixBadge status={x.status} />
                </span>
              </div>
            ))}
            {mismatches.length > 0 && (
              <div className="mt-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs">
                <div className="flex items-center gap-1.5 font-medium"><AlertTriangle className="h-3.5 w-3.5" /> {t("dash.lpMismatch")}</div>
                {mismatches.map((m) => <div key={m.symbol} className="tabular-nums">{m.symbol}: A-book {m.aBookLots} / LP {m.lpLots}</div>)}
              </div>
            )}
          </CardContent>
        </Card>
        <Card className="xl:col-span-2">
          <CardHeader><CardTitle>{t("dash.pnlChart")}</CardTitle></CardHeader>
          <CardContent>{s ? <PnlChart data={s.pnlSeries} fmt={compact} /> : t("common.loading")}</CardContent>
        </Card>
        <Card className="xl:col-span-3" data-testid="dash-lp-executions">
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
                    <th className="pr-3 font-normal">{t("reports.status")}</th>
                    <th className="font-normal">{t("reports.execId")}</th>
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
                      <td className={`pr-3 ${x.status === "rejected" ? "text-red-600 dark:text-red-400" : ""}`} title={x.reason ?? undefined}>{x.status}</td>
                      <td className="font-mono text-xs text-muted-foreground">{x.fills.map((y) => y.execId).join(", ")}</td>
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
