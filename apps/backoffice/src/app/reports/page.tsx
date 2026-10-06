"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { Download } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Button, PageHeader, Pnl, Stat, Tabs } from "@/components/ui/primitives";
import { BookBadge, SideBadge } from "@/components/badges";
import { useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Trade } from "@/lib/schemas";
import type { ExecutionRow, ExecutionSummary, LpExecution, RevenueRow, RevenueTotals, Statement } from "@/lib/api";
import { formatMinorPlain } from "@/lib/money";
import { downloadCsv, toCsv } from "@/lib/utils";

const tc = createColumnHelper<Trade>();
const sc = createColumnHelper<Statement>();
const lc = createColumnHelper<LpExecution>();
const rc = createColumnHelper<RevenueRow>();
const xc = createColumnHelper<ExecutionRow>();
const xs = createColumnHelper<ExecutionSummary>();
type Tab = "trades" | "statements" | "lp" | "execution" | "revenue";

const pts = (v: number | null | undefined, digits = 1) => (v === null || v === undefined ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(digits)}`);
const pct = (v: number) => `${(v * 100).toFixed(0)}%`;
/** Slippage colour: red when the client lost ground, green when the price improved. */
const slipTone = (v: number | null | undefined) => (v === null || v === undefined || v === 0 ? undefined : v > 0 ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400");

export default function ReportsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const [tab, setTab] = React.useState<Tab>("trades");
  const trades = useApiQuery("listTrades");
  const statements = useApiQuery("statements");
  const lp = useApiQuery("listLpExecutions", [], { live: 5000 });
  const revenue = useApiQuery("revenue", [], { live: 5000 });
  const execution = useApiQuery("execution", [], { live: 5000 });

  const execCols = [
    xc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    xc.accessor("login", { header: t("clients.login") }),
    xc.accessor("symbol", { header: t("positions.symbol") }),
    xc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    xc.accessor("type", { header: t("reports.kind") }),
    xc.accessor("lots", { header: t("positions.lots"), cell: (c) => <span className="tabular-nums">{c.row.original.filledLots < c.getValue() ? `${c.row.original.filledLots} / ${c.getValue()}` : c.getValue()}</span> }),
    xc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    xc.accessor("requested", { header: t("reports.requested"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("fill", { header: t("reports.fillPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("clientSlipPts", { header: t("reports.clientSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue())}</span> }),
    xc.accessor("lpPrice", { header: t("reports.lpPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("capturePts", { header: t("reports.capture"), cell: (c) => <span className="tabular-nums">{pts(c.getValue())}</span> }),
    xc.accessor("lpLatencyMs", { header: t("reports.lpLatency"), cell: (c) => <span className="tabular-nums">{c.getValue() === null ? "—" : `${Math.round(c.getValue() ?? 0)} ms`}</span> }),
    xc.accessor("lpFills", { header: t("reports.lpFills"), cell: (c) => <span className="tabular-nums">{c.row.original.book === "A" ? c.getValue() : "—"}{c.row.original.rearmed ? " ↻" : ""}</span> }),
    xc.accessor("status", { header: t("reports.status"), cell: (c) => <span className={c.getValue() === "rejected" ? "text-red-600 dark:text-red-400" : undefined} title={c.row.original.reason ?? undefined}>{c.getValue()}</span> }),
  ];
  const sumCols = [
    xs.accessor("symbol", { header: t("positions.symbol") }),
    xs.accessor("orders", { header: t("reports.orders") }),
    xs.accessor("fillRate", { header: t("reports.fillRate"), cell: (c) => pct(c.getValue()) }),
    xs.accessor("partialRate", { header: t("reports.partialRate"), cell: (c) => pct(c.getValue()) }),
    xs.accessor("rejectRate", { header: t("reports.rejectRate"), cell: (c) => pct(c.getValue()) }),
    xs.accessor("avgSlipPts", { header: t("reports.avgSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue(), 2)}</span> }),
    xs.accessor("p95SlipPts", { header: t("reports.p95Slip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue(), 2)}</span> }),
    xs.accessor("improvedRate", { header: t("reports.improved"), cell: (c) => pct(c.getValue()) }),
    xs.accessor("avgCapturePts", { header: t("reports.capture"), cell: (c) => <span className="tabular-nums">{pts(c.getValue(), 2)}</span> }),
    xs.accessor("avgLatencyMs", { header: t("reports.lpLatency"), cell: (c) => <span className="tabular-nums">{c.getValue() ? `${Math.round(c.getValue())} ms` : "—"}</span> }),
  ];

  const lpCols = [
    lc.accessor("createdAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    lc.accessor("symbol", { header: t("positions.symbol") }),
    lc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    lc.accessor("lots", { header: t("positions.lots") }),
    lc.accessor("filledLots", { header: t("reports.filled") }),
    lc.accessor("avgPrice", { header: t("reports.lpPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() || "—"}</span> }),
    lc.display({ id: "clientPrice", header: t("reports.clientPrice"), cell: (c) => <span className="tabular-nums">{c.row.original.clients.map((x) => x.price).join(", ") || "—"}</span> }),
    lc.display({ id: "clients", header: t("reports.clients"), cell: (c) => c.row.original.clients.map((x) => `${x.login} · ${x.lots}`).join(", ") }),
    lc.accessor("status", { header: t("reports.status"), cell: (c) => <span className={c.getValue() === "rejected" ? "text-red-600 dark:text-red-400" : undefined} title={c.row.original.reason ?? undefined}>{c.getValue()}</span> }),
    lc.display({ id: "execId", header: t("reports.execId"), cell: (c) => <span className="font-mono text-xs">{c.row.original.fills.map((x) => x.execId).join(", ")}</span> }),
  ];
  const leg = (k: "client" | "broker" | "lp", label: "reports.clientLeg" | "reports.brokerLeg" | "reports.lpLeg") =>
    rc.accessor(k, { header: t(label), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> });
  const revCols = [
    rc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    rc.accessor("login", { header: t("clients.login") }),
    rc.accessor("symbol", { header: t("positions.symbol") }),
    rc.accessor("lots", { header: t("positions.lots") }),
    rc.accessor("kind", { header: t("reports.kind") }),
    rc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    rc.accessor("price", { header: t("reports.clientPrice"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    rc.accessor("lpPrice", { header: t("reports.lpPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    leg("client", "reports.clientLeg"), leg("broker", "reports.brokerLeg"), leg("lp", "reports.lpLeg"),
    rc.accessor("ref", { header: t("reports.ref"), cell: (c) => <span className="font-mono text-xs">{c.getValue()}</span> }),
  ];
  const totals = (x: RevenueTotals | undefined, sub: string) => (
    <>
      <Stat label={t("reports.markup")} value={x ? f.money(x.markup) : "…"} sub={sub} tone={x && x.markup < 0 ? "down" : "up"} />
      <Stat label={t("reports.commission")} value={x ? f.money(x.commission) : "…"} sub={sub} />
      <Stat label={t("reports.bBook")} value={x ? f.money(x.bBook) : "…"} sub={sub} tone={x && x.bBook < 0 ? "down" : "up"} />
      <Stat label={t("reports.totalRevenue")} value={x ? f.money(x.total) : "…"} sub={sub} tone={x && x.total < 0 ? "down" : "up"} />
    </>
  );

  const tradeCols = [
    tc.accessor("closedAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    tc.accessor("login", { header: t("clients.login") }),
    tc.accessor("symbol", { header: t("positions.symbol") }),
    tc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    tc.accessor("lots", { header: t("positions.lots") }),
    tc.accessor("openPrice", { header: t("positions.open") }),
    tc.accessor("closePrice", { header: t("reports.closing") }),
    tc.accessor("pnl", { header: t("positions.pnl"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
    tc.accessor("commission", { header: t("reports.commission"), cell: (c) => f.money(c.getValue()) }),
    tc.accessor("swap", { header: t("reports.swap"), cell: (c) => f.money(c.getValue()) }),
    tc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];
  const m = (k: keyof Statement) => sc.accessor(k, { header: t(`reports.${k}` as "reports.opening"), cell: (c) => <span className="tabular-nums">{f.money(c.getValue() as number, c.row.original.currency)}</span> });
  const stmtCols = [
    sc.accessor("login", { header: t("clients.login") }),
    sc.accessor("name", { header: t("clients.name") }),
    m("opening"), m("deposits"), m("withdrawals"),
    sc.accessor("pnl", { header: t("positions.pnl"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue(), c.row.original.currency)}</Pnl> }),
    m("commission"), m("swap"), m("closing"),
  ];

  const exportCsv = () => {
    if (tab === "lp") {
      const rows = (lp.data ?? []).map((x) => [x.id, x.createdAt, x.symbol, x.side, x.lots, x.filledLots, x.avgPrice, x.status, x.fills.map((y) => y.execId).join(" "), x.clients.map((y) => `${y.login}@${y.price}`).join(" ")]);
      downloadCsv("lp-executions.csv", toCsv(["id", "created_at", "symbol", "side", "lots", "filled", "lp_price", "status", "exec_ids", "clients"], rows));
    } else if (tab === "execution") {
      const rows = (execution.data?.rows ?? []).map((x) => [x.id, x.at, x.login, x.symbol, x.side, x.type, x.lots, x.filledLots, x.book, x.requested ?? "", x.fill ?? "", x.clientSlipPts ?? "", x.lpPrice ?? "", x.capturePts ?? "", x.lpLatencyMs ?? "", x.lpFills, x.rearmed ? 1 : 0, x.status, x.reason ?? ""]);
      downloadCsv("execution.csv", toCsv(["id", "at", "login", "symbol", "side", "type", "lots", "filled", "book", "requested", "fill", "client_slip_pts", "lp_price", "capture_pts", "lp_latency_ms", "lp_fills", "rearmed", "status", "reason"], rows));
    } else if (tab === "revenue") {
      const rows = (revenue.data?.rows ?? []).map((x) => [x.id, x.at, x.login, x.symbol, x.lots, x.price, x.lpPrice ?? "", x.kind, x.book, formatMinorPlain(x.client, "USD"), formatMinorPlain(x.broker, "USD"), formatMinorPlain(x.lp, "USD"), x.ref]);
      downloadCsv("revenue.csv", toCsv(["id", "at", "login", "symbol", "lots", "client_price", "lp_price", "kind", "book", "client", "broker", "lp", "ref"], rows));
    } else if (tab === "trades") {
      const rows = (trades.data ?? []).map((x) => [x.id, x.closedAt, x.login, x.symbol, x.side, x.lots, x.openPrice, x.closePrice, formatMinorPlain(x.pnl, "USD"), formatMinorPlain(x.commission, "USD"), formatMinorPlain(x.swap, "USD"), x.book]);
      downloadCsv("trades.csv", toCsv(["id", "closed_at", "login", "symbol", "side", "lots", "open", "close", "pnl", "commission", "swap", "book"], rows));
    } else {
      const rows = (statements.data ?? []).map((s) => [s.login, s.name, s.currency, ...(["opening", "deposits", "withdrawals", "pnl", "commission", "swap", "closing"] as const).map((k) => formatMinorPlain(s[k], s.currency))]);
      downloadCsv("statements.csv", toCsv(["login", "name", "currency", "opening", "deposits", "withdrawals", "pnl", "commission", "swap", "closing"], rows));
    }
  };

  return (
    <div data-testid="page-reports">
      <PageHeader title={t("reports.title")}>
        {actor.can("reports.export") && <Button variant="outline" onClick={exportCsv} data-testid="export-csv"><Download className="h-4 w-4" />{t("common.export")}</Button>}
      </PageHeader>
      <div className="mb-3">
        <Tabs value={tab} onChange={setTab} items={[{ value: "trades", label: t("reports.trades") }, { value: "statements", label: t("reports.statements") }, { value: "lp", label: t("reports.lp") }, { value: "execution", label: t("reports.execution") }, { value: "revenue", label: t("reports.revenue") }]} />
      </div>
      {tab === "trades" && <DataTable data={trades.data ?? []} columns={tradeCols} getRowId={(x) => x.id} />}
      {tab === "statements" && <DataTable data={statements.data ?? []} columns={stmtCols} getRowId={(x) => String(x.login)} />}
      {tab === "lp" && <DataTable data={lp.data ?? []} columns={lpCols} getRowId={(x) => x.id} />}
      {tab === "execution" && (
        <div data-testid="execution-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.slipHint")}</p>
          <h3 className="mb-1 text-sm font-medium">{t("reports.bySymbol")}</h3>
          <div className="mb-4"><DataTable data={execution.data?.bySymbol ?? []} columns={sumCols} getRowId={(x) => x.symbol} /></div>
          <DataTable data={execution.data?.rows ?? []} columns={execCols} getRowId={(x) => x.id} />
        </div>
      )}
      {tab === "revenue" && (
        <div data-testid="revenue-report">
          <div className="mb-3 grid grid-cols-2 gap-3 xl:grid-cols-4">{totals(revenue.data?.last24h, t("reports.last24h"))}</div>
          <div className="mb-3 grid grid-cols-2 gap-3 xl:grid-cols-4">{totals(revenue.data?.total, t("reports.allTime"))}</div>
          <DataTable data={revenue.data?.rows ?? []} columns={revCols} getRowId={(x) => x.id} />
        </div>
      )}
    </div>
  );
}
