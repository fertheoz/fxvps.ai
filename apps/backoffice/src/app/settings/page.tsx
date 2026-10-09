"use client";
import * as React from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label, PageHeader } from "@/components/ui/primitives";
import { NumField, SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { DEFAULT_BEHAVIOR, RULE_METRICS, type AlertRule, type AlertSettings, type AlertSeverity, type RuleEval, type RuleFiring, type RuleMetric, type SwapConfig, type Tenant } from "@/lib/api";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { Book, Settings } from "@/lib/schemas";
import { CURRENCY_MINOR_DIGITS } from "@/lib/money";
import { setPrefs, usePrefs } from "@/lib/prefs";
import { LOCALES, type Locale, type MessageKey } from "@/lib/i18n";

export default function SettingsPage() {
  const t = useT();
  const prefs = usePrefs();
  const { data } = useApiQuery("getSettings");
  return (
    <div data-testid="page-settings">
      <PageHeader title={t("settings.title")} />
      <div className="grid gap-4 lg:grid-cols-2">
        {data ? <SettingsForm initial={data} /> : <Card className="p-4">{t("common.loading")}</Card>}
        <SwapCard />
        <AlertsCard />
        <AlertRulesCard />
        <StatementMailCard />
        <CalendarCard />
        <TenantsCard />
        <Card>
          <CardHeader><CardTitle>{t("settings.appearance")}</CardTitle></CardHeader>
          <CardContent className="grid gap-3 sm:grid-cols-2">
            <SelectField label={t("common.language")} value={prefs.locale} options={Object.keys(LOCALES) as Locale[]} onChange={(v) => setPrefs({ locale: v })} />
            <SelectField label={t("common.theme")} value={prefs.theme} options={["dark", "light"] as const} onChange={(v) => setPrefs({ theme: v })} />
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function SettingsForm({ initial }: { initial: Settings }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("settings.edit") && mfaOk;
  const { draft, set, errors, validate } = useZodForm(Settings, initial);
  const mut = useApiMutation((s: Settings) => api().saveSettings(s, actor), () => toast(t("common.saved")));
  return (
    <Card>
      <CardHeader><CardTitle>{t("settings.title")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 sm:grid-cols-2">
        <TextField label={t("settings.brokerName")} value={draft.brokerName} onChange={(v) => set("brokerName", v)} error={errors.brokerName} disabled={!editable} />
        <SelectField label={t("settings.baseCurrency")} value={draft.baseCurrency} options={Object.keys(CURRENCY_MINOR_DIGITS)} onChange={(v) => set("baseCurrency", v)} disabled={!editable} />
        <NumField
          label={`${t("settings.fourEyes")} (${draft.baseCurrency})`}
          value={draft.fourEyesThreshold / 10 ** (CURRENCY_MINOR_DIGITS[draft.baseCurrency] ?? 2)}
          onChange={(v) => set("fourEyesThreshold", Math.round(v * 10 ** (CURRENCY_MINOR_DIGITS[draft.baseCurrency] ?? 2)))}
          error={errors.fourEyesThreshold}
          step={1}
          disabled={!editable}
        />
        <NumField label={t("settings.sessionTimeout")} value={draft.sessionTimeoutMin} onChange={(v) => set("sessionTimeoutMin", v)} error={errors.sessionTimeoutMin} step={1} disabled={!editable} />
        <SelectField label={t("settings.requireMfa")} value={draft.requireMfa ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("requireMfa", v === "yes")} disabled={!editable} />
        <SelectField label={t("settings.defaultBook")} value={draft.defaultBook} options={Book.options} onChange={(v) => set("defaultBook", v)} disabled={!editable} />
        <TextField label={t("settings.brokerLei")} value={draft.brokerLei ?? ""} onChange={(v) => set("brokerLei", v.toUpperCase())} error={errors.brokerLei} disabled={!editable} />
        <h3 className="mt-2 text-sm font-semibold sm:col-span-2">{t("settings.funding")}</h3>
        <TextField label={t("settings.usdt")} value={draft.funding?.usdtTrc20Address ?? ""} onChange={(v) => set("funding", { ...(draft.funding ?? { usdtTrc20Address: "", bankDetails: "", minDepositMinor: 0, minWithdrawMinor: 0 }), usdtTrc20Address: v.trim() })} disabled={!editable} />
        <label className="grid gap-1 text-sm">{t("settings.bank")}
          <textarea className="min-h-20 rounded-md border border-border bg-background p-2 text-sm text-foreground" value={draft.funding?.bankDetails ?? ""} onChange={(e) => set("funding", { ...(draft.funding ?? { usdtTrc20Address: "", bankDetails: "", minDepositMinor: 0, minWithdrawMinor: 0 }), bankDetails: e.target.value })} disabled={!editable} />
        </label>
        <NumField label={t("settings.minDeposit")} value={draft.funding?.minDepositMinor ?? 0} onChange={(v) => set("funding", { ...(draft.funding ?? { usdtTrc20Address: "", bankDetails: "", minDepositMinor: 0, minWithdrawMinor: 0 }), minDepositMinor: Math.max(0, Math.round(v)) })} step={100} disabled={!editable} />
        <NumField label={t("settings.minWithdraw")} value={draft.funding?.minWithdrawMinor ?? 0} onChange={(v) => set("funding", { ...(draft.funding ?? { usdtTrc20Address: "", bankDetails: "", minDepositMinor: 0, minWithdrawMinor: 0 }), minWithdrawMinor: Math.max(0, Math.round(v)) })} step={100} disabled={!editable} />
        {editable && <div className="sm:col-span-2"><Button onClick={() => { const v = validate(); if (v) mut.mutate(v); }} disabled={mut.isPending}>{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

/** Stage 8: rollover schedule + manual run. */
function SwapCard() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const q = useApiQuery("getSwapConfig", [], { live: 15000 });
  const editable = actor.can("settings.edit") && mfaOk;
  const [draft, setDraft] = React.useState<SwapConfig | null>(null);
  const cfg = draft ?? q.data;
  const save = useApiMutation((c: SwapConfig) => api().saveSwapConfig(c, actor), () => { toast(t("common.saved")); setDraft(null); });
  const run = useApiMutation(() => api().runRollover(actor), (r) => toast(r.applied ? t("settings.rolloverDone", { n: r.positions }) : t("settings.rolloverSkipped", { reason: r.reason })));
  if (!cfg) return <Card className="p-4">{t("common.loading")}</Card>;
  const set = <K extends keyof SwapConfig>(k: K, v: SwapConfig[K]) => setDraft({ ...cfg, [k]: v });
  return (
    <Card data-testid="swap-config">
      <CardHeader><CardTitle>{t("settings.swap")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 sm:grid-cols-2">
        <p className="text-xs text-muted-foreground sm:col-span-2">{t("settings.swapHint")}</p>
        <SelectField label={t("settings.swapEnabled")} value={cfg.enabled ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("enabled", v === "yes")} disabled={!editable} />
        <NumField label={t("settings.rolloverHour")} value={cfg.rolloverHourUtc} onChange={(v) => set("rolloverHourUtc", Math.min(23, Math.max(0, Math.round(v))))} step={1} disabled={!editable} />
        <SelectField label={t("settings.skipWeekend")} value={cfg.skipWeekend ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("skipWeekend", v === "yes")} disabled={!editable} />
        <div className="text-sm"><div className="text-xs text-muted-foreground">{t("settings.lastRollover")}</div>{cfg.lastRolloverAt ? f.date(cfg.lastRolloverAt) : "-"}</div>
        {editable && (
          <div className="flex flex-wrap gap-2 sm:col-span-2">
            <Button onClick={() => save.mutate(cfg)} disabled={save.isPending || !draft} data-testid="swap-save">{t("common.save")}</Button>
            {actor.can("risk.edit") && <Button variant="outline" onClick={() => run.mutate(undefined)} disabled={run.isPending} data-testid="swap-run">{t("settings.runRollover")}</Button>}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/** Stage 13: alert thresholds and channels. */
function AlertsCard() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const q = useApiQuery("getAlertSettings");
  const editable = actor.can("settings.edit") && mfaOk;
  const [draft, setDraft] = React.useState<AlertSettings | null>(null);
  const cfg = draft ?? q.data;
  const save = useApiMutation((s: AlertSettings) => api().saveAlertSettings(s, actor), () => { toast(t("common.saved")); setDraft(null); });
  if (!cfg) return <Card className="p-4">{t("common.loading")}</Card>;
  const set = <K extends keyof AlertSettings>(k: K, v: AlertSettings[K]) => setDraft({ ...cfg, [k]: v });
  const quietFrom = cfg.quietHoursUtc ? cfg.quietHoursUtc[0] : -1;
  return (
    <Card data-testid="alert-settings">
      <CardHeader><CardTitle>{t("settings.alerts")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 sm:grid-cols-2">
        <p className="text-xs text-muted-foreground sm:col-span-2">{t("settings.alertsHint")}</p>
        <NumField label={t("settings.lpDownGrace")} value={cfg.lpDownGraceS} onChange={(v) => set("lpDownGraceS", Math.max(0, Math.round(v)))} step={10} disabled={!editable} />
        <NumField label={t("settings.lpSlow")} value={cfg.lpSlowMs ?? 0} onChange={(v) => set("lpSlowMs", Math.min(60000, Math.max(0, Math.round(v))))} step={100} disabled={!editable} />
        <p className="text-xs font-medium sm:col-span-2">{t("settings.behavior")}</p>
        {(["windowH", "scalperHoldS", "scalperMinCloses", "scalperPct", "burstPerMin", "churnConnects", "authFails", "ipCount"] as const).map((k) => (
          <NumField key={k} label={t(`settings.behavior.${k}` as MessageKey)} value={(cfg.behavior ?? DEFAULT_BEHAVIOR)[k]} onChange={(v) => set("behavior", { ...(cfg.behavior ?? DEFAULT_BEHAVIOR), [k]: Math.max(k === "windowH" ? 1 : 0, Math.min(k === "windowH" ? 168 : k === "scalperPct" ? 100 : 1_000_000, Math.round(v))) })} step={1} disabled={!editable} />
        ))}
        <NumField label={t("settings.fillRateMinOrders")} value={cfg.fillRateMinOrders} onChange={(v) => set("fillRateMinOrders", Math.max(1, Math.round(v)))} step={1} disabled={!editable} />
        <NumField label={t("settings.fillRateFloor")} value={cfg.fillRateFloorPct} onChange={(v) => set("fillRateFloorPct", Math.min(100, Math.max(0, Math.round(v))))} step={1} disabled={!editable} />
        <NumField label={t("settings.latencyFloor")} value={cfg.latencyFloorMs} onChange={(v) => set("latencyFloorMs", Math.max(0, Math.round(v)))} step={50} disabled={!editable} />
        <NumField label={t("settings.latencyMult")} value={cfg.latencyMultiplier} onChange={(v) => set("latencyMultiplier", Math.max(1, Math.round(v)))} step={1} disabled={!editable} />
        <TextField label={t("settings.webhook")} value={cfg.webhookUrl} onChange={(v) => set("webhookUrl", v.trim())} disabled={!editable} />
        <label className="grid gap-1 text-sm">{t("settings.telegramToken")}
          <input type="password" autoComplete="new-password" className="rounded-md border border-border bg-background px-2 py-1 text-sm text-foreground" value={cfg.telegramToken} placeholder={cfg.telegramTokenSet ? t("settings.telegramTokenSet") : ""} onChange={(e) => set("telegramToken", e.target.value.trim())} disabled={!editable} />
        </label>
        <TextField label={t("settings.telegramChat")} value={cfg.telegramChatId} onChange={(v) => set("telegramChatId", v.trim())} disabled={!editable} />
        <NumField label={t("settings.quietFrom")} value={quietFrom} onChange={(v) => set("quietHoursUtc", v < 0 ? null : [Math.min(23, Math.round(v)), cfg.quietHoursUtc?.[1] ?? 6])} step={1} disabled={!editable} />
        <NumField label={t("settings.quietTo")} value={cfg.quietHoursUtc ? cfg.quietHoursUtc[1] : 6} onChange={(v) => set("quietHoursUtc", cfg.quietHoursUtc ? [cfg.quietHoursUtc[0], Math.min(24, Math.max(0, Math.round(v)))] : null)} step={1} disabled={!editable || !cfg.quietHoursUtc} />
        <NumField label={t("settings.dailyReport")} value={cfg.dailyReportHourUtc ?? -1} onChange={(v) => set("dailyReportHourUtc", v < 0 ? null : Math.min(23, Math.round(v)))} step={1} disabled={!editable} />
        {editable && <div className="sm:col-span-2"><Button onClick={() => save.mutate(cfg)} disabled={save.isPending || !draft} data-testid="alerts-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

/** Plan item 8: monthly statement e-mail (env switch) + a test send to oneself. */
/** Parça 10b: dealer-defined alert rules (metric / target / threshold / severity) with a live reading and preview. */
function AlertRulesCard() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("settings.edit") && mfaOk;
  const q = useApiQuery("alertRules", [], { live: 15000 });
  const [draft, setDraft] = React.useState<AlertRule[] | null>(null);
  const [preview, setPreview] = React.useState<{ evals: RuleEval[]; firing: RuleFiring[] } | null>(null);
  const rules = draft ?? q.data?.rules ?? [];
  const evals = new Map((preview?.evals ?? q.data?.evals ?? []).map((e) => [e.id, e]));
  const save = useApiMutation((r: AlertRule[]) => api().saveAlertRules(r, actor), () => { toast(t("common.saved")); setDraft(null); setPreview(null); });
  const prev = useApiMutation((r: AlertRule[]) => api().previewAlertRules(r, actor), setPreview);
  const upd = (i: number, patch: Partial<AlertRule>) => setDraft(rules.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const add = () => setDraft([...rules, { id: `rule-${rules.length + 1}`, enabled: true, metric: "var_total_usd", target: "", op: "gt", threshold: 0, severity: "warning", title: "" }]);
  if (!q.data) return <Card className="p-4">{t("common.loading")}</Card>;
  return (
    <Card data-testid="alert-rules">
      <CardHeader><CardTitle>{t("settings.rules")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("settings.rulesHint")}</p>
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead><tr>{[t("settings.rule.id"), t("settings.rule.metric"), t("settings.rule.target"), t("settings.rule.op"), t("settings.rule.threshold"), t("settings.rule.severity"), t("settings.rule.title"), t("settings.rule.now"), ""].map((h, i) => <th key={i} className="whitespace-nowrap px-1 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
            <tbody>
              {rules.length === 0 && <tr><td colSpan={9} className="px-1 py-3 text-center text-muted-foreground">{t("settings.rulesNone")}</td></tr>}
              {rules.map((r, i) => {
                const ev = evals.get(r.id);
                return (
                  <tr key={i} className="border-t border-border/60">
                    <td className="px-1 py-1"><span className="flex items-center gap-1"><input type="checkbox" checked={r.enabled} onChange={(e) => upd(i, { enabled: e.target.checked })} disabled={!editable} aria-label={t("settings.rule.id")} /><Input className="w-24 font-mono" value={r.id} onChange={(e) => upd(i, { id: e.target.value })} disabled={!editable} /></span></td>
                    <td className="px-1 py-1"><select className="rounded-md border border-border bg-background p-1" value={r.metric} onChange={(e) => upd(i, { metric: e.target.value as RuleMetric })} disabled={!editable}>{RULE_METRICS.map((m) => <option key={m} value={m}>{t(`settings.metric.${m}` as MessageKey)}</option>)}</select></td>
                    <td className="px-1 py-1"><Input className="w-20 font-mono uppercase" value={r.target} placeholder="*" onChange={(e) => upd(i, { target: e.target.value.toUpperCase() })} disabled={!editable} /></td>
                    <td className="px-1 py-1"><select className="rounded-md border border-border bg-background p-1" value={r.op} onChange={(e) => upd(i, { op: e.target.value as "gt" | "lt" })} disabled={!editable}><option value="gt">&gt;</option><option value="lt">&lt;</option></select></td>
                    <td className="px-1 py-1"><Input type="number" className="w-24" value={r.threshold} onChange={(e) => upd(i, { threshold: Number(e.target.value) })} disabled={!editable} /></td>
                    <td className="px-1 py-1"><select className="rounded-md border border-border bg-background p-1" value={r.severity} onChange={(e) => upd(i, { severity: e.target.value as AlertSeverity })} disabled={!editable}>{(["info", "warning", "critical"] as const).map((s) => <option key={s} value={s}>{s}</option>)}</select></td>
                    <td className="px-1 py-1"><Input className="w-40" value={r.title} onChange={(e) => upd(i, { title: e.target.value })} disabled={!editable} /></td>
                    <td className="whitespace-nowrap px-1 py-1 tabular-nums">{ev ? <>{ev.value == null ? "—" : ev.value.toFixed(2)} {ev.fired && <Badge tone={r.severity === "critical" ? "danger" : "warning"}>{t("settings.rule.fires")}</Badge>}</> : "—"}</td>
                    <td className="px-1 py-1">{editable && <Button size="sm" variant="ghost" onClick={() => setDraft(rules.filter((_, j) => j !== i))} aria-label="remove">×</Button>}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {preview && <div className="text-xs text-muted-foreground">{t("settings.rulesPreview", { n: preview.firing.length })}{preview.firing.map((f) => <span key={f.id} className="ml-2"><Badge tone={f.severity === "critical" ? "danger" : "warning"}>{f.id}</Badge> {f.detail}</span>)}</div>}
        {editable && <div className="flex gap-2"><Button variant="outline" onClick={add} data-testid="rule-add">{t("settings.ruleAdd")}</Button><Button variant="outline" onClick={() => prev.mutate(rules)} disabled={prev.isPending || rules.length === 0} data-testid="rules-preview">{t("risk.preview")}</Button><Button onClick={() => save.mutate(rules)} disabled={save.isPending || draft == null} data-testid="rules-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

function StatementMailCard() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const q = useApiQuery("getStatementMail", [], { live: 60000 });
  const [account, setAccount] = React.useState("");
  const send = useApiMutation(
    () => api().sendTestStatement(account.trim() ? Number(account) : null, actor),
    (r) => toast(t("settings.statementTestSent", { month: r.month, account: r.account, to: r.to })),
  );
  const s = q.data;
  if (!s) return <Card className="p-4">{t("common.loading")}</Card>;
  const canSend = actor.can("settings.edit") && actor.can("clients.view") && mfaOk;
  return (
    <Card data-testid="statement-mail">
      <CardHeader><CardTitle>{t("settings.statementMail")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 text-sm sm:grid-cols-2">
        <p className="text-xs text-muted-foreground sm:col-span-2">{t("settings.statementMailHint", { days: s.runDays })}</p>
        <div><div className="text-xs text-muted-foreground">{t("settings.statementMailState")}</div>
          <Badge tone={s.enabled ? "success" : "muted"} data-testid="statement-mail-flag">{s.enabled ? t("settings.statementMailOn") : t("settings.statementMailOff")}</Badge>
        </div>
        <div><div className="text-xs text-muted-foreground">{t("settings.statementMailChannel")}</div>{s.configured ? s.from : <span className="text-muted-foreground">{t("settings.statementMailNoChannel")}</span>}</div>
        <div><div className="text-xs text-muted-foreground">{t("settings.statementMailRecipients")}</div>{s.recipients}</div>
        <div><div className="text-xs text-muted-foreground">{t("settings.statementMailRun", { month: s.month })}</div>
          {s.run ? t("settings.statementMailRunState", { sent: s.run.sent, failed: s.run.failed, passes: s.run.passes, done: s.run.done ? t("settings.statementMailDone") : "" }) : t("settings.statementMailNoRun")}
        </div>
        {canSend && (
          <div className="flex flex-wrap items-end gap-2 sm:col-span-2">
            <div className="space-y-1"><Label>{t("settings.statementTestAccount")}</Label><Input value={account} onChange={(e) => setAccount(e.target.value.replace(/\D/g, ""))} inputMode="numeric" className="w-40" /></div>
            <Button variant="outline" onClick={() => send.mutate(undefined)} disabled={send.isPending || !s.configured} data-testid="statement-test">{t("settings.statementTestSend")}</Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

/** Stage 13: holiday calendar. */
function CalendarCard() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const q = useApiQuery("getCalendar");
  const editable = actor.can("settings.edit") && mfaOk;
  const [text, setText] = React.useState<string | null>(null);
  const save = useApiMutation((holidays: string[]) => api().saveCalendar({ holidays }, actor), () => { toast(t("common.saved")); setText(null); });
  if (!q.data) return <Card className="p-4">{t("common.loading")}</Card>;
  const value = text ?? q.data.holidays.join("\n");
  return (
    <Card data-testid="calendar-settings">
      <CardHeader><CardTitle>{t("settings.calendar")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("settings.calendarHint")}</p>
        <textarea className="min-h-28 rounded-md border border-border bg-background p-2 font-mono text-xs text-foreground" value={value} onChange={(e) => setText(e.target.value)} disabled={!editable} placeholder="2026-12-25" />
        {editable && <div><Button onClick={() => save.mutate(value.split(/\s+/).map((x) => x.trim()).filter(Boolean))} disabled={save.isPending || text === null} data-testid="calendar-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

/** Stage 14: tenant registry (id | name | groups | hostnames, one per line). */
function TenantsCard() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const q = useApiQuery("listTenants");
  const editable = actor.can("users.edit") && mfaOk;
  const [text, setText] = React.useState<string | null>(null);
  const save = useApiMutation((ts: Tenant[]) => api().saveTenants(ts, actor), () => { toast(t("tenants.saved")); setText(null); });
  if (!q.data) return <Card className="p-4">{t("common.loading")}</Card>;
  const value = text ?? q.data.map((x) => [x.id, x.name, x.groups.join(", "), x.hostnames.join(", "), x.brandColor ?? "", x.logoUrl ?? "", x.supportEmail ?? ""].join(" | ").replace(/( \| )+$/, "")).join("\n");
  const parse = (v: string): Tenant[] => v.split("\n").map((l) => l.trim()).filter(Boolean).map((l) => {
    const [id = "", name = "", groups = "", hosts = "", brandColor = "", logoUrl = "", supportEmail = ""] = l.split("|").map((p) => p.trim());
    const list = (x: string) => x.split(",").map((p) => p.trim()).filter(Boolean);
    return { id, name, groups: list(groups), hostnames: list(hosts), brandColor, logoUrl, supportEmail };
  });
  return (
    <Card data-testid="tenants-settings">
      <CardHeader><CardTitle>{t("tenants.title")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("tenants.hint")}</p>
        <textarea className="min-h-24 rounded-md border border-border bg-background p-2 font-mono text-xs text-foreground" value={value} onChange={(e) => setText(e.target.value)} disabled={!editable} placeholder="fxvps | fxvps.ai | demo-retail, demo-hedge | trade.fxvps.ai" />
        {editable && <div><Button onClick={() => save.mutate(parse(value))} disabled={save.isPending || text === null} data-testid="tenants-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}
