"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Dialog, PageHeader, Select } from "@/components/ui/primitives";
import { NumField, SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useT } from "@/lib/hooks";
import { SymbolSpec, Weekday } from "@/lib/schemas";
import { isTradingAt } from "@/lib/sessions";

const col = createColumnHelper<SymbolSpec>();

export default function SymbolsPage() {
  const t = useT();
  const actor = useActor();
  const { data = [] } = useApiQuery("listSymbols");
  const [cat, setCat] = React.useState("all");
  const [editing, setEditing] = React.useState<SymbolSpec | null>(null);
  const [now] = React.useState(() => new Date());

  const columns = [
    col.accessor("name", { header: t("positions.symbol"), cell: (c) => <span className="font-medium">{c.getValue()}</span> }),
    col.accessor("category", { header: "Category", cell: (c) => <Badge tone="muted">{c.getValue()}</Badge> }),
    col.accessor("digits", { header: t("symbols.digits") }),
    col.accessor("contractSize", { header: t("symbols.contract"), cell: (c) => c.getValue().toLocaleString() }),
    col.accessor("marginPct", { header: t("symbols.margin"), cell: (c) => `${c.getValue()}%` }),
    col.display({ id: "lots", header: t("symbols.lots"), cell: (c) => `${c.row.original.minLot} / ${c.row.original.maxLot} / ${c.row.original.lotStep}` }),
    col.accessor("swapLong", { header: t("symbols.swapLong"), cell: (c) => `${c.getValue()} ${c.row.original.swapType === "percent" ? "%" : "pt"}` }),
    col.accessor("swapShort", { header: t("symbols.swapShort"), cell: (c) => `${c.getValue()} ${c.row.original.swapType === "percent" ? "%" : "pt"}` }),
    col.accessor("tripleSwapDay", { header: t("symbols.tripleSwap") }),
    col.display({ id: "now", header: t("common.status"), cell: (c) => isTradingAt(c.row.original.tradeSessions, now) ? <Badge tone="success">{t("symbols.tradingNow")}</Badge> : <Badge tone="muted">{t("symbols.closed")}</Badge> }),
    col.accessor("enabled", { header: t("symbols.enabled"), cell: (c) => c.getValue() ? <Badge tone="success">on</Badge> : <Badge tone="danger">off</Badge> }),
    col.accessor("lp", { header: "LP" }),
  ];

  return (
    <div data-testid="page-symbols">
      <PageHeader title={t("symbols.title")} />
      <DataTable
        data={data.filter((s) => cat === "all" || s.category === cat)}
        columns={columns}
        getRowId={(s) => s.name}
        onRowClick={actor.can("symbols.edit") ? setEditing : undefined}
        toolbar={
          <Select value={cat} onChange={(e) => setCat(e.target.value)} aria-label="category">
            <option value="all">{t("common.all")}</option>
            {["fx", "metal", "index", "energy", "crypto"].map((c) => <option key={c} value={c}>{c}</option>)}
          </Select>
        }
      />
      {editing && <SymbolDialog key={editing.name} sym={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function SymbolDialog({ sym, onClose }: { sym: SymbolSpec; onClose: () => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const { draft, set, errors, validate } = useZodForm(SymbolSpec, sym);
  const mut = useApiMutation((s: SymbolSpec) => api().saveSymbol(s, actor), () => { toast(t("common.saved")); onClose(); });
  const save = () => { const v = validate(); if (v) mut.mutate(v); };
  return (
    <Dialog open wide onClose={onClose} title={`${t("common.edit")}: ${sym.name}`} footer={<>
      <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
      <Button onClick={save} disabled={mut.isPending}>{t("common.save")}</Button>
    </>}>
      <div className="grid gap-3 sm:grid-cols-3">
        <TextField label="Description" value={draft.description} onChange={(v) => set("description", v)} />
        <NumField label={t("symbols.digits")} value={draft.digits} onChange={(v) => set("digits", v)} error={errors.digits} step={1} />
        <NumField label={t("symbols.contract")} value={draft.contractSize} onChange={(v) => set("contractSize", v)} error={errors.contractSize} step={1} />
        <NumField label={t("symbols.margin")} value={draft.marginPct} onChange={(v) => set("marginPct", v)} error={errors.marginPct} />
        <NumField label="Min lot" value={draft.minLot} onChange={(v) => set("minLot", v)} error={errors.minLot} />
        <NumField label="Max lot" value={draft.maxLot} onChange={(v) => set("maxLot", v)} error={errors.maxLot} />
        <NumField label={t("symbols.swapLong")} value={draft.swapLong} onChange={(v) => set("swapLong", v)} error={errors.swapLong} />
        <NumField label={t("symbols.swapShort")} value={draft.swapShort} onChange={(v) => set("swapShort", v)} error={errors.swapShort} />
        <SelectField label={t("symbols.swapType")} value={draft.swapType === "percent" ? "money" : draft.swapType} options={["money", "points"] as const} onChange={(v) => set("swapType", v)} />
        <SelectField label={t("symbols.tripleSwap")} value={draft.tripleSwapDay} options={Weekday.options} onChange={(v) => set("tripleSwapDay", v)} />
        <SelectField label={t("symbols.enabled")} value={draft.enabled ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("enabled", v === "yes")} />
      </div>
      <h3 className="mt-4 mb-1 text-sm font-semibold">{t("symbols.sessions")}</h3>
      <p className="mb-2 text-xs text-muted-foreground">{t("symbols.sessionsHint")}</p>
      <table className="w-full text-sm">
        <tbody>
          {Weekday.options.map((d) => {
            const s = draft.tradeSessions.filter((x) => x.day === d);
            const text = s.map((x) => `${x.open}-${x.close}`).join(", ");
            const apply = (v: string) => {
              const parsed = v.split(",").map((p) => p.trim()).filter(Boolean).map((p) => {
                const m = /^(\d{1,2}:\d{2})\s*-\s*(\d{1,2}:\d{2})$/.exec(p);
                return m ? { day: d, open: m[1].padStart(5, "0"), close: m[2].padStart(5, "0") } : null;
              });
              if (parsed.some((p) => p === null)) return;
              set("tradeSessions", [...draft.tradeSessions.filter((x) => x.day !== d), ...(parsed as { day: typeof d; open: string; close: string }[])]);
            };
            return (
              <tr key={d} className="border-t border-border">
                <td className="py-1 pr-4 uppercase text-muted-foreground">{d}</td>
                <td className="py-1"><input className="w-full rounded-md border border-border bg-background px-2 py-1 font-mono text-xs text-foreground" defaultValue={text} placeholder="—" onBlur={(e) => apply(e.target.value)} data-testid={`session-${d}`} /></td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </Dialog>
  );
}
