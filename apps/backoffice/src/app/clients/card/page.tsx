"use client";
import * as React from "react";
import { useRouter } from "next/navigation";
import { createColumnHelper } from "@tanstack/react-table";
import { ArrowLeft } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, PageHeader, Pnl, Select, Stat, Tabs } from "@/components/ui/primitives";
import { BookBadge, KycBadge, marginLevel, SideBadge, StatusBadge, ToxicityBadge } from "@/components/badges";
import { BalanceOps } from "@/components/balance-ops";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { KycStatus, type AuditEntry, type Client, type Order, type Position, type Trade } from "@/lib/schemas";
import type { ExecutionRow, RevenueRow } from "@/lib/api";

type Tab = "overview" | "positions" | "trades" | "orders" | "revenue" | "execution" | "statement" | "access" | "audit";
const TABS: Tab[] = ["overview", "positions", "trades", "orders", "revenue", "execution", "statement", "access", "audit"];

const pc = createColumnHelper<Position>();
const tc = createColumnHelper<Trade>();
const oc = createColumnHelper<Order>();
const rc = createColumnHelper<RevenueRow>();
const xc = createColumnHelper<ExecutionRow>();
const ac = createColumnHelper<AuditEntry>();

const noop = () => () => {};
/** Login from `?login=` (static export: the server snapshot is empty, the client reads the URL). */
function useLoginParam(): number | null {
  const search = React.useSyncExternalStore(noop, () => window.location.search, () => "");
  const v = new URLSearchParams(search).get("login");
  return v ? Number(v) : null;
}

const pts = (v: number | null | undefined) => (v === null || v === undefined ? "—" : `${v > 0 ? "+" : ""}${v.toFixed(1)}`);

