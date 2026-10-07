"use client";
import * as React from "react";
import { Badge, Card, CardContent, CardHeader, CardTitle, PageHeader } from "@/components/ui/primitives";
import { useApiQuery } from "@/lib/queries";
import { useFormat, useT } from "@/lib/hooks";

/** Institution (MT5 bridge partner) view: its own sessions, account, positions and deals. */
export default function PartnerPage() {
  const t = useT();
  const f = useFormat();
  const { data } = useApiQuery("getPartnerOverview", [], { live: 3000 });
  const insts = data?.institutions ?? [];
  return (
    <div data-testid="page-partner" className="space-y-4">
      <PageHeader title={t("partner.title")} />
      <p className="text-sm text-muted-foreground">
        {t("partner.intro")} <code className="rounded bg-muted px-1">{data?.endpoint ?? "wss://trade.fxvps.ai/bridge"}</code>
      </p>
      {data && insts.length === 0 && <Card><CardContent className="py-6 text-sm text-muted-foreground">{t("partner.none")}</CardContent></Card>}
      {insts.map((i) => {
        const s = i.sessions[0];
        const acc = data?.accounts.find((a) => a.id === i.account);
        return (
          <Card key={i.id} data-testid={`partner-${i.id}`}>
            <CardHeader><CardTitle className="flex items-center gap-3">{i.name || i.id}<span className="font-mono text-xs text-muted-foreground">{i.id}</span></CardTitle></CardHeader>
            <CardContent className="grid gap-4 text-sm md:grid-cols-2">
              <div className="space-y-1">
                <div className="text-xs uppercase text-muted-foreground">{t("partner.session")}</div>
                {s ? (
                  <>
                    <div><Badge tone="success">{t("partner.online")}</Badge> <span className="text-muted-foreground">{s.server} · v{s.plugin}</span></div>
                    <div className="tabular-nums">{t("partner.orders")}: <b>{s.orders}</b> · {t("partner.fills")}: <b>{s.fills}</b> · {t("partner.rejects")}: <b className={s.rejects ? "text-red-600 dark:text-red-400" : ""}>{s.rejects}</b></div>
                    <div>{t("partner.reconcile")}: {s.reconcile_ok == null ? "—" : s.reconcile_ok ? <Badge tone="success">OK</Badge> : <Badge tone="danger">diff</Badge>}{s.fill_ms_p50 ? <span className="ml-3 text-muted-foreground">{t("partner.latency")}: {s.fill_ms_p50} / {s.fill_ms_p99} ms</span> : null}</div>
                    {i.activity && <div className="text-muted-foreground">{t("partner.volume")}: 24h <b>{i.activity.h24.lots}</b> lot ({i.activity.h24.deals}) · 7d <b>{i.activity.d7.lots}</b> lot ({i.activity.d7.deals})</div>}
                  </>
                ) : <Badge tone="muted">{t("partner.offline")}</Badge>}
              </div>
              <div className="space-y-1 tabular-nums">
                <div className="text-xs uppercase text-muted-foreground">{t("partner.account")} #{i.account}</div>
                {acc ? (
                  <>
                    <div>{t("partner.balance")}: <b>{f.money(acc.balance, acc.currency)}</b></div>
                    <div>{t("partner.equity")}: <b>{f.money(acc.equity, acc.currency)}</b></div>
                    <div>{t("partner.margin")}: {f.money(acc.margin, acc.currency)} · {t("partner.free")}: {f.money(acc.equity - acc.margin, acc.currency)}</div>
                  </>
                ) : <span className="text-muted-foreground">—</span>}
              </div>
            </CardContent>
          </Card>
        );
      })}
      {data && insts.length > 0 && (
        <>
          <Card className="overflow-x-auto">
            <CardHeader><CardTitle>{t("partner.positions")}</CardTitle></CardHeader>
            <table className="w-full text-sm tabular-nums">
              <thead className="bg-muted/50 text-xs text-muted-foreground"><tr>{["#", "Symbol", "Side", "Lots", "Open", "Current", "P&L"].map((h) => <th key={h} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr></thead>
              <tbody>
                {data.positions.length === 0 && <tr><td colSpan={7} className="px-3 py-4 text-center text-muted-foreground">{t("partner.noPositions")}</td></tr>}
                {data.positions.map((p) => (
                  <tr key={p.id} className="border-t border-border">
                    <td className="px-3 py-1">{p.id}</td><td className="px-3">{p.symbol}</td>
                    <td className={`px-3 ${p.side === "buy" ? "text-emerald-600" : "text-red-600"}`}>{p.side}</td>
                    <td className="px-3">{p.lots}</td><td className="px-3">{p.openPrice}</td><td className="px-3">{p.currentPrice}</td>
                    <td className={`px-3 ${p.pnl < 0 ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400"}`}>{f.money(p.pnl, "USD")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Card>
          <Card className="overflow-x-auto">
            <CardHeader><CardTitle>{t("partner.deals")}</CardTitle></CardHeader>
            <table className="w-full text-sm tabular-nums">
              <thead className="bg-muted/50 text-xs text-muted-foreground"><tr>{["Time", "Symbol", "Side", "Lots", "Price", "P&L"].map((h) => <th key={h} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr></thead>
              <tbody>
                {data.deals.length === 0 && <tr><td colSpan={6} className="px-3 py-4 text-center text-muted-foreground">{t("partner.noDeals")}</td></tr>}
                {data.deals.map((d, k) => (
                  <tr key={k} className="border-t border-border">
                    <td className="px-3 py-1 whitespace-nowrap">{String(d.time ?? d.openedAt ?? "")}</td>
                    <td className="px-3">{String(d.symbol ?? "")}</td>
                    <td className="px-3">{String(d.side ?? "")}</td>
                    <td className="px-3">{String(d.lots ?? "")}</td>
                    <td className="px-3">{String(d.price ?? "")}</td>
                    <td className="px-3">{typeof d.pnl === "number" ? f.money(d.pnl, "USD") : String(d.pnl ?? "")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </Card>
        </>
      )}
    </div>
  );
}
