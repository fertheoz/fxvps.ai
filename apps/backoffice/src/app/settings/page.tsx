"use client";
import * as React from "react";
import { Button, Card, CardContent, CardHeader, CardTitle, PageHeader } from "@/components/ui/primitives";
import { NumField, SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import type { SwapConfig } from "@/lib/api";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { Book, Settings } from "@/lib/schemas";
import { CURRENCY_MINOR_DIGITS } from "@/lib/money";
import { setPrefs, usePrefs } from "@/lib/prefs";
import { LOCALES, type Locale } from "@/lib/i18n";

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
        <NumField label={`${t("settings.fourEyes")} (minor units)`} value={draft.fourEyesThreshold} onChange={(v) => set("fourEyesThreshold", v)} error={errors.fourEyesThreshold} step={1} disabled={!editable} />
        <NumField label={t("settings.sessionTimeout")} value={draft.sessionTimeoutMin} onChange={(v) => set("sessionTimeoutMin", v)} error={errors.sessionTimeoutMin} step={1} disabled={!editable} />
        <SelectField label={t("settings.requireMfa")} value={draft.requireMfa ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("requireMfa", v === "yes")} disabled={!editable} />
        <SelectField label={t("settings.defaultBook")} value={draft.defaultBook} options={Book.options} onChange={(v) => set("defaultBook", v)} disabled={!editable} />
        <TextField label={t("settings.brokerLei")} value={draft.brokerLei ?? ""} onChange={(v) => set("brokerLei", v.toUpperCase())} error={errors.brokerLei} disabled={!editable} />
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
