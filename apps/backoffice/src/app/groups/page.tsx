"use client";
import * as React from "react";
import { AlgoFields } from "@/components/algo-fields";
import { createColumnHelper } from "@tanstack/react-table";
import { DataTable } from "@/components/ui/data-table";
import { Button, Dialog, FieldError, PageHeader } from "@/components/ui/primitives";
import { BookBadge } from "@/components/badges";
import { NumField, SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { Book, CommissionType, EsmaCap, Group, MarginMode, PartialFillPolicy } from "@/lib/schemas";

const col = createColumnHelper<Group>();

export default function GroupsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const { data = [] } = useApiQuery("listGroups");
  const [editing, setEditing] = React.useState<Group | null>(null);

  const commission = (g: Group) =>
    g.commissionType === "per_lot" ? `${f.money(g.commissionValue, g.currency)} / lot`
      : g.commissionType === "per_million" ? `${f.money(g.commissionValue, g.currency)} / 1M`
        : `${g.commissionValue} bp`;

  const columns = [
    col.accessor("name", { header: "Name", cell: (c) => <span className="font-medium">{c.getValue()}</span> }),
    col.accessor("currency", { header: t("groups.currency") }),
    col.accessor("leverage", { header: t("groups.leverage"), cell: (c) => `1:${c.getValue()}` }),
    col.accessor("esma", { header: "ESMA", cell: (c) => (c.getValue() === "none" ? "—" : c.getValue()) }),
    col.accessor("partialFill", { header: t("groups.partialFillShort"), cell: (c) => (c.getValue() === "retry" ? `retry ×${c.row.original.maxAttempts}` : c.getValue()) }),
    col.accessor("marginMode", { header: t("groups.marginMode") }),
    col.accessor("marginCallPct", { header: t("groups.marginCall"), cell: (c) => `${c.getValue()}%` }),
    col.accessor("stopOutPct", { header: t("groups.stopOut"), cell: (c) => `${c.getValue()}%` }),
    col.display({ id: "comm", header: t("groups.commission"), cell: (c) => commission(c.row.original) }),
    col.accessor("markupPoints", { header: t("groups.markup") }),
    col.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];

  return (
    <div data-testid="page-groups">
      <PageHeader title={t("groups.title")} />
      <DataTable data={data} columns={columns} onRowClick={actor.can("groups.edit") ? setEditing : undefined} getRowId={(g) => g.id} />
      {editing && <GroupDialog key={editing.id} group={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

/** "EURUSD=7, XAUUSD=20" -> { EURUSD: 7, XAUUSD: 20 } (bad entries dropped). */
function parseSymbolMarkups(text: string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const part of text.split(/[,\s]+/)) {
    const m = /^([A-Za-z0-9._-]+)=(\d{1,4})$/.exec(part.trim());
    if (m && m[1] && m[2]) out[m[1].toUpperCase()] = Number(m[2]);
  }
  return out;
}

function GroupDialog({ group, onClose }: { group: Group; onClose: () => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const { draft, set, errors, validate } = useZodForm(Group, group);
  const mut = useApiMutation((g: Group) => api().saveGroup(g, actor), () => { toast(t("common.saved")); onClose(); });
  const save = () => { const v = validate(); if (v) mut.mutate(v); };
  return (
    <Dialog open wide onClose={onClose} title={`${t("groups.editTitle")}: ${group.name}`} footer={<>
      <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
      <Button onClick={save} disabled={mut.isPending}>{t("common.save")}</Button>
    </>}>
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField label="Name" value={draft.name} onChange={(v) => set("name", v)} error={errors.name} />
        <NumField label={t("groups.leverage")} value={draft.leverage} onChange={(v) => set("leverage", v)} error={errors.leverage} step={1} />
        <SelectField label={t("groups.marginMode")} value={draft.marginMode} options={MarginMode.options} onChange={(v) => set("marginMode", v)} />
        <SelectField label={t("groups.esma")} value={draft.esma} options={EsmaCap.options} onChange={(v) => set("esma", v)} />
        <SelectField label={t("groups.partialFill")} value={draft.partialFill} options={PartialFillPolicy.options} onChange={(v) => set("partialFill", v)} />
        <NumField label={t("groups.maxAttempts")} value={draft.maxAttempts} onChange={(v) => set("maxAttempts", v)} error={errors.maxAttempts} step={1} disabled={draft.partialFill !== "retry"} />
        <SelectField label={t("groups.book")} value={draft.book} options={Book.options} onChange={(v) => set("book", v)} />
        <NumField label={t("groups.marginCall")} value={draft.marginCallPct} onChange={(v) => set("marginCallPct", v)} error={errors.marginCallPct} />
        <NumField label={t("groups.stopOut")} value={draft.stopOutPct} onChange={(v) => set("stopOutPct", v)} error={errors.stopOutPct} />
        <SelectField label={`${t("groups.commission")} type`} value={draft.commissionType} options={CommissionType.options} onChange={(v) => set("commissionType", v)} />
        <NumField label={`${t("groups.commission")} (minor units / bp)`} value={draft.commissionValue} onChange={(v) => set("commissionValue", v)} error={errors.commissionValue} step={1} />
        <NumField label={t("groups.markup")} value={draft.markupPoints} onChange={(v) => set("markupPoints", v)} error={errors.markupPoints} step={1} />
        <NumField label={t("groups.markupBid")} value={draft.markupBidPoints ?? draft.markupPoints} onChange={(v) => set("markupBidPoints", v === draft.markupPoints ? null : v)} step={1} />
        <NumField label={t("groups.markupAsk")} value={draft.markupAskPoints ?? draft.markupPoints} onChange={(v) => set("markupAskPoints", v === draft.markupPoints ? null : v)} step={1} />
        <TextField
          label={t("groups.symbolMarkups")}
          value={Object.entries(draft.symbolMarkups).map(([s, p]) => `${s}=${p}`).join(", ")}
          onChange={(v) => set("symbolMarkups", parseSymbolMarkups(v))}
        />
        <NumField label={t("groups.maxSlippage")} value={draft.maxSlippagePoints ?? 0} onChange={(v) => set("maxSlippagePoints", v > 0 ? v : null)} step={1} />
        <NumField label={t("groups.weekendLeverage")} value={draft.weekendLeverage ?? 0} onChange={(v) => set("weekendLeverage", v > 0 ? v : null)} step={1} />
        <LeverageWindowsField value={draft.leverageWindows ?? []} onChange={(v) => set("leverageWindows", v)} />
        <MarkupWindowsField value={draft.markupWindows ?? []} onChange={(v) => set("markupWindows", v)} />
        <NumField label={t("groups.newsMarkupWindow")} value={draft.newsMarkup?.windowMin ?? 0} onChange={(v) => set("newsMarkup", v > 0 ? { windowMin: Math.min(1440, v), addPoints: draft.newsMarkup?.addPoints ?? 0 } : null)} step={5} />
        <NumField label={t("groups.newsMarkupPoints")} value={draft.newsMarkup?.addPoints ?? 0} onChange={(v) => set("newsMarkup", draft.newsMarkup ? { ...draft.newsMarkup, addPoints: v } : v !== 0 ? { windowMin: 30, addPoints: v } : null)} step={1} />
        <MarkupBandsField value={draft.markupBands ?? []} onChange={(v) => set("markupBands", v)} />
        <NumField label={t("groups.minSpread")} value={draft.minSpreadPoints ?? 0} onChange={(v) => set("minSpreadPoints", v > 0 ? Math.trunc(v) : null)} step={1} />
        <NumField label={t("groups.maxSpread")} value={draft.maxSpreadPoints ?? 0} onChange={(v) => set("maxSpreadPoints", v > 0 ? Math.trunc(v) : null)} step={1} />
        <NumField label={t("groups.skewPerLot")} value={draft.skew?.pointsPerLot ?? 0} onChange={(v) => set("skew", v > 0 ? { pointsPerLot: v, maxPoints: draft.skew?.maxPoints ?? 30 } : null)} step={0.1} />
        <NumField label={t("groups.skewMax")} value={draft.skew?.maxPoints ?? 0} onChange={(v) => set("skew", draft.skew ? { ...draft.skew, maxPoints: Math.max(0, Math.trunc(v)) } : v > 0 ? { pointsPerLot: 1, maxPoints: Math.trunc(v) } : null)} step={5} />
        <AlgoFields draft={draft} set={set} />
        <NumField label={t("groups.lastLookHold")} value={draft.lastLook?.holdMs ?? 0} onChange={(v) => set("lastLook", v > 0 ? { holdMs: Math.min(10000, Math.trunc(v)), maxMovePoints: draft.lastLook?.maxMovePoints ?? 5 } : null)} step={50} />
        <NumField label={t("groups.lastLookMove")} value={draft.lastLook?.maxMovePoints ?? 0} onChange={(v) => set("lastLook", draft.lastLook ? { ...draft.lastLook, maxMovePoints: Math.max(0, Math.trunc(v)) } : null)} step={1} />
        <SelectField label={t("groups.priceImprovement")} value={draft.passPriceImprovement ? "client" : "broker"} options={["client", "broker"] as const} onChange={(v) => set("passPriceImprovement", v === "client")} />
        <SelectField label={t("groups.lpResting")} value={draft.lpResting ? "lp" : "trigger"} options={["lp", "trigger"] as const} onChange={(v) => set("lpResting", v === "lp")} />
        <NumField label={t("groups.swapMult")} value={draft.swapMultiplier} onChange={(v) => set("swapMultiplier", v)} error={errors.swapMultiplier} step={0.1} />
        {draft.swapMultiplier === 0 && (
          <>
            <NumField label={t("groups.swapFreeFee")} value={draft.swapFreeFee ?? 0} onChange={(v) => set("swapFreeFee", v)} step={0.5} />
            <NumField label={t("groups.swapFreeGrace")} value={draft.swapFreeGraceDays ?? 0} onChange={(v) => set("swapFreeGraceDays", v)} step={1} />
          </>
        )}
        <LeverageTiersField value={draft.leverageTiers ?? []} onChange={(v) => set("leverageTiers", v)} />
      </div>
      <FieldError msg={errors._} />
    </Dialog>
  );
}

/** News-event leverage caps: [from, to) in local time, applied as UTC instants by the engine. */
function LeverageWindowsField({ value, onChange }: { value: { fromMs: number; toMs: number; leverage: number }[]; onChange: (v: { fromMs: number; toMs: number; leverage: number }[]) => void }) {
  const t = useT();
  const local = (ms: number) => {
    const d = new Date(ms);
    const p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
  };
  const upd = (i: number, w: Partial<{ fromMs: number; toMs: number; leverage: number }>) => onChange(value.map((x, k) => (k === i ? { ...x, ...w } : x)));
  return (
    <div className="sm:col-span-2 space-y-2">
      <div className="text-xs font-medium text-muted-foreground">{t("groups.newsWindows")}</div>
      {value.map((w, i) => (
        <div key={i} className="grid grid-cols-[1fr_1fr_6rem_auto] items-end gap-2" data-testid="leverage-window">
          <label className="grid gap-1 text-sm"><span className="text-xs text-muted-foreground">{t("groups.newsFrom")}</span><input type="datetime-local" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={local(w.fromMs)} onChange={(e) => upd(i, { fromMs: new Date(e.target.value).getTime() })} /></label>
          <label className="grid gap-1 text-sm"><span className="text-xs text-muted-foreground">{t("groups.newsTo")}</span><input type="datetime-local" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={local(w.toMs)} onChange={(e) => upd(i, { toMs: new Date(e.target.value).getTime() })} /></label>
          <NumField label="1:N" value={w.leverage} onChange={(v) => upd(i, { leverage: v })} step={1} />
          <Button type="button" variant="outline" size="sm" onClick={() => onChange(value.filter((_, k) => k !== i))}>×</Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" onClick={() => { const from = Date.now() + 3_600_000; onChange([...value, { fromMs: from - (from % 60_000), toMs: from - (from % 60_000) + 1_800_000, leverage: 50 }]); }} data-testid="add-leverage-window">
        + {t("groups.newsAdd")}
      </Button>
    </div>
  );
}

type MarkupWindow = { weekdays: number[]; fromMin: number; toMin: number; addPoints: number };
const DAY_KEYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const hhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
const minutes = (s: string) => { const [h = 0, m = 0] = s.split(":").map(Number); return Number.isFinite(h) ? Math.min(1440, h * 60 + (Number.isFinite(m) ? m : 0)) : 0; };

/** Scheduled markup windows: extra points inside a daily UTC window on the chosen weekdays. */
function MarkupWindowsField({ value, onChange }: { value: MarkupWindow[]; onChange: (v: MarkupWindow[]) => void }) {
  const t = useT();
  const upd = (i: number, w: Partial<MarkupWindow>) => onChange(value.map((x, k) => (k === i ? { ...x, ...w } : x)));
  return (
    <div className="sm:col-span-2 space-y-2">
      <div className="text-xs font-medium text-muted-foreground">{t("groups.markupWindows")}</div>
      {value.map((w, i) => (
        <div key={i} className="grid grid-cols-[1fr_6rem_6rem_6rem_auto] items-end gap-2" data-testid="markup-window">
          <label className="grid gap-1 text-sm"><span className="text-xs text-muted-foreground">{t("groups.weekdays")}</span>
            <input className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={w.weekdays.map((d) => DAY_KEYS[d]).join(", ")} placeholder={t("groups.weekdaysHint")} onChange={(e) => upd(i, { weekdays: e.target.value.split(/[\s,]+/).map((x) => DAY_KEYS.indexOf(x.trim().toLowerCase())).filter((d) => d >= 0) })} />
          </label>
          <label className="grid gap-1 text-sm"><span className="text-xs text-muted-foreground">{t("groups.fromUtc")}</span><input type="time" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={hhmm(w.fromMin)} onChange={(e) => upd(i, { fromMin: minutes(e.target.value) })} /></label>
          <label className="grid gap-1 text-sm"><span className="text-xs text-muted-foreground">{t("groups.toUtc")}</span><input type="time" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={hhmm(w.toMin % 1440)} onChange={(e) => upd(i, { toMin: minutes(e.target.value) })} /></label>
          <NumField label={t("groups.addPoints")} value={w.addPoints} onChange={(v) => upd(i, { addPoints: v })} step={1} />
          <Button type="button" variant="outline" size="sm" onClick={() => onChange(value.filter((_, k) => k !== i))}>×</Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" onClick={() => onChange([...value, { weekdays: [], fromMin: 21 * 60 + 55, toMin: 22 * 60 + 10, addPoints: 5 }])} data-testid="add-markup-window">
        + {t("groups.markupWindowAdd")}
      </Button>
    </div>
  );
}

/** Volume bands: orders of at least `fromLots` get `addPoints` more markup. */
function MarkupBandsField({ value, onChange }: { value: { fromLots: number; addPoints: number }[]; onChange: (v: { fromLots: number; addPoints: number }[]) => void }) {
  const t = useT();
  const upd = (i: number, w: Partial<{ fromLots: number; addPoints: number }>) => onChange(value.map((x, k) => (k === i ? { ...x, ...w } : x)));
  return (
    <div className="sm:col-span-2 space-y-2">
      <div className="text-xs font-medium text-muted-foreground">{t("groups.markupBands")}</div>
      {value.map((w, i) => (
        <div key={i} className="grid grid-cols-[8rem_8rem_auto] items-end gap-2" data-testid="markup-band">
          <NumField label={t("groups.bandFrom")} value={w.fromLots} onChange={(v) => upd(i, { fromLots: v })} step={0.1} />
          <NumField label={t("groups.addPoints")} value={w.addPoints} onChange={(v) => upd(i, { addPoints: v })} step={1} />
          <Button type="button" variant="outline" size="sm" onClick={() => onChange(value.filter((_, k) => k !== i))}>×</Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" onClick={() => onChange([...value, { fromLots: 1, addPoints: 2 }])} data-testid="add-markup-band">
        + {t("groups.bandAdd")}
      </Button>
    </div>
  );
}

/** Volume leverage tiers: the notional above each threshold gets at most that leverage. */
function LeverageTiersField({ value, onChange }: { value: { from: number; leverage: number }[]; onChange: (v: { from: number; leverage: number }[]) => void }) {
  const t = useT();
  const upd = (i: number, w: Partial<{ from: number; leverage: number }>) => onChange(value.map((x, k) => (k === i ? { ...x, ...w } : x)));
  return (
    <div className="sm:col-span-2 space-y-2" data-testid="leverage-tiers">
      <div className="text-xs font-medium text-muted-foreground">{t("groups.leverageTiers")}</div>
      {value.map((w, i) => (
        <div key={i} className="grid grid-cols-[1fr_6rem_auto] items-end gap-2">
          <NumField label={t("groups.tierFrom")} value={w.from} onChange={(v) => upd(i, { from: v })} step={50000} />
          <NumField label="1:N" value={w.leverage} onChange={(v) => upd(i, { leverage: v })} step={1} />
          <Button type="button" variant="outline" size="sm" onClick={() => onChange(value.filter((_, k) => k !== i))}>×</Button>
        </div>
      ))}
      <Button type="button" variant="outline" size="sm" onClick={() => { const last = value[value.length - 1]; onChange([...value, { from: last ? last.from * 5 : 100000, leverage: last ? Math.max(1, Math.floor(last.leverage / 2)) : 50 }]); }}>
        + {t("groups.tierAdd")}
      </Button>
    </div>
  );
}
