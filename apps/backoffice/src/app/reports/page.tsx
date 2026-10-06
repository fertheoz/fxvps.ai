"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { Download } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, PageHeader, Pnl, Stat, Tabs } from "@/components/ui/primitives";
import { BookBadge, SideBadge, ToxicityBadge } from "@/components/badges";
import { useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Trade } from "@/lib/schemas";
import type { BestExecutionRow, ClientFlowRow, IbRow, ExecutionRow, ExecutionSummary, LpExecution, RevenueRow, RevenueTotals, Statement, TransactionRow } from "@/lib/api";
import { formatMinorPlain } from "@/lib/money";
import { downloadCsv, toCsv } from "@/lib/utils";

const tc = createColumnHelper<Trade>();
const sc = createColumnHelper<Statement>();
const lc = createColumnHelper<LpExecution>();
const rc = createColumnHelper<RevenueRow>();
const xc = createColumnHelper<ExecutionRow>();
const xs = createColumnHelper<ExecutionSummary>();
const fc = createColumnHelper<ClientFlowRow>();
const txc = createColumnHelper<TransactionRow>();
const bxc = createColumnHelper<BestExecutionRow>();
const ibc = createColumnHelper<IbRow>();
type Tab = "trades" | "statements" | "lp" | "execution" | "revenue" | "flow" | "transactions" | "bestexec" | "ib";

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
  const flow = useApiQuery("clientFlow", [], { live: 10000, enabled: tab === "flow" });
  const [range, setRange] = React.useState<{ from: string; to: string }>({ from: "", to: "" });
  const tx = useApiQuery("transactions", [range.from || undefined, range.to || undefined], { enabled: tab === "transactions" });
  const bx = useApiQuery("bestExecution", [range.from || undefined, range.to || undefined], { enabled: tab === "bestexec" });
  const ib = useApiQuery("ibReport", [range.from || undefined, range.to || undefined], { enabled: tab === "ib" });

  const execCols = [
    xc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    xc.accessor("login", { header: t("clients.login") }),
    xc.accessor("name", { header: t("clients.name"), cell: (c) => c.getValue() ?? "—" }),
    xc.accessor("symbol", { header: t("positions.symbol") }),
    xc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    xc.accessor("type", { header: t("reports.kind") }),
    xc.accessor("lots", { header: t("positions.lots"), cell: (c) => <span className="tabular-nums">{c.row.original.filledLots < c.getValue() ? `${c.row.original.filledLots} / ${c.getValue()}` : c.getValue()}</span> }),
    xc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    xc.accessor("requested", { header: t("reports.requested"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("fill", { header: t("reports.fillPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("clientSlipPts", { header: t("reports.clientSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue())}</span> }),
    xc.accessor("lpPrice", { header: t("reports.lpPrice"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("lpSentPrice", { header: t("reports.lpSent"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span> }),
    xc.accessor("lpSlipPts", { header: t("reports.lpSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue())}</span> }),
    xc.accessor("attempts", { header: t("reports.attempts"), cell: (c) => <span className="tabular-nums">{c.row.original.book === "A" ? c.getValue() : "—"}</span> }),
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
    xs.accessor("avgLpSlipPts", { header: t("reports.lpSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue()) ?? ""}`}>{pts(c.getValue(), 2)}</span> }),
    xs.accessor("avgAttempts", { header: t("reports.attempts"), cell: (c) => <span className="tabular-nums">{c.getValue() ? c.getValue().toFixed(2) : "—"}</span> }),
    xs.accessor("p50LatencyMs", { header: t("reports.p50Latency"), cell: (c) => <span className="tabular-nums">{c.getValue() ? `${Math.round(c.getValue())} ms` : "—"}</span> }),
    xs.accessor("p95LatencyMs", { header: t("reports.p95Latency"), cell: (c) => <span className="tabular-nums">{c.getValue() ? `${Math.round(c.getValue())} ms` : "—"}</span> }),
  ];

  const lpCols = [
    lc.accessor("createdAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    lc.accessor("lp", { header: "LP", cell: (c) => c.getValue() ?? "—" }),
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

  const flowCols = [
    fc.accessor("login", { header: t("clients.login") }),
    fc.accessor("name", { header: t("clients.name"), cell: (c) => c.getValue() ?? "—" }),
    fc.accessor("group", { header: t("clients.group") }),
    fc.accessor("trades", { header: t("flow.trades") }),
    fc.accessor("avgHoldSecs", { header: t("flow.avgHold"), cell: (c) => { const v = c.getValue(); return v < 120 ? `${Math.round(v)} s` : v < 7200 ? `${Math.round(v / 60)} min` : `${(v / 3600).toFixed(1)} h`; } }),
    fc.accessor("shortHoldPct", { header: t("flow.shortHold"), cell: (c) => `${c.getValue().toFixed(0)}%` }),
    fc.accessor("winRate", { header: t("flow.winRate"), cell: (c) => `${c.getValue().toFixed(0)}%` }),
    fc.accessor("realisedPnl", { header: t("flow.realised"), cell: (c) => <Pnl value={c.getValue()}>{f.num(c.getValue(), 2)} {c.row.original.currency}</Pnl> }),
    fc.accessor("brokerPnl", { header: t("flow.broker"), cell: (c) => <Pnl value={c.getValue()}>{f.num(c.getValue(), 2)} {c.row.original.currency}</Pnl> }),
    fc.accessor("avgSlipGainPoints", { header: t("flow.slipGain"), cell: (c) => c.getValue().toFixed(2) }),
    fc.accessor("toxicity", { header: t("flow.toxicity"), cell: (c) => <ToxicityBadge score={c.getValue()} /> }),
  ];
  const txCols = [
    txc.accessor("tradingDateTime", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    txc.accessor("txId", { header: "ID" }),
    txc.accessor("clientLogin", { header: t("clients.login") }),
    txc.accessor("buyerId", { header: t("tx.buyer"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span> }),
    txc.accessor("sellerId", { header: t("tx.seller"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span> }),
    txc.accessor("instrument", { header: t("positions.symbol") }),
    txc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    txc.accessor("price", { header: t("positions.open") }),
    txc.accessor("quantityUnits", { header: t("tx.units"), cell: (c) => f.num(c.getValue(), 0) }),
    txc.accessor("notional", { header: t("tx.notional"), cell: (c) => f.num(c.getValue(), 2) }),
    txc.accessor("tradingCapacity", { header: t("tx.capacity"), cell: (c) => <Badge tone="muted">{c.getValue()}</Badge> }),
    txc.accessor("executionLp", { header: "LP", cell: (c) => c.getValue() ?? "—" }),
    txc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];
  const bxCols = [
    bxc.accessor("venue", { header: t("bestexec.venue") }),
    bxc.accessor("assetClass", { header: t("bestexec.class") }),
    bxc.accessor("orders", { header: t("dash.orders") }),
    bxc.accessor("fillRate", { header: t("reports.fillRate"), cell: (c) => `${(c.getValue() * 100).toFixed(1)}%` }),
    bxc.accessor("rejected", { header: t("lp.rejects") }),
    bxc.accessor("lots", { header: t("positions.lots"), cell: (c) => c.getValue().toFixed(2) }),
    bxc.accessor("volumeSharePct", { header: t("bestexec.share"), cell: (c) => `${c.getValue().toFixed(1)}%` }),
    bxc.accessor("avgClientSlipPts", { header: t("reports.avgSlip"), cell: (c) => c.getValue().toFixed(2) }),
    bxc.accessor("p95ClientSlipPts", { header: "p95 slip", cell: (c) => c.getValue().toFixed(2) }),
    bxc.accessor("priceImprovementPct", { header: t("bestexec.improved"), cell: (c) => `${c.getValue().toFixed(0)}%` }),
    bxc.accessor("p50LatencyMs", { header: t("reports.p50Latency"), cell: (c) => `${f.num(c.getValue())} ms` }),
    bxc.accessor("p95LatencyMs", { header: t("reports.p95Latency"), cell: (c) => `${f.num(c.getValue())} ms` }),
  ];
  const ibCols = [
    ibc.accessor("ib", { header: "IB" }),
    ibc.accessor("name", { header: t("clients.name"), cell: (c) => c.getValue() ?? "—" }),
    ibc.accessor("sharePct", { header: t("ib.share"), cell: (c) => `${c.getValue()}%` }),
    ibc.accessor("clients", { header: t("ib.clients") }),
    ibc.accessor("deals", { header: t("ib.deals") }),
    ibc.accessor("lots", { header: t("positions.lots"), cell: (c) => c.getValue().toFixed(2) }),
    ibc.accessor("commission", { header: t("reports.commission"), cell: (c) => f.money(c.getValue(), c.row.original.currency ?? "USD") }),
    ibc.accessor("markup", { header: t("reports.markup"), cell: (c) => f.money(c.getValue(), c.row.original.currency ?? "USD") }),
    ibc.accessor("payout", { header: t("ib.payout"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue(), c.row.original.currency ?? "USD")}</Pnl> }),
  ];
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
    if (tab === "ib") {
      const rows = (ib.data?.rows ?? []).map((x) => [x.ib, x.name ?? "", x.sharePct, x.clients, x.deals, x.lots.toFixed(2), formatMinorPlain(x.commission, x.currency ?? "USD"), formatMinorPlain(x.markup, x.currency ?? "USD"), formatMinorPlain(x.payout, x.currency ?? "USD")]);
      downloadCsv("ib-report.csv", toCsv(["ib", "name", "share_pct", "clients", "deals", "lots", "commission", "markup", "payout"], rows));
    } else if (tab === "transactions") {
      const rows = (tx.data?.rows ?? []).map((x) => [x.txId, x.tradingDateTime, x.executingEntity, x.buyerId, x.sellerId, x.clientLogin, x.instrument, x.assetClass ?? "", x.isin, x.side, x.entry, x.price, x.priceCurrency ?? "", x.quantityLots, x.quantityUnits, x.notional.toFixed(2), x.tradingCapacity, x.venue, x.executionLp ?? "", x.book, formatMinorPlain(x.commission, "USD"), formatMinorPlain(x.swap, "USD"), formatMinorPlain(x.realisedPnl, "USD"), x.reason]);
      downloadCsv("transactions.csv", toCsv(["tx_id", "trading_date_time", "executing_entity_lei", "buyer_id", "seller_id", "client_login", "instrument", "asset_class", "isin", "side", "entry", "price", "price_currency", "quantity_lots", "quantity_units", "notional", "trading_capacity", "venue", "execution_lp", "book", "commission", "swap", "realised_pnl", "reason"], rows));
    } else if (tab === "bestexec") {
      const rows = (bx.data?.rows ?? []).map((x) => [x.venue, x.assetClass, x.orders, x.filled, x.rejected, (x.fillRate * 100).toFixed(2), x.lots.toFixed(2), x.volumeSharePct.toFixed(2), x.avgClientSlipPts.toFixed(3), x.p95ClientSlipPts.toFixed(3), x.priceImprovementPct.toFixed(1), x.p50LatencyMs.toFixed(1), x.p95LatencyMs.toFixed(1)]);
      downloadCsv("best-execution.csv", toCsv(["venue", "asset_class", "orders", "filled", "rejected", "fill_rate_pct", "lots", "volume_share_pct", "avg_client_slip_pts", "p95_client_slip_pts", "price_improvement_pct", "p50_latency_ms", "p95_latency_ms"], rows));
    } else if (tab === "lp") {
      const rows = (lp.data ?? []).map((x) => [x.id, x.createdAt, x.symbol, x.side, x.lots, x.filledLots, x.avgPrice, x.status, x.fills.map((y) => y.execId).join(" "), x.clients.map((y) => `${y.login}@${y.price}`).join(" ")]);
      downloadCsv("lp-executions.csv", toCsv(["id", "created_at", "symbol", "side", "lots", "filled", "lp_price", "status", "exec_ids", "clients"], rows));
    } else if (tab === "execution") {
      const rows = (execution.data?.rows ?? []).map((x) => [x.id, x.at, x.login, x.name ?? "", x.symbol, x.side, x.type, x.lots, x.filledLots, x.book, x.requested ?? "", x.fill ?? "", x.clientSlipPts ?? "", x.lpPrice ?? "", x.lpSentPrice ?? "", x.lpSlipPts ?? "", x.attempts, x.capturePts ?? "", x.lpLatencyMs ?? "", x.lpFills, x.rearmed ? 1 : 0, x.status, x.reason ?? ""]);
      downloadCsv("execution.csv", toCsv(["id", "at", "login", "name", "symbol", "side", "type", "lots", "filled", "book", "requested", "fill", "client_slip_pts", "lp_price", "lp_sent_price", "lp_slip_pts", "attempts", "capture_pts", "lp_latency_ms", "lp_fills", "rearmed", "status", "reason"], rows));
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
        <Tabs value={tab} onChange={setTab} items={[{ value: "trades", label: t("reports.trades") }, { value: "statements", label: t("reports.statements") }, { value: "lp", label: t("reports.lp") }, { value: "execution", label: t("reports.execution") }, { value: "revenue", label: t("reports.revenue") }, { value: "flow", label: t("reports.flow") }, { value: "transactions", label: t("reports.transactions") }, { value: "bestexec", label: t("reports.bestExec") }, { value: "ib", label: t("reports.ib") }]} />
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
      {tab === "ib" && (
        <div data-testid="ib-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.ibHint")}</p>
          <DataTable data={ib.data?.rows ?? []} columns={ibCols} getRowId={(x) => String(x.ib)} />
        </div>
      )}
      {(tab === "transactions" || tab === "bestexec" || tab === "ib") && (
        <div className="mb-3 flex flex-wrap items-end gap-2 text-sm">
          <label className="grid gap-1 text-xs text-muted-foreground">{t("reports.range")}
            <span className="flex gap-1"><input type="date" className="rounded-md border border-border bg-background px-2 py-1 text-sm text-foreground" value={range.from} onChange={(e) => setRange({ ...range, from: e.target.value })} /><input type="date" className="rounded-md border border-border bg-background px-2 py-1 text-sm text-foreground" value={range.to} onChange={(e) => setRange({ ...range, to: e.target.value })} /></span>
          </label>
        </div>
      )}
      {tab === "transactions" && (
        <div data-testid="transactions-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.transactionsHint")}</p>
          <DataTable data={tx.data?.rows ?? []} columns={txCols} getRowId={(x) => x.txId} />
        </div>
      )}
      {tab === "bestexec" && (
        <div data-testid="bestexec-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.bestExecHint")}</p>
          <DataTable data={bx.data?.rows ?? []} columns={bxCols} getRowId={(x) => `${x.venue}-${x.assetClass}`} />
        </div>
      )}
      {tab === "flow" && (
        <div data-testid="flow-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.flowHint")}</p>
          <DataTable data={flow.data ?? []} columns={flowCols} getRowId={(x) => String(x.login)} />
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
