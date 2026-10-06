"use client";
import * as React from "react";
import { Play, Plus, RefreshCw, Square, Trash2 } from "lucide-react";
import { Button, Card, CardContent, CardHeader, CardTitle, Input, Label, PageHeader } from "@/components/ui/primitives";
import { NumField, SelectField, TextField } from "@/components/form";
import { AGG_MODES, type AggMode, type LpAggregation, type LpConfig, type LpEndpoint, type LpPolicy } from "@/lib/api/types";
import { Badge } from "@/components/ui/primitives";
import { FixBadge } from "@/components/badges";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { MessageKey } from "@/lib/i18n";

export default function LpPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const { data = [] } = useApiQuery("listFixSessions", [], { live: 2000 });
  const rc = useApiMutation((id: string) => api().reconnect(id, actor), (s) => toast(`${s.lp} ${s.kind}: ${s.status}`));
  return (
    <div data-testid="page-lp">
      <PageHeader title={t("lp.title")} />
      <Card className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            <tr>{["LP", t("lp.session"), "SenderCompID → TargetCompID", t("common.status"), t("lp.inSeq"), t("lp.outSeq"), t("lp.latency"), t("lp.rejects"), t("lp.heartbeat"), ""].map((h, i) => <th key={i} className="whitespace-nowrap px-3 py-2 text-left font-medium">{h}</th>)}</tr>
          </thead>
          <tbody>
            {data.map((s) => (
              <tr key={s.id} className="border-t border-border tabular-nums">
                <td className="px-3 py-2 font-medium">{s.lp}</td>
                <td className="px-3">{s.kind}</td>
                <td className="px-3 font-mono text-xs">{s.senderCompId} → {s.targetCompId}</td>
                <td className="px-3">
                  <FixBadge status={s.status} />
                  {s.status !== "logged_on" && s.lastError && <div className="mt-1 max-w-xs text-xs text-red-600 dark:text-red-400" data-testid="lp-error">{s.lastError}</div>}
                </td>
                <td className="px-3">{s.inSeq.toLocaleString()}</td>
                <td className="px-3">{s.outSeq.toLocaleString()}</td>
                <td className="px-3">{s.status === "logged_on" ? `${f.num(s.latencyMs)} ms` : "—"}</td>
                <td className={`px-3 ${s.rejects24h > 5 ? "text-red-600 dark:text-red-400" : ""}`}>{s.rejects24h}</td>
                <td className="px-3 whitespace-nowrap">{f.date(s.lastHeartbeat)}</td>
                <td className="px-3">
                  {actor.can("lp.reconnect") && s.status !== "logged_on" && (
                    <Button size="sm" variant="outline" onClick={() => rc.mutate(s.id)} disabled={rc.isPending}><RefreshCw className="h-3 w-3" />{t("lp.reconnect")}</Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
      {actor.can("lp.view") && <AggregationCard />}
      {actor.can("reports.view") && <LpPerformanceCard />}
      {actor.can("lp.view") && <LpConfigCard />}
    </div>
  );
}

const emptyEndpoint = (): LpEndpoint => ({
  addr: "", sender_comp_id: "", target_comp_id: "", username: "", password: null, reset_on_logon: true, tls: {},
});

const DEFAULT_INSTRUMENTS = "EUR/USD 4001 0.00001 10000";

/** Blank LMAX-style config used until the first save. */
const blankConfig = (): LpConfig => ({
  lp: "LMAX", heartbeat_secs: 30, market_depth: 5, reconnect_delay_ms: 5000, security_id_source: "8", store_dir: null,
  md: emptyEndpoint(), trade: emptyEndpoint(), instruments: [], nats: null,
});

const instrumentsText = (c: LpConfig) => c.instruments.map((i) => `${i.symbol} ${i.security_id} ${i.tick_size} ${i.contract_size ?? 1}`).join("\n") || DEFAULT_INSTRUMENTS;

function parseInstruments(text: string): LpConfig["instruments"] {
  return text.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
    const [symbol = "", security_id = "", tick_size = "0.00001", size = "1"] = l.split(/\s+/);
    return { symbol, security_id, tick_size, contract_size: Math.max(1, Math.trunc(Number(size)) || 1) };
  });
}

function LpConfigCard() {
  const t = useT();
  const q = useApiQuery("getLpConfig");
  if (q.isLoading) return <Card className="mt-4 p-4">{t("common.loading")}</Card>;
  if (q.error) return <Card className="mt-4 p-4 text-sm text-muted-foreground">{t("lp.notManaged")}</Card>;
  return (
    <>
      {q.data && <LpRunControl config={q.data} />}
      <LpConfigForm key={JSON.stringify(q.data ?? null)} initial={q.data ?? blankConfig()} />
    </>
  );
}

/** Stop = no logon attempts at all (protects the LP account); Play = connect again. */
function LpRunControl({ config }: { config: LpConfig }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const running = config.enabled !== false;
  const mut = useApiMutation(
    (enabled: boolean) => api().saveLpConfig(withoutSecrets({ ...config, enabled }), actor),
    (c) => toast(c.enabled === false ? t("lp.stopped") : t("lp.started")),
  );
  const can = actor.can("lp.manage") && mfaOk;
  return (
    <Card className="mt-4 flex flex-wrap items-center gap-3 p-4" data-testid="lp-run">
      <span className="text-sm font-medium">{running ? t("lp.running") : t("lp.paused")}</span>
      <span className="text-xs text-muted-foreground">{t("lp.runHint")}</span>
      <div className="ml-auto flex gap-2">
        <Button variant="outline" onClick={() => mut.mutate(false)} disabled={!can || !running || mut.isPending} data-testid="lp-stop">
          <Square className="h-3 w-3" />{t("lp.stop")}
        </Button>
        <Button onClick={() => mut.mutate(true)} disabled={!can || running || mut.isPending} data-testid="lp-play">
          <Play className="h-3 w-3" />{t("lp.play")}
        </Button>
      </div>
    </Card>
  );
}

/** Never resend redacted fields; empty password = keep the stored one. */
function withoutSecrets(c: LpConfig): LpConfig {
  const strip = (e: LpEndpoint): LpEndpoint => {
    const rest = { ...e, password: e.password || null };
    delete rest.password_set;
    return rest;
  };
  return { ...c, md: strip(c.md), trade: strip(c.trade) };
}

function EndpointFields({ title, ep, onChange, disabled, extra }: { title: string; ep: LpEndpoint; onChange: (e: LpEndpoint) => void; disabled: boolean; extra?: React.ReactNode }) {
  const t = useT();
  const set = <K extends keyof LpEndpoint>(k: K, v: LpEndpoint[K]) => onChange({ ...ep, [k]: v });
  return (
    <fieldset className="grid gap-3 sm:grid-cols-2">
      <legend className="mb-1 flex items-center gap-2 text-sm font-medium">{title}{extra}</legend>
      <TextField label={t("lp.addr")} value={ep.addr} onChange={(v) => set("addr", v.trim())} disabled={disabled} />
      <TextField label={t("lp.username")} value={ep.username ?? ""} onChange={(v) => set("username", v || null)} disabled={disabled} />
      <TextField label="SenderCompID" value={ep.sender_comp_id} onChange={(v) => set("sender_comp_id", v.trim())} disabled={disabled} />
      <TextField label="TargetCompID" value={ep.target_comp_id} onChange={(v) => set("target_comp_id", v.trim())} disabled={disabled} />
      <Label>
        {t("lp.password")}
        <Input type="password" autoComplete="new-password" value={ep.password ?? ""} placeholder={ep.password_set ? t("lp.passwordSet") : ""} onChange={(e) => set("password", e.target.value || null)} disabled={disabled} />
      </Label>
      <SelectField label={t("lp.tls")} value={ep.tls ? "on" : "off"} options={["on", "off"] as const} onChange={(v) => set("tls", v === "on" ? {} : null)} disabled={disabled} />
      {ep.tls && <TextField label={t("lp.serverName")} value={ep.tls.server_name ?? ""} onChange={(v) => set("tls", { ...ep.tls, server_name: v.trim() || null })} disabled={disabled} />}
    </fieldset>
  );
}

function LpConfigForm({ initial }: { initial: LpConfig }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("lp.manage") && mfaOk;
  const [c, setC] = React.useState<LpConfig>(initial);
  const [instr, setInstr] = React.useState(instrumentsText(initial));
  const mut = useApiMutation((v: LpConfig) => api().saveLpConfig(v, actor), () => toast(t("lp.saved")));
  const save = () => mut.mutate({ ...withoutSecrets(c), instruments: parseInstruments(instr) });
  return (
    <Card className="mt-4" data-testid="lp-config">
      <CardHeader><CardTitle>{t("lp.config")}</CardTitle></CardHeader>
      <CardContent className="grid gap-5">
        <p className="text-xs text-muted-foreground">{t("lp.configHint")}</p>
        <EndpointFields title={t("lp.md")} ep={c.md} onChange={(md) => setC({ ...c, md })} disabled={!editable} />
        <EndpointFields
          title={t("lp.trade")}
          ep={c.trade}
          onChange={(trade) => setC({ ...c, trade })}
          disabled={!editable}
          extra={editable && (
            <Button size="sm" variant="outline" type="button" onClick={() => setC({ ...c, trade: { ...c.trade, username: c.md.username, tls: c.md.tls } })}>{t("lp.copyFromMd")}</Button>
          )}
        />
        <div className="grid gap-3 sm:grid-cols-3">
          <TextField label="LP" value={c.lp} onChange={(v) => setC({ ...c, lp: v })} disabled={!editable} />
          <NumField label={t("lp.heartbeatSecs")} value={c.heartbeat_secs} onChange={(v) => setC({ ...c, heartbeat_secs: v })} step={1} disabled={!editable} />
          <TextField label={t("lp.idSource")} value={c.security_id_source} onChange={(v) => setC({ ...c, security_id_source: v.trim() })} disabled={!editable} />
        </div>
        <Label>
          {t("lp.instruments")}
          <textarea className="min-h-24 rounded-md border border-border bg-background p-2 font-mono text-xs" value={instr} onChange={(e) => setInstr(e.target.value)} disabled={!editable} />
        </Label>
        {editable && <div><Button onClick={save} disabled={mut.isPending}>{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Multi-LP aggregation (stage 6)
// ---------------------------------------------------------------------------

type PolicyDraft = LpPolicy & { symbolsText: string };

const toDraft = (a: LpAggregation): PolicyDraft[] => a.lps.map((p) => ({
  name: p.name, enabled: p.enabled, priority: p.priority,
  minLots: p.minLots == null ? null : String(p.minLots), maxLots: p.maxLots == null ? null : String(p.maxLots),
  symbols: p.symbols, symbolsText: p.symbols.join(" "),
}));

function AggregationCard() {
  const t = useT();
  const q = useApiQuery("getLpAggregation", [], { live: 5000 });
  if (q.isLoading) return <Card className="mt-4 p-4">{t("common.loading")}</Card>;
  if (q.error || !q.data) return <Card className="mt-4 p-4 text-sm text-muted-foreground" data-testid="lp-agg-none">{t("lp.aggNone")}</Card>;
  return <AggregationForm key={JSON.stringify([q.data.mode, q.data.maxDeviationPoints, q.data.lps.map((p) => [p.name, p.enabled, p.priority, p.minLots, p.maxLots, p.symbols])])} data={q.data} />;
}

function AggregationForm({ data }: { data: LpAggregation }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("lp.manage") && mfaOk;
  const [mode, setMode] = React.useState<AggMode>(data.mode);
  const [dev, setDev] = React.useState(data.maxDeviationPoints);
  const [lps, setLps] = React.useState<PolicyDraft[]>(toDraft(data));
  const [newLp, setNewLp] = React.useState("");
  const mut = useApiMutation((v: Parameters<ReturnType<typeof api>["saveLpAggregation"]>[0]) => api().saveLpAggregation(v, actor), () => toast(t("lp.aggSaved")));
  const upd = (i: number, patch: Partial<PolicyDraft>) => setLps(lps.map((p, j) => (j === i ? { ...p, ...patch } : p)));
  const save = () => mut.mutate({
    mode, maxDeviationPoints: Math.max(0, Math.trunc(dev) || 0),
    lps: lps.map(({ symbolsText, ...p }) => ({ ...p, minLots: p.minLots?.trim() || null, maxLots: p.maxLots?.trim() || null, symbols: symbolsText.split(/[\s,]+/).map((x) => x.trim().toUpperCase()).filter(Boolean) })),
  });
  const runtime = (name: string) => data.lps.find((p) => p.name === name);
  return (
    <Card className="mt-4" data-testid="lp-agg">
      <CardHeader><CardTitle>{t("lp.agg")}</CardTitle></CardHeader>
      <CardContent className="grid gap-4">
        <p className="text-xs text-muted-foreground">{t("lp.aggHint")}</p>
        <div className="grid gap-3 sm:grid-cols-2">
          <Label>
            {t("lp.mode")}
            <select className="rounded-md border border-border bg-background p-2 text-sm" value={mode} onChange={(e) => setMode(e.target.value as AggMode)} disabled={!editable} data-testid="lp-agg-mode">
              {AGG_MODES.map((m) => <option key={m} value={m}>{t(`lp.mode.${m}` as MessageKey)}</option>)}
            </select>
          </Label>
          <NumField label={t("lp.deviation")} value={dev} onChange={setDev} step={1} disabled={!editable} />
        </div>
        <p className="-mt-2 text-xs text-muted-foreground">{t("lp.deviationHint")}</p>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead className="bg-muted/50 text-xs text-muted-foreground">
              <tr>{["LP", t("lp.enabled"), t("lp.priority"), t("lp.minLots"), t("lp.maxLots"), t("lp.symbols"), t("common.status"), t("lp.quoting"), t("lp.lastQuote"), ""].map((h, i) => <th key={i} className="whitespace-nowrap px-2 py-2 text-left font-medium">{h}</th>)}</tr>
            </thead>
            <tbody>
              {lps.map((p, i) => {
                const r = runtime(p.name);
                return (
                  <tr key={p.name} className="border-t border-border align-top" data-testid={`lp-agg-row-${p.name}`}>
                    <td className="px-2 py-2 font-medium">{p.name}</td>
                    <td className="px-2 py-2"><input type="checkbox" checked={p.enabled} onChange={(e) => upd(i, { enabled: e.target.checked })} disabled={!editable} aria-label={t("lp.enabled")} /></td>
                    <td className="px-2 py-1"><Input type="number" min={1} max={1000} className="w-16" value={p.priority} onChange={(e) => upd(i, { priority: Math.max(1, Math.trunc(Number(e.target.value)) || 1) })} disabled={!editable} /></td>
                    <td className="px-2 py-1"><Input className="w-20" value={p.minLots ?? ""} placeholder="—" onChange={(e) => upd(i, { minLots: e.target.value || null })} disabled={!editable} /></td>
                    <td className="px-2 py-1"><Input className="w-20" value={p.maxLots ?? ""} placeholder="—" onChange={(e) => upd(i, { maxLots: e.target.value || null })} disabled={!editable} /></td>
                    <td className="px-2 py-1"><Input className="w-40 font-mono text-xs" value={p.symbolsText} placeholder="EURUSD GBPUSD" onChange={(e) => upd(i, { symbolsText: e.target.value })} disabled={!editable} /></td>
                    <td className="px-2 py-2">
                      {r ? (
                        <div className="flex flex-wrap gap-1">
                          <Badge tone={r.mdUp ? "success" : "danger"}>MD</Badge>
                          <Badge tone={r.tradeUp ? "success" : "danger"}>TRD</Badge>
                          {r.deviating.length > 0 && <Badge tone="warning" title={r.deviating.join(", ")}>{t("lp.deviating")} {r.deviating.length}</Badge>}
                        </div>
                      ) : "—"}
                    </td>
                    <td className="px-2 py-2 tabular-nums">{r?.quoting ?? 0}</td>
                    <td className="px-2 py-2 whitespace-nowrap text-xs">{r?.lastQuoteAt ? f.date(r.lastQuoteAt) : "—"}</td>
                    <td className="px-2 py-1">{editable && <Button size="sm" variant="ghost" onClick={() => setLps(lps.filter((_, j) => j !== i))} aria-label="remove"><Trash2 className="h-3 w-3" /></Button>}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {editable && (
          <div className="flex flex-wrap items-end gap-2">
            <TextField label={t("lp.lpName")} value={newLp} onChange={setNewLp} />
            <Button variant="outline" disabled={!newLp.trim() || lps.some((p) => p.name === newLp.trim())} onClick={() => { setLps([...lps, { name: newLp.trim(), enabled: true, priority: lps.length + 1, minLots: null, maxLots: null, symbols: [], symbolsText: "" }]); setNewLp(""); }}>
              <Plus className="h-3 w-3" />{t("lp.addLp")}
            </Button>
            <Button className="ml-auto" onClick={save} disabled={mut.isPending} data-testid="lp-agg-save">{t("common.save")}</Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function LpPerformanceCard() {
  const t = useT();
  const f = useFormat();
  const q = useApiQuery("lpReport", [], { live: 10000 });
  const rows = q.data ?? [];
  const pct = (v: number) => `${(v * 100).toFixed(1)}%`;
  return (
    <Card className="mt-4" data-testid="lp-perf">
      <CardHeader><CardTitle>{t("lp.perf")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("lp.perfHint")}</p>
        {rows.length === 0 ? <p className="text-sm text-muted-foreground">{t("lp.noOrders")}</p> : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="bg-muted/50 text-xs text-muted-foreground">
                <tr>{["LP", t("lp.orders"), t("lp.fillRate"), t("lp.rejectRate"), t("lp.slip"), t("lp.latencyP"), t("lp.lastFill")].map((h, i) => <th key={i} className="whitespace-nowrap px-3 py-2 text-left font-medium">{h}</th>)}</tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.lp} className="border-t border-border tabular-nums">
                    <td className="px-3 py-2 font-medium">{r.lp}</td>
                    <td className="px-3 py-2">{r.orders} <span className="text-xs text-muted-foreground">({r.filled}/{r.partial}/{r.rejected}{r.working ? `/${r.working}` : ""})</span></td>
                    <td className={`px-3 py-2 ${r.fillRate < 0.9 && r.orders > 0 ? "text-amber-600 dark:text-amber-400" : ""}`}>{pct(r.fillRate)} <span className="text-xs text-muted-foreground">{f.num(r.filledLots)}/{f.num(r.requestedLots)} lot</span></td>
                    <td className={`px-3 py-2 ${r.rejectRate > 0.05 ? "text-red-600 dark:text-red-400" : ""}`}>{pct(r.rejectRate)}</td>
                    <td className="px-3 py-2">{r.avgSlipPoints.toFixed(2)} / {r.p95SlipPoints.toFixed(2)}</td>
                    <td className="px-3 py-2">{f.num(r.p50LatencyMs)} / {f.num(r.p95LatencyMs)} ms</td>
                    <td className="px-3 py-2 whitespace-nowrap text-xs">{r.lastFillAt ? f.date(r.lastFillAt) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