export default function ClientCardPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const router = useRouter();
  const login = useLoginParam();
  const [tab, setTab] = React.useState<Tab>("overview");
  const clients = useApiQuery("listClients", [{}], { live: 5000 });
  const positions = useApiQuery("listPositions", [], { live: 5000 });
  const orders = useApiQuery("listOrders", [], { live: 5000 });
  const trades = useApiQuery("listTrades", [], { live: 10000 });
  const statements = useApiQuery("statements", [], { live: 10000 });
  const revenue = useApiQuery("revenue", [], { live: 10000 });
  const execution = useApiQuery("execution", [], { live: 10000 });
  const audit = useApiQuery("listAudit", [], { live: 15000 });
  const groups = useApiQuery("listGroups", []);
  const flow = useApiQuery("clientFlow", [], { live: 15000 });
  const client = (clients.data ?? []).find((c) => c.login === login) ?? null;
  const subs = client ? (clients.data ?? []).filter((c) => c.parentId === client.id) : [];
  const kycMut = useApiMutation((k: Client["kyc"]) => api().setKyc(client!.id, k, actor));
  const groupMut = useApiMutation((g: string) => api().setGroup(client!.id, g, actor));
  const leiMut = useApiMutation((lei: string) => api().setProfile(client!.id, { lei: lei.trim() || null }, actor));
  const [lei, setLei] = React.useState<string | null>(null);

  const my = <T extends { login: number }>(rows: T[] | undefined) => (rows ?? []).filter((r) => r.login === login);
  const ps = my(positions.data);
  const os = my(orders.data);
  const ts = my(trades.data);
  const rs = my(revenue.data?.rows);
  const xs = my(execution.data?.rows);
  const st = my(statements.data)[0];
  const au = (audit.data ?? []).filter((a) => login !== null && (a.target.includes(`#${login}`) || a.target === String(login) || a.details.includes(String(login))));

  // Our revenue from this client, by leg and by symbol.
  const rev = React.useMemo(() => {
    const sum = (rows: RevenueRow[], pick: (r: RevenueRow) => number) => rows.reduce((a, r) => a + pick(r), 0);
    const markup = sum(rs.filter((r) => r.kind === "pnl" && r.book === "A"), (r) => r.broker);
    const bBook = sum(rs.filter((r) => r.kind === "pnl" && r.book === "B"), (r) => r.broker);
    const commission = sum(rs.filter((r) => r.kind === "commission"), (r) => r.broker);
    const lp = sum(rs, (r) => r.lp);
    const clientPnl = sum(rs, (r) => r.client);
    const bySymbol = new Map<string, { lots: number; markup: number; commission: number; bBook: number }>();
    for (const r of rs) {
      const s = bySymbol.get(r.symbol) ?? { lots: 0, markup: 0, commission: 0, bBook: 0 };
      if (r.kind === "pnl") {
        s.lots += r.lots;
        if (r.book === "A") s.markup += r.broker;
        else s.bBook += r.broker;
      } else s.commission += r.broker;
      bySymbol.set(r.symbol, s);
    }
    return { markup, bBook, commission, lp, clientPnl, total: markup + bBook + commission, bySymbol: [...bySymbol.entries()].sort((a, b) => b[1].lots - a[1].lots) };
  }, [rs]);
  const wins = ts.filter((x) => x.pnl > 0).length;
  const lots = ts.reduce((a, x) => a + x.lots, 0);
  const avgSlip = xs.length ? xs.reduce((a, x) => a + (x.clientSlipPts ?? 0), 0) / xs.length : null;
  const floating = ps.reduce((a, p) => a + p.pnl, 0);

  if (login === null || clients.isLoading) return <div className="text-sm text-muted-foreground">{t("common.loading")}</div>;
  if (!client) return <div className="text-sm text-muted-foreground">{t("card.notFound")}</div>;
  const ml = marginLevel(client.equity, client.margin);
  const money = (v: number) => f.money(v, client.currency);

  const posCols = [
    pc.accessor("openedAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    pc.accessor("symbol", { header: t("positions.symbol") }),
    pc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    pc.accessor("lots", { header: t("positions.lots") }),
    pc.accessor("openPrice", { header: t("positions.open") }),
    pc.accessor("currentPrice", { header: t("positions.current") }),
    pc.accessor("swap", { header: t("reports.swap"), cell: (c) => money(c.getValue()) }),
    pc.accessor("pnl", { header: t("positions.pnl"), cell: (c) => <Pnl value={c.getValue()}>{money(c.getValue())}</Pnl> }),
    pc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];
  const tradeCols = [
    tc.accessor("closedAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    tc.accessor("symbol", { header: t("positions.symbol") }),
    tc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    tc.accessor("lots", { header: t("positions.lots") }),
    tc.accessor("openPrice", { header: t("positions.open") }),
    tc.accessor("closePrice", { header: t("reports.closing") }),
    tc.accessor("pnl", { header: t("positions.pnl"), cell: (c) => <Pnl value={c.getValue()}>{money(c.getValue())}</Pnl> }),
    tc.accessor("commission", { header: t("reports.commission"), cell: (c) => money(c.getValue()) }),
    tc.accessor("swap", { header: t("reports.swap"), cell: (c) => money(c.getValue()) }),
    tc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];
  const orderCols = [
    oc.accessor("createdAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    oc.accessor("symbol", { header: t("positions.symbol") }),
    oc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    oc.accessor("type", { header: t("reports.kind") }),
    oc.accessor("lots", { header: t("positions.lots") }),
    oc.accessor("price", { header: t("reports.clientPrice") }),
    oc.accessor("status", { header: t("reports.status") }),
  ];
  const revCols = [
    rc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    rc.accessor("symbol", { header: t("positions.symbol") }),
    rc.accessor("lots", { header: t("positions.lots") }),
    rc.accessor("kind", { header: t("reports.kind") }),
    rc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    rc.accessor("price", { header: t("reports.clientPrice") }),
    rc.accessor("lpPrice", { header: t("reports.lpPrice"), cell: (c) => c.getValue() ?? "—" }),
    rc.accessor("client", { header: t("reports.clientLeg"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
    rc.accessor("broker", { header: t("reports.brokerLeg"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
    rc.accessor("lp", { header: t("reports.lpLeg"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
  ];
  const execCols = [
    xc.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    xc.accessor("symbol", { header: t("positions.symbol") }),
    xc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    xc.accessor("type", { header: t("reports.kind") }),
    xc.accessor("lots", { header: t("positions.lots") }),
    xc.accessor("requested", { header: t("reports.requested"), cell: (c) => c.getValue() ?? "—" }),
    xc.accessor("fill", { header: t("reports.fillPrice"), cell: (c) => c.getValue() ?? "—" }),
    xc.accessor("clientSlipPts", { header: t("reports.clientSlip"), cell: (c) => pts(c.getValue()) }),
    xc.accessor("lpLatencyMs", { header: t("reports.lpLatency"), cell: (c) => (c.getValue() === null ? "—" : `${Math.round(c.getValue() ?? 0)} ms`) }),
    xc.accessor("status", { header: t("reports.status") }),
  ];
  const auditCols = [
    ac.accessor("at", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
    ac.accessor("actor", { header: t("audit.actor") }),
    ac.accessor("action", { header: t("audit.action") }),
    ac.accessor("details", { header: t("audit.details") }),
  ];

  return (
    <div data-testid="page-client-card">
      <PageHeader title={`${client.name} · #${client.login}`}>
        <Button variant="outline" onClick={() => router.push("/clients/")}><ArrowLeft className="h-4 w-4" />{t("card.back")}</Button>
      </PageHeader>
      <div className="mb-3 flex flex-wrap items-center gap-2 text-sm">
        <StatusBadge status={client.status} />
        <KycBadge kyc={client.kyc} />
        <Badge tone="muted">{client.group}</Badge>
        <Badge tone="muted">1:{client.leverage}</Badge>
        {(() => { const fl = (flow.data ?? []).find((r) => r.login === client.login); return fl ? <span className="flex items-center gap-1 text-xs text-muted-foreground">{t("flow.toxicity")} <ToxicityBadge score={fl.toxicity} /></span> : null; })()}
        <span className="text-muted-foreground">{client.email} · {client.country} · {t("clients.lastIp")} {client.lastIp}</span>
        {client.parentId && <Badge tone="muted">{t("card.subOf")} {(clients.data ?? []).find((c) => c.id === client.parentId)?.login}</Badge>}
      </div>

      <div className="grid grid-cols-2 gap-3 md:grid-cols-4 xl:grid-cols-8">
        <Stat label={t("clients.balance")} value={money(client.balance)} sub={client.credit ? `${t("clients.credit")} ${money(client.credit)}` : undefined} />
        <Stat label={t("clients.equity")} value={money(client.equity)} sub={`${t("dash.floating")} ${money(floating)}`} tone={floating < 0 ? "down" : "up"} />
        <Stat label={t("card.margin")} value={money(client.margin)} sub={`${t("card.free")} ${money(client.equity - client.margin)}`} />
        <Stat label={t("clients.marginLevel")} value={ml === null ? "—" : `${f.num(ml, 0)}%`} tone={ml !== null && ml < 100 ? "down" : undefined} />
        <Stat label={t("card.ourRevenue")} value={f.money(rev.total)} sub={`${t("reports.markup")} ${f.money(rev.markup)} · ${t("reports.commission")} ${f.money(rev.commission)}`} tone={rev.total < 0 ? "down" : "up"} />
        <Stat label={t("card.clientRealised")} value={f.money(rev.clientPnl)} tone={rev.clientPnl < 0 ? "down" : "up"} />
        <Stat label={t("card.trades")} value={`${ts.length}`} sub={ts.length ? `${Math.round((wins / ts.length) * 100)}% ${t("card.winRate")} · ${lots.toFixed(2)} lot` : undefined} />
        <Stat label={t("reports.avgSlip")} value={avgSlip === null ? "—" : pts(avgSlip)} sub={`${xs.length} ${t("dash.orders").toLowerCase()}`} />
      </div>

      <div className="my-4 overflow-x-auto">
        <Tabs value={tab} onChange={setTab} items={TABS.map((v) => ({ value: v, label: t(`card.tab.${v}`) }))} />
      </div>

      {tab === "overview" && (
        <div className="grid gap-4 xl:grid-cols-3">
          <Card className="xl:col-span-2">
            <CardHeader><CardTitle>{t("card.revenueBySymbol")}</CardTitle></CardHeader>
            <CardContent>
              {rev.bySymbol.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noData")}</div>}
              <table className="w-full text-sm tabular-nums">
                <thead className="text-left text-xs text-muted-foreground"><tr><th className="py-1 font-normal">{t("positions.symbol")}</th><th className="text-right font-normal">{t("positions.lots")}</th><th className="text-right font-normal">{t("reports.markup")}</th><th className="text-right font-normal">{t("reports.commission")}</th><th className="text-right font-normal">{t("reports.bBook")}</th><th className="text-right font-normal">{t("card.total")}</th></tr></thead>
                <tbody>
                  {rev.bySymbol.map(([s, v]) => (
                    <tr key={s} className="border-t border-border/60">
                      <td className="py-1 font-medium">{s}</td>
                      <td className="text-right">{v.lots.toFixed(2)}</td>
                      <td className="text-right"><Pnl value={v.markup}>{f.money(v.markup)}</Pnl></td>
                      <td className="text-right"><Pnl value={v.commission}>{f.money(v.commission)}</Pnl></td>
                      <td className="text-right"><Pnl value={v.bBook}>{f.money(v.bBook)}</Pnl></td>
                      <td className="text-right"><Pnl value={v.markup + v.commission + v.bBook}>{f.money(v.markup + v.commission + v.bBook)}</Pnl></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </CardContent>
          </Card>
          <Card>
            <CardHeader><CardTitle>{t("card.breakdown")}</CardTitle></CardHeader>
            <CardContent className="grid grid-cols-2 gap-x-3 gap-y-1.5 text-sm tabular-nums">
              <span className="text-muted-foreground">{t("reports.markup")}</span><span className="text-right"><Pnl value={rev.markup}>{f.money(rev.markup)}</Pnl></span>
              <span className="text-muted-foreground">{t("reports.commission")}</span><span className="text-right"><Pnl value={rev.commission}>{f.money(rev.commission)}</Pnl></span>
              <span className="text-muted-foreground">{t("reports.bBook")}</span><span className="text-right"><Pnl value={rev.bBook}>{f.money(rev.bBook)}</Pnl></span>
              <span className="text-muted-foreground">{t("reports.swap")}</span><span className="text-right">{st ? money(st.swap) : "—"}</span>
              <span className="text-muted-foreground">{t("reports.lpLeg")}</span><span className="text-right"><Pnl value={rev.lp}>{f.money(rev.lp)}</Pnl></span>
              <span className="font-medium">{t("card.ourRevenue")}</span><span className="text-right font-medium"><Pnl value={rev.total}>{f.money(rev.total)}</Pnl></span>
              <span className="text-muted-foreground">{t("reports.deposits")}</span><span className="text-right">{st ? money(st.deposits) : "—"}</span>
              <span className="text-muted-foreground">{t("reports.withdrawals")}</span><span className="text-right">{st ? money(st.withdrawals) : "—"}</span>
              <span className="text-muted-foreground">{t("card.openPositions")}</span><span className="text-right">{ps.length}</span>
              <span className="text-muted-foreground">{t("card.workingOrders")}</span><span className="text-right">{os.length}</span>
              {subs.length > 0 && (<><span className="text-muted-foreground">{t("clients.subs")}</span><span className="text-right">{subs.map((s) => s.login).join(", ")}</span></>)}
            </CardContent>
          </Card>
        </div>
      )}
      {tab === "positions" && <DataTable data={ps} columns={posCols} getRowId={(x) => x.id} testId="card-positions" />}
      {tab === "trades" && <DataTable data={ts} columns={tradeCols} getRowId={(x) => x.id} testId="card-trades" />}
      {tab === "orders" && <DataTable data={os} columns={orderCols} getRowId={(x) => x.id} testId="card-orders" />}
      {tab === "revenue" && <DataTable data={rs} columns={revCols} getRowId={(x) => x.id} testId="card-revenue" />}
      {tab === "execution" && <DataTable data={xs} columns={execCols} getRowId={(x) => x.id} testId="card-execution" />}
      {tab === "statement" && (
        <Card>
          <CardContent className="grid grid-cols-2 gap-x-3 gap-y-1.5 pt-4 text-sm tabular-nums md:grid-cols-4">
            <div className="col-span-2 md:col-span-4"><Button variant="outline" onClick={() => router.push(`/clients/statement/?login=${client.login}`)} data-testid="card-statement-print">{t("card.printStatement")}</Button></div>
            {st ? (
              ([["opening", "reports.opening"], ["deposits", "reports.deposits"], ["withdrawals", "reports.withdrawals"], ["pnl", "positions.pnl"], ["commission", "reports.commission"], ["swap", "reports.swap"], ["closing", "reports.closing"]] as const).map(([k, label]) => (
                <React.Fragment key={k}><span className="text-muted-foreground">{t(label)}</span><span className="text-right">{money(st[k])}</span></React.Fragment>
              ))
            ) : (
              <span className="text-muted-foreground">{t("dash.noData")}</span>
            )}
          </CardContent>
        </Card>
      )}
      {tab === "access" && (
        <div className="grid gap-4 md:grid-cols-2">
          <Card>
            <CardHeader><CardTitle>{t("card.accessSettings")}</CardTitle></CardHeader>
            <CardContent className="grid gap-3 text-sm">
              <label className="grid gap-1">{t("clients.kyc")}
                <Select value={client.kyc} disabled={!actor.can("clients.edit") || kycMut.isPending} onChange={(e) => kycMut.mutate(e.target.value as Client["kyc"])}>
                  {KycStatus.options.map((k) => <option key={k} value={k}>{k}</option>)}
                </Select>
              </label>
              <label className="grid gap-1">{t("clients.group")}
                <Select value={client.group} disabled={!actor.can("clients.edit") || groupMut.isPending} onChange={(e) => groupMut.mutate(e.target.value)}>
                  {(groups.data ?? []).map((g) => <option key={g.id} value={g.name}>{g.name} · {g.marginMode === "retail_netting" ? "netting" : "hedging"} · 1:{g.leverage}</option>)}
                </Select>
                <span className="text-xs text-muted-foreground">{t("clients.groupHint")}</span>
                {groupMut.error && <span className="text-xs text-red-600 dark:text-red-400">{String((groupMut.error as Error).message ?? groupMut.error)}</span>}
              </label>
              <label className="grid gap-1">{t("clients.lei")}
                <span className="flex gap-2">
                  <input className="w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-sm uppercase text-foreground" maxLength={20} value={lei ?? client.lei ?? ""} onChange={(e) => setLei(e.target.value.toUpperCase())} disabled={!actor.can("clients.edit")} data-testid="card-lei" />
                  {actor.can("clients.edit") && <Button size="sm" variant="outline" disabled={lei === null || leiMut.isPending} onClick={() => { leiMut.mutate(lei ?? ""); setLei(null); }}>{t("common.save")}</Button>}
                </span>
                <span className="text-xs text-muted-foreground">{t("clients.leiHint")}</span>
                {leiMut.error && <span className="text-xs text-red-600 dark:text-red-400">{String((leiMut.error as Error).message ?? leiMut.error)}</span>}
              </label>
              <div className="text-xs text-muted-foreground">{t("card.accessHint")}</div>
            </CardContent>
          </Card>
          <Card><CardContent className="pt-4"><BalanceOps client={client} /></CardContent></Card>
        </div>
      )}
      {tab === "audit" && <DataTable data={au} columns={auditCols} getRowId={(x) => x.id} testId="card-audit" />}
    </div>
  );
}
