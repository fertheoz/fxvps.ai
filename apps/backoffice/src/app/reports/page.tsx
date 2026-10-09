"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { Download } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Dialog, PageHeader, Pnl, Stat, Tabs } from "@/components/ui/primitives";
import { BookBadge, SideBadge, ToxicityBadge } from "@/components/badges";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useToast } from "@/components/shell/providers";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Trade } from "@/lib/schemas";
import type { BestExecutionRow, BookSnapshot, ClientFlowRow, ClientOrderDetail, IbRow, ExecutionRow, ExecutionSummary, LpExecution, LpOrderDetail, MarkoutRow, ReconciliationRow, RevenueRow, RevenueTotals, Statement, TransactionRow } from "@/lib/api";
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
const mc = createColumnHelper<ReconciliationRow>();
type Tab = "trades" | "statements" | "lp" | "execution" | "revenue" | "flow" | "transactions" | "bestexec" | "ib" | "reconciliation" | "analytics";

const pts = (v: number | null | undefined, digits = 1) => (v === null || v === undefined ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(digits)}`);
const pct = (v: number) => `${(v * 100).toFixed(0)}%`;
/** Slippage colour: red when the client lost ground, green when the price improved. */
const slipTone = (v: number | null | undefined) => (v === null || v === undefined || v === 0 ? undefined : v > 0 ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400");

export default function ReportsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const [tab, setTab] = React.useState<Tab>("trades");
  const [range, setRange] = React.useState<{ from: string; to: string }>({ from: "", to: "" });
  const ranged = !!(range.from || range.to);
  const rangeArgs: [string | undefined, string | undefined] = [range.from || undefined, range.to || undefined];
  // Live refresh only on the default (newest rows) view; a chosen range is a one-off load.
  const trades = useApiQuery("listTrades", rangeArgs);
  const statements = useApiQuery("statements");
  const lp = useApiQuery("listLpExecutions", rangeArgs, { live: ranged ? undefined : 5000 });
  const revenue = useApiQuery("revenue", [], { live: 5000 });
  const recon = useApiQuery("reconciliation", rangeArgs, { live: ranged ? undefined : 5000, enabled: tab === "reconciliation" });
  const execution = useApiQuery("execution", [], { live: 5000 });
  const flow = useApiQuery("clientFlow", [], { live: 10000, enabled: tab === "flow" });
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
    lc.accessor("clOrdId", { header: t("reports.clOrdId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("kind", { header: t("reports.kind"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    lc.accessor("attempt", { header: t("reports.attempts"), cell: (c) => (c.getValue() === undefined ? "—" : `${c.getValue()} / ${c.row.original.attempts ?? 1}`), meta: { defaultHidden: true } }),
    lc.accessor("firstFillMs", { header: t("reports.lpLatency"), cell: (c) => ms(c.getValue()), meta: { defaultHidden: true } }),
    lc.accessor("lastFillMs", { header: t("reports.lastFill"), cell: (c) => ms(c.getValue()), meta: { defaultHidden: true } }),
    lc.accessor("lpSlipPts", { header: t("reports.lpSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue() ?? null) ?? ""}`}>{pts(c.getValue())}</span>, meta: { defaultHidden: true } }),
    lc.accessor("sentBid", { header: t("reports.sentBid"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("sentAsk", { header: t("reports.sentAsk"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("limit", { header: t("detail.limit"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("stop", { header: t("detail.stop"), cell: (c) => <span className="tabular-nums">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("revision", { header: t("reports.revision"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    lc.accessor("fillCount", { header: t("reports.lpFills"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    lc.accessor("login", { header: t("clients.login"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    lc.accessor("orderIds", { header: t("reports.orderId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span>, meta: { defaultHidden: true } }),
    lc.accessor("reason", { header: t("detail.reason"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
  ];
  const leg = (k: "client" | "broker" | "lp", label: "reports.clientLeg" | "reports.brokerLeg" | "reports.lpLeg") =>
    rc.accessor(k, { header: t(label), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> });
  const gain = (k: "markup" | "commission" | "swapFee" | "broker") =>
    mc.accessor(k, { header: t(`reports.recon.${k}`), cell: (c) => <span className={`tabular-nums ${c.getValue() > 0 ? "text-emerald-600 dark:text-emerald-400" : c.getValue() < 0 ? "text-red-600 dark:text-red-400" : "text-muted-foreground"}`}>{f.money(c.getValue())}</span> });
  const px = (v: number | null) => <span className="tabular-nums">{v ?? "—"}</span>;
  const reconCols = [
    mc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    mc.accessor("login", { header: t("clients.login") }),
    mc.accessor("symbol", { header: t("positions.symbol") }),
    mc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    mc.accessor("lots", { header: t("positions.lots") }),
    mc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    mc.accessor("openClient", { header: t("reports.recon.openClient"), cell: (c) => px(c.getValue()) }),
    mc.accessor("openLp", { header: t("reports.recon.openLp"), cell: (c) => px(c.getValue()) }),
    mc.accessor("closeClient", { header: t("reports.recon.closeClient"), cell: (c) => px(c.getValue()) }),
    mc.accessor("closeLp", { header: t("reports.recon.closeLp"), cell: (c) => px(c.getValue()) }),
    mc.accessor("clientPnl", { header: t("reports.recon.clientPnl"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
    mc.accessor("lpPnl", { header: t("reports.recon.lpPnl"), cell: (c) => (c.row.original.book === "A" ? <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> : "—") }),
    gain("markup"), gain("commission"), gain("swapFee"), gain("broker"),
    mc.accessor("ok", { header: t("reports.recon.ok"), cell: (c) => <Badge tone={c.getValue() ? "success" : "danger"}>{c.getValue() ? "✓" : "✗"}</Badge> }),
    mc.accessor("reason", { header: t("reports.recon.reason"), cell: (c) => <span className="text-xs text-muted-foreground">{c.getValue()}</span> }),
    mc.accessor("openAt", { header: t("reports.openAt"), cell: (c) => (c.getValue() ? f.date(c.getValue()!) : "—"), meta: { defaultHidden: true } }),
    mc.accessor("holdSecs", { header: t("reports.hold"), cell: (c) => hold(c.getValue()), meta: { defaultHidden: true } }),
    mc.accessor("position", { header: t("reports.positionId"), cell: (c) => <span className="font-mono text-xs">{c.getValue()}</span>, meta: { defaultHidden: true } }),
    mc.accessor("orderId", { header: t("reports.orderId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    mc.accessor("platform", { header: t("reports.platform"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    mc.accessor("origin", { header: t("reports.origin"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    mc.accessor("rule", { header: t("rules.title"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    mc.accessor("clientSlipPts", { header: t("reports.clientSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue() ?? null) ?? ""}`}>{pts(c.getValue())}</span>, meta: { defaultHidden: true } }),
    mc.accessor("lpSlipPts", { header: t("reports.lpSlip"), cell: (c) => <span className={`tabular-nums ${slipTone(c.getValue() ?? null) ?? ""}`}>{pts(c.getValue())}</span>, meta: { defaultHidden: true } }),
    mc.accessor("attempts", { header: t("reports.attempts"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    mc.accessor("latencyMs", { header: t("reports.lpLatency"), cell: (c) => ms(c.getValue()), meta: { defaultHidden: true } }),
    mc.accessor("lpKind", { header: t("detail.kind"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    mc.accessor("lpOrderIds", { header: t("reports.clOrdId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span>, meta: { defaultHidden: true } }),
    mc.accessor("swap", { header: t("reports.swap"), cell: (c) => (c.getValue() === undefined ? "—" : f.money(c.getValue()!)), meta: { defaultHidden: true } }),
  ];
  const reconTotals = React.useMemo(() => {
    const rows = recon.data ?? [];
    const sum = (k: "clientPnl" | "lpPnl" | "markup" | "commission" | "swapFee" | "broker") => rows.reduce((a, r) => a + r[k], 0);
    return { n: rows.length, bad: rows.filter((r) => !r.ok).length, clientPnl: sum("clientPnl"), lpPnl: sum("lpPnl"), markup: sum("markup"), commission: sum("commission"), swapFee: sum("swapFee"), broker: sum("broker") };
  }, [recon.data]);
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
    fc.accessor("markout5s", { header: t("flow.markout5"), cell: (c) => { const v = c.getValue(); return v == null ? "—" : <span className={v > 0 ? "text-red-600 dark:text-red-400" : ""}>{v.toFixed(1)}</span>; } }),
    fc.accessor("markout60s", { header: t("flow.markout60"), cell: (c) => { const v = c.getValue(); return v == null ? "—" : v.toFixed(1); } }),
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
    ibc.accessor("rebate", { header: t("ib.rebate"), cell: (c) => f.money(c.getValue() ?? 0, c.row.original.currency ?? "USD") }),
    ibc.accessor("override", { header: t("ib.override"), cell: (c) => f.money(c.getValue() ?? 0, c.row.original.currency ?? "USD") }),
    ibc.accessor("payout", { header: t("ib.payout"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue(), c.row.original.currency ?? "USD")}</Pnl> }),
    ibc.accessor("paidThrough", { header: t("ib.paidThrough"), cell: (c) => c.getValue() ?? "—" }),
    ibc.display({ id: "pay", header: "", cell: (c) => <IbPayButton row={c.row.original} to={range.to} /> }),
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
    tc.accessor("openAt", { header: t("reports.openAt"), cell: (c) => (c.getValue() ? f.date(c.getValue()!) : "—"), meta: { defaultHidden: true } }),
    tc.accessor("holdSecs", { header: t("reports.hold"), cell: (c) => hold(c.getValue()), meta: { defaultHidden: true } }),
    tc.accessor("reason", { header: t("reports.recon.reason"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    tc.accessor("origin", { header: t("reports.origin"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    tc.accessor("platform", { header: t("reports.platform"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    tc.accessor("rule", { header: t("rules.title"), cell: (c) => c.getValue() ?? "—", meta: { defaultHidden: true } }),
    tc.accessor("commissionOpen", { header: t("reports.commissionOpen"), cell: (c) => (c.getValue() === undefined ? "—" : f.money(c.getValue()!)), meta: { defaultHidden: true } }),
    tc.accessor("positionId", { header: t("reports.positionId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
    tc.accessor("orderId", { header: t("reports.orderId"), cell: (c) => <span className="font-mono text-xs">{c.getValue() ?? "—"}</span>, meta: { defaultHidden: true } }),
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
        <Tabs value={tab} onChange={setTab} items={[{ value: "trades", label: t("reports.trades") }, { value: "statements", label: t("reports.statements") }, { value: "lp", label: t("reports.lp") }, { value: "execution", label: t("reports.execution") }, { value: "revenue", label: t("reports.revenue") }, { value: "flow", label: t("reports.flow") }, { value: "transactions", label: t("reports.transactions") }, { value: "bestexec", label: t("reports.bestExec") }, { value: "ib", label: t("reports.ib") }, { value: "reconciliation", label: t("reports.recon") }, { value: "analytics", label: t("reports.analytics") }]} />
      </div>
      {tab === "statements" && <DataTable data={statements.data ?? []} columns={stmtCols} getRowId={(x) => String(x.login)} />}
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
      {(tab === "transactions" || tab === "bestexec" || tab === "ib" || tab === "trades" || tab === "lp" || tab === "reconciliation") && (
        <div className="mb-3 flex flex-wrap items-end gap-2 text-sm">
          {(tab === "trades" || tab === "lp" || tab === "reconciliation") && (
            <span className="text-xs text-muted-foreground">
              {t("reports.rangeHint")}
              {(tab === "trades" ? trades : tab === "lp" ? lp : recon).isFetching && <span className="ml-2 text-primary">{t("common.loadingRange")}</span>}
            </span>
          )}
          <label className="grid gap-1 text-xs text-muted-foreground">{t("reports.range")}
            <span className="flex gap-1"><input type="date" className="rounded-md border border-border bg-background px-2 py-1 text-sm text-foreground" value={range.from} onChange={(e) => setRange({ ...range, from: e.target.value })} /><input type="date" className="rounded-md border border-border bg-background px-2 py-1 text-sm text-foreground" value={range.to} onChange={(e) => setRange({ ...range, to: e.target.value })} /></span>
          </label>
        </div>
      )}
      {tab === "trades" && <DataTable data={trades.data ?? []} columns={tradeCols} getRowId={(x) => x.id} storageKey="reports-trades" />}
      {tab === "lp" && <DataTable data={lp.data ?? []} columns={lpCols} getRowId={(x) => x.id} renderDetail={(x) => <LpExecutionDetail row={x} />} storageKey="reports-lp" />}
      {tab === "reconciliation" && (
        <div data-testid="reconciliation-report">
          <p className="mb-2 text-xs text-muted-foreground">{t("reports.recon.hint")}</p>
          <div className="mb-3 grid grid-cols-2 gap-3 xl:grid-cols-6">
            <Stat label={t("reports.recon.clientPnl")} value={<Pnl value={reconTotals.clientPnl}>{f.money(reconTotals.clientPnl)}</Pnl>} sub={`${reconTotals.n} ${t("reports.recon.deals")}`} />
            <Stat label={t("reports.recon.lpPnl")} value={<Pnl value={reconTotals.lpPnl}>{f.money(reconTotals.lpPnl)}</Pnl>} sub="A-book" />
            <Stat label={t("reports.recon.markup")} value={<Pnl value={reconTotals.markup}>{f.money(reconTotals.markup)}</Pnl>} sub={t("reports.brokerLeg")} />
            <Stat label={t("reports.recon.commission")} value={<Pnl value={reconTotals.commission}>{f.money(reconTotals.commission)}</Pnl>} sub={t("reports.brokerLeg")} />
            <Stat label={t("reports.recon.broker")} value={<Pnl value={reconTotals.broker}>{f.money(reconTotals.broker)}</Pnl>} sub={t("reports.recon.expected")} />
            <Stat label={t("reports.recon.ok")} value={<Badge tone={reconTotals.bad === 0 ? "success" : "danger"}>{reconTotals.bad === 0 ? t("reports.recon.allOk") : `${reconTotals.bad} ✗`}</Badge>} sub={t("reports.recon.invariant")} />
          </div>
          <DataTable data={recon.data ?? []} columns={reconCols} getRowId={(x) => x.id} renderDetail={(x) => <ReconciliationDetail row={x} />} storageKey="reports-reconciliation" />
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
      {tab === "analytics" && <AnalyticsTab />}
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

/** Parça 13: liquidity-by-hour map, deal markout from the tick warehouse, what-if markup replay. */
function AnalyticsTab() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const [symbol, setSymbol] = React.useState("");
  const [day, setDay] = React.useState("");
  const liq = useApiQuery("liquidity", [{ symbol: symbol || undefined, day: day || undefined }], { live: 30000 });
  const [hours, setHours] = React.useState(24);
  const mo = useApiQuery("markout", [{ hours, limit: 500 }], { live: 30000 });
  const intl = useApiQuery("internalization", [{ hours }], { live: 30000 });
  const [delta, setDelta] = React.useState(2);
  const [group, setGroup] = React.useState("");
  const [wi, setWi] = React.useState<import("@/lib/api").WhatIfReport | null>(null);
  const run = useApiMutation((v: { hours: number; deltaPoints: number; group: string | null }) => api().whatIf(v, actor), setWi);
  const mc = createColumnHelper<MarkoutRow>();
  const moCols = React.useMemo(() => [
    mc.accessor("at", { header: t("reports.at"), cell: (c) => <span className="whitespace-nowrap text-xs">{f.date(c.getValue())}</span> }),
    mc.accessor("login", { header: t("clients.login"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    mc.accessor("symbol", { header: t("positions.symbol") }),
    mc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    mc.accessor("entry", { header: t("reports.entry"), cell: (c) => <Badge tone="muted">{c.getValue()}</Badge> }),
    mc.accessor("lots", { header: t("positions.lots"), cell: (c) => <span className="tabular-nums">{f.num(c.getValue())}</span> }),
    mc.accessor("price", { header: t("positions.price"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ...(["m1", "m5", "m30"] as const).map((k) => mc.accessor(k, { header: t(`reports.${k}`), cell: (c) => { const v = c.getValue(); return v == null ? <span className="text-muted-foreground">—</span> : <span className={`tabular-nums ${v > 0 ? "text-emerald-600 dark:text-emerald-400" : v < 0 ? "text-red-600 dark:text-red-400" : ""}`}>{v > 0 ? "+" : ""}{f.num(v, 1)}</span>; } })),
  ], [t, f, mc]);
  const shade = (v: number, max: number) => `rgba(245, 158, 11, ${Math.min(1, max > 0 ? v / max : 0) * 0.6})`;
  const maxSpread = Math.max(1, ...(liq.data?.hours ?? []).map((h) => h.avgSpreadPoints || 0));
  return (
    <div className="grid gap-4" data-testid="analytics">
      <p className="text-xs text-muted-foreground">{t("reports.analyticsHint")}</p>
      <div className="rounded-md border border-border p-3">
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <span className="font-medium">{t("reports.liquidityMap")}</span>
          <select className="rounded-md border border-border bg-background p-1 text-sm" value={symbol || liq.data?.symbol || ""} onChange={(e) => setSymbol(e.target.value)} aria-label={t("positions.symbol")} data-testid="liq-symbol">
            {(liq.data?.symbols ?? []).map((s) => <option key={s} value={s}>{s}</option>)}
          </select>
          <select className="rounded-md border border-border bg-background p-1 text-sm" value={day || liq.data?.day || ""} onChange={(e) => setDay(e.target.value)} aria-label={t("reports.day")} data-testid="liq-day">
            {(liq.data?.days ?? []).map((d) => <option key={d} value={d}>{d}</option>)}
          </select>
          <span className="text-xs text-muted-foreground">{liq.data ? `${liq.data.ticks} ${t("reports.ticks")}` : t("common.loading")}</span>
        </div>
        {liq.data && liq.data.ticks === 0 && <div className="text-sm text-muted-foreground">{t("reports.noTicks")}</div>}
        {liq.data && liq.data.ticks > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-xs tabular-nums">
              <thead><tr>{[t("reports.hourUtc"), t("reports.ticks"), t("reports.spreadAvg"), t("reports.spreadMin"), t("reports.spreadMax"), t("reports.topBid"), t("reports.topAsk")].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
              <tbody>
                {liq.data.hours.map((h) => (
                  <tr key={h.hour} className="border-t border-border/60">
                    <td className="px-2 py-0.5">{String(h.hour).padStart(2, "0")}:00</td>
                    <td className="px-2 py-0.5">{h.ticks}</td>
                    <td className="px-2 py-0.5" style={{ background: h.ticks ? shade(h.avgSpreadPoints, maxSpread) : undefined }}>{h.ticks ? f.num(h.avgSpreadPoints, 1) : "—"}</td>
                    <td className="px-2 py-0.5">{h.ticks ? f.num(h.minSpreadPoints, 1) : "—"}</td>
                    <td className="px-2 py-0.5">{h.ticks ? f.num(h.maxSpreadPoints, 1) : "—"}</td>
                    <td className="px-2 py-0.5">{h.ticks ? f.num(h.avgBidLots, 1) : "—"}</td>
                    <td className="px-2 py-0.5">{h.ticks ? f.num(h.avgAskLots, 1) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <div className="rounded-md border border-border p-3">
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <span className="font-medium">{t("reports.markout")}</span>
          <select className="rounded-md border border-border bg-background p-1 text-sm" value={String(hours)} onChange={(e) => setHours(Number(e.target.value))} aria-label={t("platform.window")}>
            {[6, 24, 72, 168].map((h) => <option key={h} value={h}>{t("platform.lastHours", { n: h })}</option>)}
          </select>
          {(mo.data?.summary ?? []).map((s) => <Badge key={s.symbol} tone={s.m5 > 0 ? "danger" : "success"}>{s.symbol} · {s.deals} · 5s {s.m5 > 0 ? "+" : ""}{f.num(s.m5, 1)} pt</Badge>)}
        </div>
        <p className="mb-2 text-xs text-muted-foreground">{t("reports.markoutHint")}</p>
        <DataTable data={mo.data?.rows ?? []} columns={moCols} getRowId={(x) => x.id} storageKey="analytics-markout" />
      </div>
      <div className="rounded-md border border-border p-3" data-testid="internalization">
        <div className="mb-2 flex flex-wrap items-center gap-2">
          <span className="font-medium">{t("reports.internalization")}</span>
          {intl.data && <Badge tone={intl.data.internalPct >= 50 ? "success" : "muted"}>{f.num(intl.data.internalPct, 0)}% · {f.num(intl.data.internalLots)} / {f.num(intl.data.clientLots)} lot</Badge>}
        </div>
        <p className="mb-2 text-xs text-muted-foreground">{t("reports.internalizationHint")}</p>
        {intl.data && intl.data.rows.length > 0 && (
          <table className="w-full text-xs tabular-nums">
            <thead><tr>{[t("positions.symbol"), t("reports.deals"), t("reports.clientLots"), t("reports.lpLots"), t("reports.internalLots"), "%", t("reports.captured")].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
            <tbody>{intl.data.rows.map((r) => <tr key={r.symbol} className="border-t border-border/60"><td className="px-2 py-0.5">{r.symbol}</td><td className="px-2 py-0.5">{r.deals}</td><td className="px-2 py-0.5">{f.num(r.clientLots)}</td><td className="px-2 py-0.5">{f.num(r.lpLots)}</td><td className="px-2 py-0.5">{f.num(r.internalLots)}</td><td className="px-2 py-0.5">{f.num(r.internalPct, 0)}</td><td className="px-2 py-0.5"><Pnl value={r.captured}>{f.money(r.captured)}</Pnl></td></tr>)}</tbody>
          </table>
        )}
        {intl.data && intl.data.rows.length === 0 && <div className="text-xs text-muted-foreground">{t("common.noResults")}</div>}
      </div>
      <div className="rounded-md border border-border p-3" data-testid="whatif">
        <div className="mb-2 font-medium">{t("reports.whatIf")}</div>
        <p className="mb-2 text-xs text-muted-foreground">{t("reports.whatIfHint")}</p>
        <div className="flex flex-wrap items-end gap-2">
          <label className="grid gap-1 text-xs">{t("reports.deltaPoints")}<input type="number" className="w-24 rounded-md border border-border bg-background p-1 text-sm" value={delta} onChange={(e) => setDelta(Number(e.target.value))} data-testid="whatif-delta" /></label>
          <label className="grid gap-1 text-xs">{t("clients.group")}<input className="w-32 rounded-md border border-border bg-background p-1 text-sm" value={group} onChange={(e) => setGroup(e.target.value)} placeholder="*" /></label>
          <Button onClick={() => run.mutate({ hours, deltaPoints: Math.round(delta), group: group.trim() || null })} disabled={run.isPending || !Number.isFinite(delta) || Math.round(delta) === 0} data-testid="whatif-run">{t("reports.whatIfRun")}</Button>
          {wi && <span className="text-sm">{t("reports.whatIfResult", { n: wi.deltaPoints })} <strong className={wi.totalUsd >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400"}>{wi.totalUsd >= 0 ? "+" : ""}{f.num(wi.totalUsd, 0)} USD</strong></span>}
        </div>
        {wi && wi.rows.length > 0 && (
          <table className="mt-2 w-full text-xs tabular-nums">
            <thead><tr>{[t("positions.symbol"), t("reports.legs"), t("positions.lots"), t("reports.delta"), "USD"].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
            <tbody>{wi.rows.map((r) => <tr key={r.symbol} className="border-t border-border/60"><td className="px-2 py-0.5">{r.symbol}</td><td className="px-2 py-0.5">{r.legs}</td><td className="px-2 py-0.5">{f.num(r.lots)}</td><td className="px-2 py-0.5">{f.num(r.delta)} {r.currency}</td><td className="px-2 py-0.5">{r.deltaUsd == null ? "—" : f.num(r.deltaUsd, 0)}</td></tr>)}</tbody>
          </table>
        )}
      </div>
    </div>
  );
}

/** Today (UTC) as YYYY-MM-DD: only days before it can be paid out. */
const todayUtc = () => new Date().toISOString().slice(0, 10);

/** IB payout up to the range's "to" day. The server works out the amount from
 * the IB's paid-through date (never the table's figure), books it through the
 * four-eyes flow and moves paid-through only once the deposit is applied. */
function IbPayButton({ row, to }: { row: IbRow; to: string }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const [open, setOpen] = React.useState(false);
  const preview = useApiQuery("ibPayoutPreview", [row.ib, to], { enabled: open });
  const pay = useApiMutation(
    () => api().ibPayout(row.ib, to, actor),
    (r) => {
      setOpen(false);
      toast(r.op.status === "applied" ? t("ib.payApplied") : t("ib.payQueued"));
    },
  );
  if (!actor.can("balance.deposit")) return null;
  if (row.payoutPending) return <Badge tone="muted" title={row.payoutPending} data-testid={`ib-pay-pending-${row.ib}`}>{t("ib.payAwaiting")}</Badge>;
  const closed = !!to && to < todayUtc();
  const p = preview.data;
  return (
    <>
      <Button size="sm" variant="outline" disabled={!mfa || !closed} title={closed ? undefined : t("ib.payPeriod")} onClick={() => setOpen(true)} data-testid={`ib-pay-${row.ib}`}>
        {t("ib.pay")}
      </Button>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title={`${t("ib.payTitle")} #${row.ib}`}
        footer={<>
          <Button variant="outline" onClick={() => setOpen(false)}>{t("common.cancel")}</Button>
          <Button onClick={() => pay.mutate(undefined)} disabled={!p || p.amount <= 0 || !!p.pending || pay.isPending} data-testid={`ib-pay-confirm-${row.ib}`}>{t("common.confirm")}</Button>
        </>}
      >
        {preview.error ? (
          <p className="text-sm text-red-600 dark:text-red-400">{preview.error.message}</p>
        ) : !p ? (
          <p className="text-sm text-muted-foreground">…</p>
        ) : (
          <div className="grid gap-2 text-sm" data-testid={`ib-pay-preview-${row.ib}`}>
            <p>{p.from ?? t("ib.payFromStart")} … {p.to}</p>
            <p><strong>{f.money(p.amount, p.currency)}</strong></p>
            {p.pending && <p className="text-xs text-amber-600 dark:text-amber-400">{t("ib.payAwaiting")} ({p.pending})</p>}
            {!p.pending && p.amount <= 0 && <p className="text-xs text-muted-foreground">{t("ib.payNothing")}</p>}
            <p className="text-xs text-muted-foreground">{t("ib.payHint")}</p>
          </div>
        )}
      </Dialog>
    </>
  );
}


// ---------------------------------------------------------------------------
// Row detail panels: everything under the tab's heading, for one row.

const ms = (v: number | null | undefined) => (v === null || v === undefined ? "—" : `${v < 10 ? v.toFixed(1) : Math.round(v)} ms`);
/** Hold time in seconds → "4d 3h", "2h 05m", "48s". */
const hold = (s: number | null | undefined) => {
  if (s === null || s === undefined) return "—";
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
  return `${Math.floor(s / 86400)}d ${Math.floor((s % 86400) / 3600)}h`;
};
const ptsStr = (v: number | null | undefined) => (v === null || v === undefined ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)}`);
const Kv = ({ k, v, tone }: { k: string; v: React.ReactNode; tone?: "good" | "bad" }) => (
  <div className="flex items-baseline justify-between gap-3 border-b border-border/60 py-0.5 text-xs">
    <span className="text-muted-foreground">{k}</span>
    <span className={`tabular-nums text-right ${tone === "good" ? "text-emerald-600 dark:text-emerald-400" : tone === "bad" ? "text-red-600 dark:text-red-400" : ""}`}>{v}</span>
  </div>
);
const slipTone2 = (v: number | null | undefined): "good" | "bad" | undefined => (v === null || v === undefined || v === 0 ? undefined : v < 0 ? "good" : "bad");

const FIX_TYPES: Record<string, string> = { D: "NewOrderSingle", "8": "ExecutionReport", F: "OrderCancelRequest", G: "OrderCancelReplaceRequest", "9": "OrderCancelReject", "3": "Reject", j: "BusinessMessageReject", AN: "RequestForPositions", AP: "PositionReport", AD: "TradeCaptureReportRequest", AE: "TradeCaptureReport", AQ: "TradeCaptureReportRequestAck" };

/** The raw FIX frames behind one LP order, with a copy button. Read-only:
 * the treasurer pastes them into their own correspondence with the LP. */
function FixMessages({ clOrdId }: { clOrdId: string }) {
  const t = useT();
  const f = useFormat();
  const toast = useToast();
  const [open, setOpen] = React.useState(false);
  const q = useApiQuery("fixMessages", [clOrdId], { enabled: open });
  const msgs = q.data?.messages ?? [];
  const copy = async () => {
    const text = msgs.map((m) => `${m.at} ${m.dir === "out" ? "→ LP" : "← LP"} ${m.msgType} ${FIX_TYPES[m.msgType] ?? ""}\n${m.raw}`).join("\n\n");
    try {
      await navigator.clipboard.writeText(text);
      toast(t("detail.fixCopied"));
    } catch {
      toast(t("detail.fixCopyFailed"));
    }
  };
  return (
    <div className="sm:col-span-2">
      <div className="flex items-center gap-2">
        <button className="text-xs text-primary hover:underline" onClick={() => setOpen((o) => !o)} data-testid="fix-messages-toggle">{open ? "▾" : "▸"} {t("detail.fixMessages")} · {clOrdId}</button>
        {open && msgs.length > 0 && <Button size="sm" variant="outline" onClick={() => void copy()} data-testid="fix-messages-copy">{t("detail.fixCopy")}</Button>}
      </div>
      {open && (
        <div className="mt-1 grid gap-1">
          {q.isLoading && <div className="text-xs text-muted-foreground">{t("common.loading")}</div>}
          {!q.isLoading && msgs.length === 0 && <div className="text-xs text-muted-foreground">{t("detail.fixNone")}</div>}
          {msgs.map((m, i) => (
            <div key={i} className="rounded border border-border/70 bg-background p-1.5">
              <div className="mb-0.5 flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                <span>{f.date(m.at)}</span>
                <span className={m.dir === "out" ? "text-primary" : "text-emerald-600 dark:text-emerald-400"}>{m.dir === "out" ? "→ LP" : "← LP"}</span>
                <span>35={m.msgType} {FIX_TYPES[m.msgType] ?? ""}</span>
                {m.clOrdId && <span>11={m.clOrdId}</span>}{m.origClOrdId && <span>41={m.origClOrdId}</span>}{m.orderId && <span>37={m.orderId}</span>}{m.execId && <span>17={m.execId}</span>}
              </div>
              <pre className="whitespace-pre-wrap break-all font-mono text-[11px]">{m.raw}</pre>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** LP-side timeline of one LP order: sent → reports, each with its latency. */
function LpTimeline({ d, f }: { d: LpOrderDetail; f: ReturnType<typeof useFormat> }) {
  const t = useT();
  return (
    <div className="grid gap-2 sm:grid-cols-2">
      <div>
        <Kv k={t("detail.kind")} v={<>{d.kind}{d.resting ? ` · r${d.revision}` : ""}{d.hedge ? ` · ${t("detail.hedge")}` : ""}</>} />
        <Kv k={t("detail.sentAt")} v={f.date(d.sentAt)} />
        <Kv k={t("detail.sentQuote")} v={`${d.sentBid ?? "—"} / ${d.sentAsk ?? "—"}`} />
        {d.limit !== null && <Kv k={t("detail.limit")} v={d.limit} />}
        {d.stop !== null && <Kv k={t("detail.stop")} v={d.stop} />}
        <Kv k={t("detail.attempt")} v={`${d.attempt} / ${d.attempts}`} tone={d.attempts > 1 ? "bad" : undefined} />
        <Kv k={t("detail.firstFill")} v={ms(d.firstFillMs)} />
        <Kv k={t("detail.lastFill")} v={ms(d.lastFillMs)} />
        <Kv k={t("detail.lpSlip")} v={ptsStr(d.lpSlipPts)} tone={slipTone2(d.lpSlipPts)} />
        {d.reason && <Kv k={t("detail.reason")} v={d.reason} tone="bad" />}
      </div>
      <div>
        <div className="mb-1 text-xs font-medium text-muted-foreground">{t("detail.reports")} ({d.fills.length})</div>
        {d.fills.length === 0 && <div className="text-xs text-muted-foreground">{d.done ? t("detail.noFill") : t("detail.working")}</div>}
        {d.fills.map((x) => (
          <div key={x.execId} className="grid grid-cols-[auto_1fr_auto_auto] gap-2 border-b border-border/60 py-0.5 font-mono text-[11px]">
            <span>{f.date(x.at)}</span><span className="truncate">{x.execId}</span><span className="tabular-nums">{x.lots} @ {x.price}</span><span className="tabular-nums text-muted-foreground">{ms(x.latencyMs)}</span>
          </div>
        ))}
      </div>
      {d.book && <BookAtSend b={d.book} />}
      {d.clOrdId && <FixMessages clOrdId={d.clOrdId} />}
    </div>
  );
}

/** Parça 15: the book the router saw when it sent the LP order. */
function BookAtSend({ b }: { b: BookSnapshot }) {
  const t = useT();
  const f = useFormat();
  const rows = Math.max(b.merged.bids.length, b.merged.asks.length);
  const lpAt = (side: "bids" | "asks", price: number) => b.lps.filter((l) => l[side].some(([p]) => p === price)).map((l) => l.lp).join(" ");
  return (
    <div className="sm:col-span-2">
      <div className="mb-1 text-xs font-medium text-muted-foreground">{t("detail.book")} · {f.date(new Date(b.tsNs / 1e6).toISOString())} · {b.lps.map((l) => `${l.lp} ${l.ageMs} ms`).join(" · ")}</div>
      <div className="grid grid-cols-2 gap-3 font-mono text-[11px] tabular-nums">
        {(["bids", "asks"] as const).map((side) => (
          <div key={side}>
            <div className="mb-0.5 text-muted-foreground">{side === "bids" ? t("detail.bids") : t("detail.asks")}</div>
            {Array.from({ length: rows }, (_, i) => b.merged[side][i]).map((lv, i) => lv ? (
              <div key={i} className="grid grid-cols-[1fr_auto_auto] gap-2 border-b border-border/60 py-0.5">
                <span className={side === "bids" ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400"}>{lv[0]}</span><span>{f.num(lv[1])}</span><span className="text-muted-foreground">{lpAt(side, lv[0])}</span>
              </div>
            ) : <div key={i} className="py-0.5 text-muted-foreground">—</div>)}
          </div>
        ))}
      </div>
    </div>
  );
}

/** Client side of one order and the plain-language verdict on its execution. */
function ClientOrderPanel({ d, f }: { d: ClientOrderDetail; f: ReturnType<typeof useFormat> }) {
  const t = useT();
  const notes: { text: string; tone?: "good" | "bad" }[] = [];
  if (d.lpOrders && d.lpOrders.length > 1) notes.push({ text: t("detail.noteRetries", { n: d.lpOrders.length }), tone: "bad" });
  if (d.clientSlipPts !== null && d.clientSlipPts > 0) notes.push({ text: t("detail.noteClientSlip", { p: d.clientSlipPts.toFixed(1) }), tone: "bad" });
  if (d.clientSlipPts !== null && d.clientSlipPts < 0) notes.push({ text: t("detail.noteImprovement", { p: (-d.clientSlipPts).toFixed(1) }), tone: "good" });
  const worstLp = d.lpOrders?.reduce<number | null>((m, l) => (l.lpSlipPts !== null && (m === null || l.lpSlipPts > m) ? l.lpSlipPts : m), null) ?? null;
  if (worstLp !== null && worstLp > 0) notes.push({ text: t("detail.noteLpSlip", { p: worstLp.toFixed(1) }), tone: "bad" });
  const slowest = d.lpOrders?.reduce<number | null>((m, l) => (l.firstFillMs !== null && (m === null || l.firstFillMs > m) ? l.firstFillMs : m), null) ?? null;
  if (slowest !== null && slowest > 500) notes.push({ text: t("detail.noteSlow", { ms: Math.round(slowest) }), tone: "bad" });
  if (d.lpOrders?.some((l) => l.resting)) notes.push({ text: t("detail.noteResting"), tone: "good" });
  if (d.filledLots > 0 && d.filledLots < d.lots) notes.push({ text: t("detail.notePartial", { a: d.filledLots, b: d.lots }), tone: "bad" });
  if (d.reason) notes.push({ text: `${t("detail.reason")}: ${d.reason}`, tone: "bad" });
  if (notes.length === 0) notes.push({ text: t("detail.noteClean"), tone: "good" });
  return (
    <div className="rounded-md border border-border bg-card p-2">
      <div className="mb-1 flex flex-wrap items-center gap-2 text-xs">
        <span className="font-mono">#{d.orderId}</span><span className="text-muted-foreground">{d.clientOrderId}</span>
        <SideBadge side={d.side} /><BookBadge book={d.book} /><span>{d.kind}</span><span className="text-muted-foreground">{d.origin} · {d.platform}{d.ip ? ` · ${d.ip}` : ""}</span>
        {d.rule && <Badge tone="default">{t("rules.title")}: {d.rule}</Badge>}
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <div>
          <Kv k={t("detail.createdAt")} v={f.date(d.createdAt)} />
          <Kv k={t("positions.lots")} v={`${d.filledLots} / ${d.lots}`} />
          <Kv k={t("reports.requested")} v={d.requested ?? "—"} />
          <Kv k={t("reports.fillPrice")} v={d.price ?? "—"} />
          <Kv k={t("reports.clientSlip")} v={ptsStr(d.clientSlipPts)} tone={slipTone2(d.clientSlipPts)} />
          {d.maxDeviationPts !== null && <Kv k={t("detail.maxDeviation")} v={d.maxDeviationPts} />}
          {d.markupOverridePts !== null && <Kv k={t("detail.markupOverride")} v={d.markupOverridePts} />}
          <Kv k={t("reports.status")} v={d.status} />
        </div>
        <div>
          <div className="mb-1 text-xs font-medium text-muted-foreground">{t("detail.verdict")}</div>
          {notes.map((n, i) => <div key={i} className={`text-xs ${n.tone === "good" ? "text-emerald-600 dark:text-emerald-400" : n.tone === "bad" ? "text-red-600 dark:text-red-400" : ""}`}>• {n.text}</div>)}
        </div>
      </div>
      {d.lpOrders && d.lpOrders.length > 0 && (
        <div className="mt-2 grid gap-2">
          {d.lpOrders.map((l) => (
            <div key={l.lpOrderId ?? l.sentAt} className="rounded border border-border/70 p-2">
              <div className="mb-1 text-xs font-medium">LP #{l.lpOrderId} · {l.lp ?? "—"} · {l.side} {l.lots}</div>
              <LpTimeline d={l} f={f} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function LpExecutionDetail({ row }: { row: LpExecution }) {
  const t = useT();
  const f = useFormat();
  if (!row.detail) return <div className="text-xs text-muted-foreground">{t("detail.none")}</div>;
  return (
    <div className="grid gap-3">
      <div className="rounded-md border border-border bg-card p-2">
        <div className="mb-1 text-xs font-medium">LP #{row.id} · {row.lp ?? "—"} · {row.symbol} · {row.side} {row.lots}</div>
        <LpTimeline d={row.detail} f={f} />
      </div>
      {row.clients.filter((c) => c.detail).map((c) => <ClientOrderPanel key={c.orderId} d={c.detail!} f={f} />)}
    </div>
  );
}

function ReconciliationDetail({ row }: { row: ReconciliationRow }) {
  const t = useT();
  const f = useFormat();
  if (!row.detail) return <div className="text-xs text-muted-foreground">{t("detail.none")}</div>;
  const d = row.detail;
  return (
    <div className="grid gap-3">
      <div className="rounded-md border border-border bg-card p-2">
        <div className="mb-1 text-xs font-medium">{t("detail.positionDeals")} · #{row.position}</div>
        <div className="grid grid-cols-[auto_auto_auto_1fr_1fr_1fr_1fr_1fr_1fr] gap-x-3 text-[11px]">
          {[t("audit.at"), t("detail.entry"), t("positions.lots"), t("reports.clientPrice"), t("reports.lpPrice"), t("reports.recon.clientPnl"), t("reports.recon.lpPnl"), t("reports.recon.markup"), t("reports.recon.commission")].map((h) => <span key={h} className="text-muted-foreground">{h}</span>)}
          {d.deals.map((x) => (
            <React.Fragment key={x.dealId}>
              <span>{f.date(x.at)}</span><span>{x.entry} · {x.reason}</span><span className="tabular-nums">{x.lots}</span>
              <span className="tabular-nums">{x.price}</span><span className="tabular-nums">{x.lpPrice ?? "—"}</span>
              <Pnl value={x.pnl}>{f.money(x.pnl)}</Pnl><Pnl value={x.lpPnl}>{f.money(x.lpPnl)}</Pnl><Pnl value={x.markup}>{f.money(x.markup)}</Pnl><Pnl value={x.commission}>{f.money(x.commission)}</Pnl>
            </React.Fragment>
          ))}
        </div>
      </div>
      {d.orders.map((o) => <ClientOrderPanel key={o.orderId} d={o} f={f} />)}
    </div>
  );
}
