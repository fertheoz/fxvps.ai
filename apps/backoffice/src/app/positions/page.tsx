"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Dialog, PageHeader, Pnl, Select, Tabs } from "@/components/ui/primitives";
import { BookBadge, SideBadge } from "@/components/badges";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Order, Position } from "@/lib/schemas";

const pc = createColumnHelper<Position>();
const oc = createColumnHelper<Order>();

export default function PositionsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const [tab, setTab] = React.useState<"positions" | "orders">("positions");
  const [symbol, setSymbol] = React.useState("all");
  const [book, setBook] = React.useState("all");
  const [selected, setSelected] = React.useState<Position[]>([]);
  const [confirm, setConfirm] = React.useState<string[] | null>(null);
  const positions = useApiQuery("listPositions", [], { live: 2000 });
  const orders = useApiQuery("listOrders", [], { live: 5000 });
  const close = useApiMutation((ids: string[]) => api().forceClose(ids, actor), (r) => {
    toast(`${t("positions.forceClose")}: ${r.closed}`);
    setConfirm(null);
    setSelected([]);
  });
  const canClose = actor.can("positions.forceClose");

  const posData = (positions.data ?? []).filter((p) => (symbol === "all" || p.symbol === symbol) && (book === "all" || p.book === book));
  const symbols = [...new Set((positions.data ?? []).map((p) => p.symbol))].sort();

  const posCols = [
    pc.accessor("id", { header: "ID" }),
    pc.accessor("login", { header: t("clients.login") }),
    pc.accessor("symbol", { header: t("positions.symbol"), cell: (c) => <span className="font-medium">{c.getValue()}</span> }),
    pc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    pc.accessor("lots", { header: t("positions.lots"), cell: (c) => <span className="tabular-nums">{f.num(c.getValue())}</span> }),
    pc.accessor("openPrice", { header: t("positions.open"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    pc.accessor("currentPrice", { header: t("positions.current"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    pc.accessor("pnl", { header: t("positions.pnl"), cell: (c) => <Pnl value={c.getValue()}>{f.money(c.getValue())}</Pnl> }),
    pc.accessor("swap", { header: t("reports.swap"), cell: (c) => <span className="tabular-nums">{f.money(c.getValue())}</span> }),
    pc.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
    pc.display({
      id: "act", header: "",
      cell: (c) => canClose ? <Button size="sm" variant="outline" onClick={(e) => { e.stopPropagation(); setConfirm([c.row.original.id]); }}>{t("positions.forceClose")}</Button> : null,
    }),
  ];
  const ordCols = [
    oc.accessor("id", { header: "ID" }),
    oc.accessor("login", { header: t("clients.login") }),
    oc.accessor("symbol", { header: t("positions.symbol") }),
    oc.accessor("side", { header: t("positions.side"), cell: (c) => <SideBadge side={c.getValue()} /> }),
    oc.accessor("type", { header: t("positions.type") }),
    oc.accessor("lots", { header: t("positions.lots") }),
    oc.accessor("price", { header: t("positions.price"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    oc.accessor("status", { header: t("common.status"), cell: (c) => <Badge tone={c.getValue() === "pending_lp" ? "warning" : "info"}>{c.getValue()}</Badge> }),
    oc.accessor("createdAt", { header: t("audit.at"), cell: (c) => f.date(c.getValue()) }),
  ];

  const total = posData.reduce((a, p) => a + p.pnl, 0);

  return (
    <div data-testid="page-positions">
      <PageHeader title={t("positions.title")}>
        <Badge tone="success" className="gap-1"><span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />{t("positions.live")}</Badge>
        <span className="text-sm">{t("positions.pnl")}: <Pnl value={total}>{f.money(total)}</Pnl></span>
      </PageHeader>
      <div className="mb-3">
        <Tabs value={tab} onChange={setTab} items={[
          { value: "positions", label: `${t("positions.positions")} (${positions.data?.length ?? 0})` },
          { value: "orders", label: `${t("positions.orders")} (${orders.data?.length ?? 0})` },
        ]} />
      </div>
      {tab === "positions" ? (
        <DataTable
          data={posData}
          columns={posCols}
          getRowId={(p) => p.id}
          selectable={canClose}
          onSelectionChange={setSelected}
          testId="positions-table"
          toolbar={<>
            <Select value={symbol} onChange={(e) => setSymbol(e.target.value)} aria-label={t("positions.symbol")}>
              <option value="all">{t("positions.symbol")}: {t("common.all")}</option>
              {symbols.map((s) => <option key={s}>{s}</option>)}
            </Select>
            <Select value={book} onChange={(e) => setBook(e.target.value)} aria-label={t("groups.book")}>
              <option value="all">{t("groups.book")}: {t("common.all")}</option>
              <option value="A">A-book</option>
              <option value="B">B-book</option>
            </Select>
            {canClose && selected.length > 0 && (
              <Button variant="destructive" onClick={() => setConfirm(selected.map((p) => p.id))}>{t("positions.forceCloseSelected")} ({selected.length})</Button>
            )}
          </>}
        />
      ) : (
        <DataTable data={orders.data ?? []} columns={ordCols} getRowId={(o) => o.id} />
      )}
      <Dialog
        open={!!confirm}
        onClose={() => setConfirm(null)}
        title={t("positions.forceClose")}
        footer={<>
          <Button variant="outline" onClick={() => setConfirm(null)}>{t("common.cancel")}</Button>
          <Button variant="destructive" disabled={close.isPending} onClick={() => confirm && close.mutate(confirm)}>{t("common.confirm")}</Button>
        </>}
      >
        <p className="text-sm">{t("positions.confirmClose", { n: confirm?.length ?? 0 })}</p>
      </Dialog>
    </div>
  );
}
