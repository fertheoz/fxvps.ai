"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { Download } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Button, PageHeader, Pnl, Tabs } from "@/components/ui/primitives";
import { BookBadge, SideBadge } from "@/components/badges";
import { useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Trade } from "@/lib/schemas";
import type { Statement } from "@/lib/api";
import { formatMinorPlain } from "@/lib/money";
import { downloadCsv, toCsv } from "@/lib/utils";

const tc = createColumnHelper<Trade>();
const sc = createColumnHelper<Statement>();

export default function ReportsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const [tab, setTab] = React.useState<"trades" | "statements">("trades");
  const trades = useApiQuery("listTrades");
  const statements = useApiQuery("statements");

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
    if (tab === "trades") {
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
        <Tabs value={tab} onChange={setTab} items={[{ value: "trades", label: t("reports.trades") }, { value: "statements", label: t("reports.statements") }]} />
      </div>
      {tab === "trades"
        ? <DataTable data={trades.data ?? []} columns={tradeCols} getRowId={(x) => x.id} />
        : <DataTable data={statements.data ?? []} columns={stmtCols} getRowId={(x) => String(x.login)} />}
    </div>
  );
}
