"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Dialog, Input, PageHeader, Select, Tabs } from "@/components/ui/primitives";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useToast } from "@/components/shell/providers";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { MessageKey } from "@/lib/i18n";
import type { AccountActivity, ActivityEvent, BehaviorFlag, IpActivity, PlatformUser } from "@/lib/api";

type Tab = "users" | "alerts" | "ips";

const FLAG_TONE: Record<BehaviorFlag, "warning" | "danger"> = { scalper: "warning", burst: "warning", churn: "warning", brute_force: "danger", ip_hopping: "warning", flood: "danger" };

function Flags({ flags }: { flags: BehaviorFlag[] }) {
  const t = useT();
  if (flags.length === 0) return <span className="text-muted-foreground">—</span>;
  return <span className="flex flex-wrap gap-1">{flags.map((f) => <Badge key={f} tone={FLAG_TONE[f]}>{t(`platform.flag.${f}` as MessageKey)}</Badge>)}</span>;
}

function hold(s: number | null) {
  if (s == null) return "—";
  if (s < 90) return `${s} s`;
  if (s < 5400) return `${Math.round(s / 60)} min`;
  return `${(s / 3600).toFixed(1)} h`;
}

function EventList({ events }: { events: ActivityEvent[] }) {
  const t = useT();
  const f = useFormat();
  if (events.length === 0) return <div className="text-xs text-muted-foreground">{t("platform.noEvents")}</div>;
  return (
    <table className="w-full text-xs">
      <thead><tr>{[t("platform.when"), t("platform.kind"), t("platform.platformCol"), t("platform.ip"), t("platform.detail")].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
      <tbody>
        {events.map((e, i) => (
          <tr key={i} className="border-t border-border/60">
            <td className="whitespace-nowrap px-2 py-1 tabular-nums">{f.date(new Date(e.tsMs).toISOString())}</td>
            <td className="px-2 py-1"><Badge tone={e.kind === "connect" ? "success" : e.kind === "disconnect" ? "muted" : "danger"}>{t(`platform.event.${e.kind}` as MessageKey)}</Badge></td>
            <td className="px-2 py-1">{e.platform}</td>
            <td className="px-2 py-1 font-mono">{e.ip ?? "—"}</td>
            <td className="px-2 py-1 text-muted-foreground">{[e.names.join(" "), e.detail].filter(Boolean).join(" · ")}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function AccountDetail({ row, hours }: { row: AccountActivity; hours: number }) {
  const t = useT();
  const q = useApiQuery("activityEvents", [{ hours, login: row.login, limit: 100 }]);
  return (
    <div className="grid gap-3 p-3 text-sm lg:grid-cols-[18rem_1fr]">
      <div className="space-y-1 text-xs">
        <div><span className="text-muted-foreground">{t("platform.platforms")}:</span> {row.platforms.join(", ") || "—"}</div>
        <div><span className="text-muted-foreground">{t("platform.ips")}:</span> <span className="font-mono">{row.ips.join(", ") || "—"}</span></div>
        <div><span className="text-muted-foreground">{t("platform.orders")}:</span> {row.orders} · {t("platform.cancels")} {row.cancels} · {t("platform.maxPerMin")} {row.maxPerMin}</div>
        <div><span className="text-muted-foreground">{t("platform.closes")}:</span> {row.closes} · {t("platform.medianHold")} {hold(row.medianHoldS)} · {t("platform.scalpPct")} {row.scalpPct}%</div>
        <div><span className="text-muted-foreground">{t("platform.connects")}:</span> {row.connects} / {row.disconnects} · {t("platform.authFails")} {row.authFails}</div>
        <div className="pt-1"><Flags flags={row.flags} /></div>
      </div>
      <div className="min-w-0 overflow-x-auto">
        <div className="mb-1 text-[10px] uppercase text-muted-foreground">{t("platform.events")}</div>
        {q.data ? <EventList events={q.data} /> : t("common.loading")}
      </div>
    </div>
  );
}

function UserDetail({ row }: { row: PlatformUser }) {
  const t = useT();
  const f = useFormat();
  const q = useApiQuery("getPlatformUser", [`${row.institution}:${row.login}`]);
  const fields: [MessageKey, React.ReactNode][] = [
    ["platform.user.login", row.login], ["platform.user.name", row.name], ["platform.user.email", row.email], ["platform.user.phone", row.phone],
    ["platform.user.country", row.country], ["platform.user.city", row.city], ["platform.user.address", row.address], ["platform.user.group", row.group],
    ["platform.user.server", row.server], ["platform.user.leverage", row.leverage ? `1:${row.leverage}` : ""], ["platform.user.balance", row.currency ? `${f.num(row.balance)} ${row.currency}` : f.num(row.balance)],
    ["platform.user.registeredAt", row.registeredAt], ["platform.user.lastLoginAt", row.lastLoginAt], ["platform.user.lastIp", row.lastIp], ["platform.user.status", row.status], ["platform.user.comment", row.comment],
  ];
  return (
    <div className="grid gap-3 p-3 text-sm lg:grid-cols-[22rem_1fr]">
      <div className="grid grid-cols-[9rem_1fr] gap-x-2 gap-y-0.5 text-xs">
        {fields.map(([k, v]) => <React.Fragment key={k}><span className="text-muted-foreground">{t(k)}</span><span className="break-all">{v === "" || v == null ? "—" : v}</span></React.Fragment>)}
        {Object.entries(row.extra ?? {}).map(([k, v]) => <React.Fragment key={k}><span className="text-muted-foreground">{k}</span><span className="break-all">{v}</span></React.Fragment>)}
        <span className="text-muted-foreground">{t("platform.user.updated")}</span><span>{row.updatedMs ? f.date(new Date(row.updatedMs).toISOString()) : "—"}</span>
      </div>
      <div className="min-w-0 overflow-x-auto">
        <div className="mb-1 text-[10px] uppercase text-muted-foreground">{t("platform.institution")}: {q.data?.institution ? `${q.data.institution.name} (${q.data.institution.id} · ${t("clients.login")} ${q.data.institution.account})` : row.institution}</div>
        {q.data ? <EventList events={q.data.events} /> : t("common.loading")}
      </div>
    </div>
  );
}

function ImportDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const [text, setText] = React.useState("");
  const [err, setErr] = React.useState<string | null>(null);
  const mut = useApiMutation((users: Partial<PlatformUser>[]) => api().upsertPlatformUsers(users, actor), (r) => { toast(t("platform.imported", { n: r.upserted })); setText(""); onClose(); });
  const submit = () => {
    try {
      const v: unknown = JSON.parse(text);
      const arr = Array.isArray(v) ? v : [v];
      if (arr.length === 0 || arr.length > 500) throw new Error("1-500");
      setErr(null);
      mut.mutate(arr as Partial<PlatformUser>[]);
    } catch (e) {
      setErr(String(e instanceof Error ? e.message : e));
    }
  };
  return (
    <Dialog open={open} onClose={onClose} title={t("platform.import")}>
      <p className="mb-2 text-xs text-muted-foreground">{t("platform.importHint")}</p>
      <textarea className="h-56 w-full rounded-md border border-border bg-background p-2 font-mono text-xs" value={text} onChange={(e) => setText(e.target.value)} placeholder={'[{"institution":"demo-broker","login":500123,"name":"…","group":"real\\\\pro","lastIp":"1.2.3.4"}]'} data-testid="platform-import-text" />
      {err && <div className="mt-1 text-xs text-destructive">{err}</div>}
      <div className="mt-3 flex justify-end gap-2"><Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button><Button onClick={submit} disabled={mut.isPending || !text.trim()} data-testid="platform-import-submit">{t("platform.importSubmit")}</Button></div>
    </Dialog>
  );
}

/** Query string from `?tab=&login=` (static export: the server snapshot is empty, the client reads the URL). */
const noop = () => () => {};
function useQueryParams() {
  const search = React.useSyncExternalStore(noop, () => window.location.search, () => "");
  return React.useMemo(() => new URLSearchParams(search), [search]);
}

export default function PlatformPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const mfaOk = useMfaOk();
  const params = useQueryParams();
  const [tab, setTab] = React.useState<Tab>((params.get("tab") as Tab) || "users");
  const [hours, setHours] = React.useState(24);
  const [search, setSearch] = React.useState(params.get("login") ?? "");
  const [importing, setImporting] = React.useState(false);
  const users = useApiQuery("listPlatformUsers", [{}], { live: 10000 });
  const act = useApiQuery("activityAccounts", [{ hours }], { live: 15000, enabled: tab !== "users" });
  const inst = useApiQuery("listInstitutions", [], { live: 30000 });
  const [institution, setInstitution] = React.useState("");

  const uc = createColumnHelper<PlatformUser>();
  const userCols = React.useMemo(() => [
    uc.accessor("login", { header: t("platform.user.login"), cell: (c) => <span className="font-medium tabular-nums">{c.getValue()}</span> }),
    uc.accessor("name", { header: t("platform.user.name") }),
    uc.accessor("group", { header: t("platform.user.group"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span> }),
    uc.accessor("institution", { header: t("platform.institution"), cell: (c) => <Badge tone="info">{c.getValue()}</Badge> }),
    uc.accessor("server", { header: t("platform.user.server"), cell: (c) => c.getValue() || "—" }),
    uc.accessor("country", { header: t("platform.user.country"), cell: (c) => c.getValue() || "—" }),
    uc.accessor("email", { header: t("platform.user.email"), cell: (c) => <span className="text-xs">{c.getValue() || "—"}</span> }),
    uc.accessor("phone", { header: t("platform.user.phone"), cell: (c) => <span className="text-xs">{c.getValue() || "—"}</span>, meta: { defaultHidden: true } }),
    uc.accessor("city", { header: t("platform.user.city"), cell: (c) => c.getValue() || "—", meta: { defaultHidden: true } }),
    uc.accessor("address", { header: t("platform.user.address"), cell: (c) => <span className="text-xs">{c.getValue() || "—"}</span>, meta: { defaultHidden: true } }),
    uc.accessor("leverage", { header: t("platform.user.leverage"), cell: (c) => (c.getValue() ? `1:${c.getValue()}` : "—"), meta: { defaultHidden: true } }),
    uc.accessor("balance", { header: t("platform.user.balance"), cell: (c) => <span className="tabular-nums">{f.num(c.getValue())} {c.row.original.currency}</span> }),
    uc.accessor("lastLoginAt", { header: t("platform.user.lastLoginAt"), cell: (c) => <span className="whitespace-nowrap text-xs">{c.getValue() ? f.date(c.getValue()) : "—"}</span> }),
    uc.accessor("lastIp", { header: t("platform.user.lastIp"), cell: (c) => <span className="font-mono text-xs">{c.getValue() || "—"}</span> }),
    uc.accessor("registeredAt", { header: t("platform.user.registeredAt"), cell: (c) => <span className="whitespace-nowrap text-xs">{c.getValue() ? f.date(c.getValue()) : "—"}</span>, meta: { defaultHidden: true } }),
    uc.accessor("status", { header: t("common.status"), cell: (c) => (c.getValue() ? <Badge tone={c.getValue() === "active" ? "success" : "muted"}>{c.getValue()}</Badge> : "—") }),
  ], [t, f, uc]);

  const ac = createColumnHelper<AccountActivity>();
  const actCols = React.useMemo(() => [
    ac.accessor("login", { header: t("clients.login"), cell: (c) => <span className="font-medium tabular-nums">{c.getValue()}</span> }),
    ac.accessor("name", { header: t("clients.name"), cell: (c) => c.getValue() || "—" }),
    ac.accessor("group", { header: t("clients.group"), cell: (c) => <span className="text-xs">{c.getValue()}</span> }),
    ac.accessor((r) => r.platforms.join(", "), { id: "platforms", header: t("platform.platforms"), cell: (c) => <span className="text-xs">{c.getValue() || "—"}</span> }),
    ac.accessor((r) => r.ips.length, { id: "ips", header: t("platform.ips"), cell: (c) => <span className="tabular-nums" title={c.row.original.ips.join(", ")}>{c.getValue()}</span> }),
    ac.accessor("orders", { header: t("platform.orders"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ac.accessor("maxPerMin", { header: t("platform.maxPerMin"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ac.accessor("cancels", { header: t("platform.cancels"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span>, meta: { defaultHidden: true } }),
    ac.accessor("closes", { header: t("platform.closes"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ac.accessor("medianHoldS", { header: t("platform.medianHold"), cell: (c) => <span className="tabular-nums">{hold(c.getValue())}</span> }),
    ac.accessor("scalpPct", { header: t("platform.scalpPct"), cell: (c) => <span className="tabular-nums">{c.getValue()}%</span> }),
    ac.accessor("connects", { header: t("platform.connects"), cell: (c) => <span className="tabular-nums">{c.getValue()} / {c.row.original.disconnects}</span> }),
    ac.accessor("authFails", { header: t("platform.authFails"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ac.accessor((r) => r.flags.join(" "), { id: "flags", header: t("platform.flags"), cell: (c) => <Flags flags={c.row.original.flags} /> }),
    ac.accessor("score", { header: t("platform.score"), cell: (c) => <Badge tone={c.getValue() >= 50 ? "danger" : c.getValue() > 0 ? "warning" : "muted"}>{c.getValue()}</Badge> }),
  ], [t, ac]);

  const ic = createColumnHelper<IpActivity>();
  const ipCols = React.useMemo(() => [
    ic.accessor("ip", { header: t("platform.ip"), cell: (c) => <span className="font-mono">{c.getValue()}</span> }),
    ic.accessor("authFails", { header: t("platform.authFails"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ic.accessor("keyFails", { header: t("platform.keyFails"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ic.accessor("connRejects", { header: t("platform.connRejects"), cell: (c) => <span className="tabular-nums">{c.getValue()}</span> }),
    ic.accessor((r) => r.accounts.join(", "), { id: "accounts", header: t("platform.accounts"), cell: (c) => <span className="tabular-nums text-xs">{c.getValue() || "—"}</span> }),
    ic.accessor((r) => r.flags.join(" "), { id: "flags", header: t("platform.flags"), cell: (c) => <Flags flags={c.row.original.flags} /> }),
  ], [t, ic]);

  const userRows = React.useMemo(() => (users.data ?? []).filter((u) => (!institution || u.institution === institution) && (!search || String(u.login).includes(search) || u.name.toLowerCase().includes(search.toLowerCase()) || u.email.toLowerCase().includes(search.toLowerCase()) || u.lastIp.includes(search))), [users.data, institution, search]);
  const actRows = React.useMemo(() => (act.data?.accounts ?? []).filter((a) => !search || String(a.login).includes(search) || a.name.toLowerCase().includes(search.toLowerCase()) || a.ips.some((ip) => ip.includes(search))), [act.data, search]);
  const flagged = actRows.filter((a) => a.flags.length > 0).length;

  return (
    <div data-testid="page-platform">
      <PageHeader title={t("platform.title")}>
        {tab === "users" && actor.can("clients.edit") && <Button onClick={() => setImporting(true)} disabled={!mfaOk} data-testid="platform-import">{t("platform.import")}</Button>}
      </PageHeader>
      <p className="mb-3 text-xs text-muted-foreground">{t("platform.intro")}</p>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <Tabs value={tab} onChange={setTab} items={[{ value: "users", label: `${t("platform.users")} (${users.data?.length ?? 0})` }, { value: "alerts", label: <>{t("platform.alerts")}{flagged > 0 && <Badge tone="warning" className="ml-1">{flagged}</Badge>}</> }, { value: "ips", label: t("platform.ipsTab") }]} />
        <Input className="w-56" value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("platform.search")} aria-label={t("platform.search")} data-testid="platform-search" />
        {tab === "users" && (
          <Select value={institution} onChange={(e) => setInstitution(e.target.value)} aria-label={t("platform.institution")}>
            <option value="">{t("platform.allInstitutions")}</option>
            {(inst.data?.institutions ?? []).map((i) => <option key={i.id} value={i.id}>{i.id}</option>)}
          </Select>
        )}
        {tab !== "users" && (
          <Select value={String(hours)} onChange={(e) => setHours(Number(e.target.value))} aria-label={t("platform.window")}>
            {[1, 6, 24, 72, 168].map((h) => <option key={h} value={h}>{t("platform.lastHours", { n: h })}</option>)}
          </Select>
        )}
      </div>
      {tab === "users" && <DataTable data={userRows} columns={userCols} getRowId={(u) => `${u.institution}:${u.login}`} renderDetail={(u) => <UserDetail row={u} />} storageKey="platform-users" testId="platform-users" />}
      {tab === "alerts" && (
        <Card>
          <CardHeader><CardTitle>{t("platform.alertsTitle")}</CardTitle></CardHeader>
          <CardContent>
            <p className="mb-2 text-xs text-muted-foreground">{t("platform.alertsHint")}</p>
            {(act.data?.herd ?? []).length > 0 && (
              <div className="mb-3 flex flex-wrap items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-xs" data-testid="herd-signals">
                <span className="font-medium">{t("platform.herd")}:</span>
                {(act.data?.herd ?? []).map((h) => <Badge key={`${h.symbol}-${h.side}`} tone={h.accounts >= 5 ? "danger" : "warning"}>{h.symbol} {h.side === "Buy" ? "BUY" : "SELL"} · {h.accounts} {t("platform.accounts").toLowerCase()} · {f.num(h.lots)} lot</Badge>)}
              </div>
            )}
            <DataTable data={actRows} columns={actCols} getRowId={(a) => String(a.login)} renderDetail={(a) => <AccountDetail row={a} hours={hours} />} storageKey="platform-alerts" testId="platform-alerts" />
          </CardContent>
        </Card>
      )}
      {tab === "ips" && <DataTable data={act.data?.ips ?? []} columns={ipCols} getRowId={(r) => r.ip} storageKey="platform-ips" testId="platform-ips" />}
      <ImportDialog open={importing} onClose={() => setImporting(false)} />
    </div>
  );
}
