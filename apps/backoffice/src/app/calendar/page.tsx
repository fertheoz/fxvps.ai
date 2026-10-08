"use client";
import * as React from "react";
import { Badge, Button, Card, Dialog, FieldError, Input, Label, PageHeader, Select } from "@/components/ui/primitives";
import { NumField, SelectField, TextField } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { EconEvent, EconEventInput, EconImpact } from "@/lib/api";
import type { Group } from "@/lib/schemas";

const IMPACTS: readonly EconImpact[] = ["low", "medium", "high"];
const IMPACT_TONE = { low: "muted", medium: "warning", high: "danger" } as const;
const DAY_MS = 86_400_000;

/** `YYYY-MM-DD` (UTC) of epoch ms. */
const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);
/** `datetime-local` value in UTC and back. */
const toInputUtc = (ms: number) => new Date(ms).toISOString().slice(0, 16);
const fromInputUtc = (v: string) => Date.parse(`${v}:00Z`);

/**
 * Economic calendar (plan item 10): releases clients see in the terminal's
 * calendar and as chart pins. "Import this week" pulls the free ForexFactory
 * feed on the server; a high-impact event can get a leverage window.
 */
export default function CalendarPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const mfaOk = useMfaOk();
  const toast = useToast();
  const editable = actor.can("settings.edit") && mfaOk;
  const [from, setFrom] = React.useState(() => isoDay(Date.now() - 7 * DAY_MS));
  const [to, setTo] = React.useState(() => isoDay(Date.now() + 14 * DAY_MS));
  const [ccy, setCcy] = React.useState("");
  const [editing, setEditing] = React.useState<EconEvent | "new" | null>(null);
  const [leverageFor, setLeverageFor] = React.useState<{ ev: EconEvent; offered: boolean } | null>(null);
  // the API's `to` is exclusive: include the whole last day
  const q = useApiQuery("listEconEvents", [from, isoDay(Date.parse(to) + DAY_MS)]);
  const importWeek = useApiMutation(() => api().importEconWeek(actor), (r) => toast(t("econ.imported", { added: r.added, updated: r.updated, unchanged: r.unchanged })));
  const remove = useApiMutation((id: string) => api().deleteEconEvent(id, actor), () => toast(t("econ.deleted")));
  const events = q.data?.events ?? [];
  const currencies = [...new Set(events.map((e) => e.currency))].sort();
  const shown = ccy ? events.filter((e) => e.currency === ccy || e.currency === "ALL") : events;
  const days = new Map<string, EconEvent[]>();
  for (const e of shown) days.set(isoDay(e.time), [...(days.get(isoDay(e.time)) ?? []), e]);
  const dayLabel = (d: string) => new Intl.DateTimeFormat(f.intl, { weekday: "long", day: "numeric", month: "long", timeZone: "UTC" }).format(new Date(`${d}T00:00:00Z`));
  const time = (ms: number) => new Intl.DateTimeFormat(f.intl, { hour: "2-digit", minute: "2-digit", timeZone: "UTC" }).format(new Date(ms));
  const canLeverage = actor.can("groups.edit") && mfaOk;

  return (
    <div data-testid="page-calendar" className="space-y-4">
      <PageHeader title={t("econ.title")}>
        {editable && (
          <>
            <Button variant="outline" onClick={() => importWeek.mutate(undefined)} disabled={importWeek.isPending} data-testid="econ-import">{t("econ.import")}</Button>
            <Button onClick={() => setEditing("new")} data-testid="econ-add">+ {t("econ.add")}</Button>
          </>
        )}
      </PageHeader>
      <p className="text-sm text-muted-foreground">{t("econ.intro")}</p>
      <div className="flex flex-wrap items-end gap-3">
        <Label>{t("common.from")}<Input type="date" value={from} onChange={(e) => e.target.value && setFrom(e.target.value)} className="w-40" /></Label>
        <Label>{t("common.to")}<Input type="date" value={to} onChange={(e) => e.target.value && setTo(e.target.value)} className="w-40" /></Label>
        <Label>{t("econ.currency")}
          <Select value={ccy} onChange={(e) => setCcy(e.target.value)} data-testid="econ-ccy">
            <option value="">{t("econ.allCurrencies")}</option>
            {currencies.map((c) => <option key={c} value={c}>{c}</option>)}
          </Select>
        </Label>
      </div>
      <Card className="overflow-x-auto">
        <table className="w-full text-sm tabular-nums" data-testid="econ-table">
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            <tr>{[t("econ.timeUtc"), t("econ.currency"), t("econ.impact"), t("econ.event"), t("econ.actual"), t("econ.forecast"), t("econ.previous"), ""].map((h, i) => <th key={i} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr>
          </thead>
          <tbody>
            {q.isLoading && <tr><td colSpan={8} className="px-3 py-4 text-center text-muted-foreground">{t("common.loading")}</td></tr>}
            {!q.isLoading && shown.length === 0 && <tr><td colSpan={8} className="px-3 py-4 text-center text-muted-foreground">{t("econ.none")}</td></tr>}
            {[...days.entries()].map(([d, list]) => (
              <React.Fragment key={d}>
                <tr className="border-t border-border bg-muted/30"><td colSpan={8} className="px-3 py-1.5 text-xs font-semibold">{dayLabel(d)}</td></tr>
                {list.map((e) => (
                  <tr key={e.id} className="border-t border-border" data-testid={`econ-row-${e.id}`}>
                    <td className="px-3 py-1">{time(e.time)}</td>
                    <td className="px-3 font-medium">{e.currency}</td>
                    <td className="px-3"><Badge tone={IMPACT_TONE[e.impact]}>{t(`econ.impact.${e.impact}`)}</Badge></td>
                    <td className="px-3">{e.title}</td>
                    <td className="px-3 font-medium">{e.actual ?? "—"}</td>
                    <td className="px-3">{e.forecast ?? "—"}</td>
                    <td className="px-3 text-muted-foreground">{e.previous ?? "—"}</td>
                    <td className="px-3 py-1 text-right whitespace-nowrap">
                      {canLeverage && e.impact === "high" && <Button size="sm" variant="ghost" onClick={() => setLeverageFor({ ev: e, offered: false })} data-testid={`econ-leverage-${e.id}`}>{t("econ.leverage")}</Button>}
                      {editable && <Button size="sm" variant="ghost" onClick={() => setEditing(e)}>{t("common.edit")}</Button>}
                      {editable && <Button size="sm" variant="ghost" onClick={() => remove.mutate(e.id)} disabled={remove.isPending} data-testid={`econ-delete-${e.id}`}>{t("econ.delete")}</Button>}
                    </td>
                  </tr>
                ))}
              </React.Fragment>
            ))}
          </tbody>
        </table>
      </Card>
      {editing && (
        <EventDialog
          key={editing === "new" ? "new" : editing.id}
          event={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={(ev, created) => {
            setEditing(null);
            // a new high-impact release: offer a leverage window around it
            if (created && ev.impact === "high" && canLeverage) setLeverageFor({ ev, offered: true });
          }}
        />
      )}
      {leverageFor && <LeverageDialog ev={leverageFor.ev} offered={leverageFor.offered} onClose={() => setLeverageFor(null)} />}
    </div>
  );
}

function EventDialog({ event, onClose, onSaved }: { event: EconEvent | null; onClose: () => void; onSaved: (e: EconEvent, created: boolean) => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const [draft, setDraft] = React.useState<EconEventInput>(() =>
    event
      ? { time: event.time, currency: event.currency, title: event.title, impact: event.impact, actual: event.actual ?? "", forecast: event.forecast ?? "", previous: event.previous ?? "" }
      : { time: Math.ceil(Date.now() / 3_600_000) * 3_600_000, currency: "USD", title: "", impact: "high", actual: "", forecast: "", previous: "" },
  );
  const [error, setError] = React.useState<string>();
  const save = useApiMutation(
    (e: EconEventInput) => (event ? api().updateEconEvent(event.id, e, actor) : api().createEconEvent(e, actor)),
    (r) => {
      toast(t("common.saved"));
      onSaved(r, !event);
    },
  );
  const set = <K extends keyof EconEventInput>(k: K, v: EconEventInput[K]) => setDraft({ ...draft, [k]: v });
  const submit = () => {
    if (!/^[A-Za-z]{3}$/.test(draft.currency.trim())) return setError(t("econ.badCurrency"));
    if (!draft.title.trim()) return setError(t("econ.badTitle"));
    if (!Number.isFinite(draft.time)) return setError(t("econ.badTime"));
    setError(undefined);
    save.mutate({ ...draft, currency: draft.currency.trim().toUpperCase(), title: draft.title.trim() });
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title={event ? t("econ.edit") : t("econ.add")}
      footer={
        <>
          <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
          <Button onClick={submit} disabled={save.isPending} data-testid="econ-save">{t("common.save")}</Button>
        </>
      }
    >
      <div className="grid gap-3 sm:grid-cols-2" data-testid="econ-dialog">
        <Label>{t("econ.timeUtc")}
          <Input type="datetime-local" value={Number.isFinite(draft.time) ? toInputUtc(draft.time) : ""} onChange={(e) => set("time", fromInputUtc(e.target.value))} data-testid="econ-time" />
        </Label>
        <TextField label={t("econ.currency")} value={draft.currency} onChange={(v) => set("currency", v.toUpperCase())} />
        <div className="sm:col-span-2"><TextField label={t("econ.event")} value={draft.title} onChange={(v) => set("title", v)} /></div>
        <SelectField label={t("econ.impact")} value={draft.impact} options={IMPACTS} onChange={(v) => set("impact", v)} />
        <TextField label={t("econ.actual")} value={draft.actual ?? ""} onChange={(v) => set("actual", v)} />
        <TextField label={t("econ.forecast")} value={draft.forecast ?? ""} onChange={(v) => set("forecast", v)} />
        <TextField label={t("econ.previous")} value={draft.previous ?? ""} onChange={(v) => set("previous", v)} />
      </div>
      <FieldError msg={error} />
    </Dialog>
  );
}

/** Caps a group's leverage from N minutes before to M minutes after the release (group leverage windows). */
function LeverageDialog({ ev, offered, onClose }: { ev: EconEvent; offered: boolean; onClose: () => void }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const groups = useApiQuery("listGroups");
  const [groupId, setGroupId] = React.useState("");
  const [leverage, setLeverage] = React.useState(20);
  const [before, setBefore] = React.useState(15);
  const [after, setAfter] = React.useState(15);
  const group: Group | undefined = groups.data?.find((g) => g.id === (groupId || groups.data?.[0]?.id));
  const valid = !!group && leverage >= 1 && leverage <= 1000 && before >= 0 && after >= 0 && before + after > 0;
  const save = useApiMutation(
    (g: Group) => api().saveGroup({ ...g, leverageWindows: [...(g.leverageWindows ?? []), { fromMs: ev.time - before * 60_000, toMs: ev.time + after * 60_000, leverage: Math.round(leverage) }] }, actor),
    (g) => {
      toast(t("econ.leverageAdded", { group: g.name }));
      onClose();
    },
  );
  return (
    <Dialog
      open
      onClose={onClose}
      title={t("econ.leverage")}
      footer={
        <>
          <Button variant="outline" onClick={onClose}>{offered ? t("econ.skip") : t("common.cancel")}</Button>
          <Button onClick={() => group && save.mutate(group)} disabled={!valid || save.isPending} data-testid="econ-leverage-save">{t("econ.leverageAdd")}</Button>
        </>
      }
    >
      <div className="grid gap-3 sm:grid-cols-2" data-testid="econ-leverage-dialog">
        <p className="text-sm sm:col-span-2">
          {offered && <span className="block font-medium">{t("econ.leverageOffer")}</span>}
          <span className="text-muted-foreground">{ev.currency} · {ev.title} · {f.date(ev.at)} UTC</span>
        </p>
        <Label>{t("econ.leverageGroup")}
          <Select value={group?.id ?? ""} onChange={(e) => setGroupId(e.target.value)}>
            {(groups.data ?? []).map((g) => <option key={g.id} value={g.id}>{g.name} (1:{g.leverage})</option>)}
          </Select>
        </Label>
        <NumField label="1:N" value={leverage} onChange={setLeverage} step={1} />
        <NumField label={t("econ.minutesBefore")} value={before} onChange={(v) => setBefore(Math.max(0, Math.round(v)))} step={5} />
        <NumField label={t("econ.minutesAfter")} value={after} onChange={(v) => setAfter(Math.max(0, Math.round(v)))} step={5} />
      </div>
    </Dialog>
  );
}
