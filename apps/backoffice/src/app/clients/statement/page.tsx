"use client";
import * as React from "react";
import { Printer } from "lucide-react";
import { Button } from "@/components/ui/primitives";
import { useApiQuery } from "@/lib/queries";
import { useFormat, useT } from "@/lib/hooks";

const noop = () => () => {};
function useParams(): { login: number | null; from: string; to: string } {
  const search = React.useSyncExternalStore(noop, () => window.location.search, () => "");
  const p = new URLSearchParams(search);
  const v = p.get("login");
  return { login: v ? Number(v) : null, from: p.get("from") ?? "", to: p.get("to") ?? "" };
}

/** Print-friendly account statement (`?login=&from=YYYY-MM-DD&to=YYYY-MM-DD`): the browser's
 *  print dialog produces the PDF; e-mail delivery arrives with the mailer (stage 12). */
export default function StatementPage() {
  const t = useT();
  const f = useFormat();
  const { login, from, to } = useParams();
  const clients = useApiQuery("listClients", [{}]);
  const trades = useApiQuery("listTrades");
  const statements = useApiQuery("statements");
  const positions = useApiQuery("listPositions");
  const settings = useApiQuery("getSettings");
  const client = (clients.data ?? []).find((c) => c.login === login);
  if (!login) return <div className="p-6 text-sm text-muted-foreground">?login=</div>;
  if (!client) return <div className="p-6 text-sm text-muted-foreground">{t("common.loading")}</div>;
  const fromMs = from ? Date.parse(from) : 0;
  const toMs = to ? Date.parse(to) + 86_400_000 : Number.POSITIVE_INFINITY;
  const ts = (trades.data ?? []).filter((x) => x.login === login && Date.parse(x.closedAt) >= fromMs && Date.parse(x.closedAt) < toMs);
  const ps = (positions.data ?? []).filter((p) => p.login === login);
  const st = (statements.data ?? []).find((s) => s.login === login);
  const money = (v: number) => f.money(v, client.currency);
  const sum = (k: "pnl" | "commission" | "swap") => ts.reduce((a, x) => a + x[k], 0);
  const lots = ts.reduce((a, x) => a + x.lots, 0);
  return (
    <div className="mx-auto max-w-4xl bg-background p-4 text-foreground print:p-0" data-testid="page-statement">
      <style>{`@media print { header, nav, aside, .no-print { display: none !important } main { padding: 0 !important } }`}</style>
      <div className="no-print mb-3 flex items-center gap-2">
        <Button variant="outline" onClick={() => window.print()} data-testid="statement-print"><Printer className="h-4 w-4" />{t("stmt.print")}</Button>
        <span className="text-xs text-muted-foreground">?from=YYYY-MM-DD&to=YYYY-MM-DD</span>
      </div>
      <div className="flex items-start justify-between border-b border-border pb-3">
        <div>
          <div className="text-lg font-semibold">{settings.data?.brokerName ?? "fxvps.ai"}</div>
          <div className="text-xs text-muted-foreground">{settings.data?.brokerLei ? `LEI ${settings.data.brokerLei}` : ""}</div>
        </div>
        <div className="text-right text-sm">
          <div className="font-semibold">{t("stmt.title")}</div>
          <div>{client.name} · #{client.login} · {client.group} · {client.currency}</div>
          <div className="text-xs text-muted-foreground">{t("stmt.period")}: {from || "—"} → {to || "—"} · {t("stmt.generated")} {f.date(new Date().toISOString())}</div>
        </div>
      </div>

      <h2 className="mt-4 mb-1 text-sm font-semibold">{t("stmt.summary")}</h2>
      <table className="w-full text-sm tabular-nums">
        <tbody>
          {st && ([["opening", "reports.opening"], ["deposits", "reports.deposits"], ["withdrawals", "reports.withdrawals"], ["commission", "reports.commission"], ["swap", "reports.swap"], ["pnl", "positions.pnl"], ["closing", "reports.closing"]] as const).map(([k, label]) => (
            <tr key={k} className="border-b border-border/60"><td className="py-1 text-muted-foreground">{t(label)}</td><td className="py-1 text-right">{money(st[k])}</td></tr>
          ))}
          <tr><td className="py-1 text-muted-foreground">{t("stmt.closedTrades")} ({t("stmt.period").toLowerCase()})</td><td className="py-1 text-right">{ts.length} · {lots.toFixed(2)} lot · {money(sum("pnl"))} · {t("reports.commission")} {money(sum("commission"))} · {t("reports.swap")} {money(sum("swap"))}</td></tr>
        </tbody>
      </table>

      <h2 className="mt-4 mb-1 text-sm font-semibold">{t("stmt.closedTrades")}</h2>
      <table className="w-full text-xs tabular-nums">
        <thead className="text-left text-muted-foreground"><tr><th className="py-1">{t("audit.at")}</th><th>ID</th><th>{t("positions.symbol")}</th><th>{t("positions.side")}</th><th className="text-right">{t("positions.lots")}</th><th className="text-right">{t("positions.open")}</th><th className="text-right">{t("reports.closing")}</th><th className="text-right">{t("reports.commission")}</th><th className="text-right">{t("reports.swap")}</th><th className="text-right">{t("positions.pnl")}</th></tr></thead>
        <tbody>
          {ts.map((x) => (
            <tr key={x.id} className="border-t border-border/60"><td className="py-0.5">{f.date(x.closedAt)}</td><td>{x.id}</td><td>{x.symbol}</td><td>{x.side}</td><td className="text-right">{x.lots.toFixed(2)}</td><td className="text-right">{x.openPrice}</td><td className="text-right">{x.closePrice}</td><td className="text-right">{money(x.commission)}</td><td className="text-right">{money(x.swap)}</td><td className="text-right">{money(x.pnl)}</td></tr>
          ))}
          {ts.length === 0 && <tr><td colSpan={10} className="py-2 text-muted-foreground">{t("dash.noData")}</td></tr>}
        </tbody>
      </table>

      <h2 className="mt-4 mb-1 text-sm font-semibold">{t("stmt.openPositions")}</h2>
      <table className="w-full text-xs tabular-nums">
        <thead className="text-left text-muted-foreground"><tr><th className="py-1">ID</th><th>{t("positions.symbol")}</th><th>{t("positions.side")}</th><th className="text-right">{t("positions.lots")}</th><th className="text-right">{t("positions.open")}</th><th className="text-right">{t("reports.swap")}</th><th className="text-right">{t("positions.pnl")}</th></tr></thead>
        <tbody>
          {ps.map((p) => (
            <tr key={p.id} className="border-t border-border/60"><td className="py-0.5">{p.id}</td><td>{p.symbol}</td><td>{p.side}</td><td className="text-right">{p.lots.toFixed(2)}</td><td className="text-right">{p.openPrice}</td><td className="text-right">{money(p.swap)}</td><td className="text-right">{money(p.pnl)}</td></tr>
          ))}
          {ps.length === 0 && <tr><td colSpan={7} className="py-2 text-muted-foreground">{t("dash.noData")}</td></tr>}
        </tbody>
      </table>
    </div>
  );
}
