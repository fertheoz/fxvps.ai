"use client";
import * as React from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Label, PageHeader, Select } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { HEDGE_MODES, type HedgeMode, type HedgePolicy, type MarginCallRow } from "@/lib/api";
import { NumField, SelectField } from "@/components/form";
import { useMfaOk } from "@/lib/queries";
import type { MessageKey } from "@/lib/i18n";

export default function RiskPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const exp = useApiQuery("exposure", [], { live: 5000 });
  const mc = useApiQuery("marginCalls", [], { live: 5000 });
  const presets = useApiQuery("esmaPresets");
  const groups = useApiQuery("listGroups");
  const [target, setTarget] = React.useState<Record<string, string>>({});
  const apply = useApiMutation((v: { preset: string; group: string }) => api().applyPreset(v.preset, v.group, actor), (g) => toast(`${g.name}: 1:${g.leverage}`));

  const stopOuts = (mc.data ?? []).filter((r) => r.state === "stop_out");
  const calls = (mc.data ?? []).filter((r) => r.state === "margin_call");

  const list = (rows: MarginCallRow[], testId: string) => (
    <div className="grid gap-1" data-testid={testId}>
      {rows.length === 0 && <p className="text-sm text-muted-foreground">{t("common.noResults")}</p>}
      {rows.map((r) => (
        <div key={r.client.id} className="flex items-center justify-between gap-2 border-b border-border py-1.5 text-sm last:border-0">
          <span><span className="font-medium tabular-nums">#{r.client.login}</span> <span className="text-muted-foreground">{r.client.name}</span></span>
          <span className="flex items-center gap-2 tabular-nums">
            <span className="text-xs text-muted-foreground">{f.money(r.client.equity)} / {f.money(r.client.margin)}</span>
            <Badge tone={r.state === "stop_out" ? "danger" : "warning"}>{f.num(r.marginLevel, 1)}%</Badge>
          </span>
        </div>
      ))}
    </div>
  );

  return (
    <div data-testid="page-risk">
      <PageHeader title={t("risk.title")} />
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader><CardTitle>{t("risk.topExposure")}</CardTitle></CardHeader>
          <CardContent>
            <table className="w-full text-sm">
              <thead className="text-xs text-muted-foreground"><tr><th className="text-left">{t("positions.symbol")}</th><th className="text-right">Net lots</th><th className="text-right">A</th><th className="text-right">B</th><th className="text-right">LP</th><th className="text-right">{t("risk.hedgeCol")}</th><th className="text-right">{t("risk.limitCol")}</th><th className="text-right">Notional</th></tr></thead>
              <tbody>
                {(exp.data ?? []).slice(0, 10).map((e) => (
                  <tr key={e.symbol} className="border-t border-border tabular-nums">
                    <td className="py-1 font-medium">{e.symbol}</td>
                    <td className="text-right">{e.netLots}</td>
                    <td className="text-right">{e.aBookLots}</td>
                    <td className="text-right">{e.bBookLots}</td>
                    <td className={`text-right ${Math.abs(e.lpLots - e.aBookLots) > 0.001 ? "text-amber-600 dark:text-amber-400 font-semibold" : ""}`}>{e.lpLots}</td>
                    <td className="text-right">{e.hedgeLots ? e.hedgeLots : "—"}{e.hedgePendingLots ? <span className="text-xs text-muted-foreground"> (+{e.hedgePendingLots})</span> : null}</td>
                    <td className={`text-right ${e.overLimit ? "text-red-600 dark:text-red-400 font-semibold" : ""}`}>{e.limitLots != null ? e.limitLots : "—"}</td>
                    <td className="text-right">{f.money(e.notional, "USD", { compact: true })}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("risk.stopOutQueue")} ({stopOuts.length})</CardTitle></CardHeader>
          <CardContent>{list(stopOuts, "stopout-queue")}</CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("risk.marginCalls")} ({calls.length})</CardTitle></CardHeader>
          <CardContent>{list(calls, "margin-calls")}</CardContent>
        </Card>
        <HedgeCard />
        <Card>
          <CardHeader><CardTitle>{t("risk.presets")}</CardTitle></CardHeader>
          <CardContent className="grid gap-3">
            {(presets.data ?? []).map((p) => (
              <div key={p.id} className="grid gap-2 rounded-md border border-border p-2 text-sm">
                <div className="font-medium">{p.label}</div>
                <div className="flex flex-wrap gap-1.5 text-xs">
                  <Badge>1:{p.maxLeverage}</Badge><Badge>MC {p.marginCallPct}%</Badge><Badge>SO {p.stopOutPct}%</Badge>
                  {p.negativeBalanceProtection && <Badge tone="success">{t("risk.nbp")}</Badge>}
                </div>
                {actor.can("risk.edit") && (
                  <div className="flex gap-2">
                    <Select value={target[p.id] ?? ""} onChange={(e) => setTarget({ ...target, [p.id]: e.target.value })} aria-label="group" className="flex-1">
                      <option value="">—</option>
                      {(groups.data ?? []).map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
                    </Select>
                    <Button size="sm" disabled={!target[p.id] || apply.isPending} onClick={() => apply.mutate({ preset: p.id, group: target[p.id]! })}>{t("risk.apply")}</Button>
                  </div>
                )}
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Stage 7: B-book exposure limits / auto-hedge
// ---------------------------------------------------------------------------

function HedgeCard() {
  const t = useT();
  const q = useApiQuery("hedgePolicy", [], { live: 10000 });
  if (q.isLoading) return <Card className="p-4">{t("common.loading")}</Card>;
  if (q.error || !q.data) return <Card className="p-4 text-sm text-muted-foreground">{t("common.noResults")}</Card>;
  return <HedgeForm key={JSON.stringify(q.data)} initial={q.data} />;
}

function HedgeForm({ initial }: { initial: HedgePolicy }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("risk.edit") && mfaOk;
  const [p, setP] = React.useState<HedgePolicy>(initial);
  const [limitsText, setLimitsText] = React.useState(Object.entries(initial.symbolLimits).map(([k, v]) => `${k} ${v}`).join("\n"));
  const mut = useApiMutation((v: HedgePolicy) => api().saveHedgePolicy(v, actor), () => toast(t("risk.hedgeSaved")));
  const set = <K extends keyof HedgePolicy>(k: K, v: HedgePolicy[K]) => setP({ ...p, [k]: v });
  const save = () => {
    const symbolLimits: Record<string, number> = {};
    for (const line of limitsText.split("\n")) {
      const [sym, lots] = line.trim().split(/[\s,=]+/);
      const n = Number(lots);
      if (sym && n > 0) symbolLimits[sym.toUpperCase()] = n;
    }
    mut.mutate({ ...p, symbolLimits });
  };
  return (
    <Card data-testid="hedge-policy">
      <CardHeader><CardTitle>{t("risk.hedge")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("risk.hedgeHint")}</p>
        <div className="grid gap-3 sm:grid-cols-2">
          <SelectField label={t("risk.hedgeEnabled")} value={p.enabled ? "on" : "off"} options={["on", "off"] as const} onChange={(v) => set("enabled", v === "on")} disabled={!editable} />
          <Label>
            {t("risk.hedgeMode")}
            <select className="rounded-md border border-border bg-background p-2 text-sm" value={p.mode} onChange={(e) => set("mode", e.target.value as HedgeMode)} disabled={!editable} data-testid="hedge-mode">
              {HEDGE_MODES.map((m) => <option key={m} value={m}>{t(`risk.hedgeMode.${m}` as MessageKey)}</option>)}
            </select>
          </Label>
          <NumField label={t("risk.symbolLimit")} value={p.defaultSymbolLimit ?? 0} onChange={(v) => set("defaultSymbolLimit", v > 0 ? v : null)} step={0.1} disabled={!editable} />
          <NumField label={t("risk.totalLimit")} value={p.totalLimit ?? 0} onChange={(v) => set("totalLimit", v > 0 ? v : null)} step={1} disabled={!editable} />
          <NumField label={t("risk.accountLimit")} value={p.accountLimit ?? 0} onChange={(v) => set("accountLimit", v > 0 ? v : null)} step={0.1} disabled={!editable || p.mode !== "switch_to_a_book"} />
          <NumField label={t("risk.hedgeRatio")} value={p.hedgeRatioPct} onChange={(v) => set("hedgeRatioPct", Math.min(100, Math.max(1, Math.round(v))))} step={5} disabled={!editable || p.mode !== "hedge_excess"} />
          <NumField label={t("risk.releasePct")} value={p.releasePct} onChange={(v) => set("releasePct", Math.min(100, Math.max(0, Math.round(v))))} step={5} disabled={!editable || p.mode !== "hedge_excess"} />
        </div>
        <Label>
          {t("risk.symbolLimits")}
          <textarea className="min-h-16 rounded-md border border-border bg-background p-2 font-mono text-xs" value={limitsText} onChange={(e) => setLimitsText(e.target.value)} disabled={!editable} placeholder="XAUUSD 5" />
        </Label>
        {editable && <div><Button onClick={save} disabled={mut.isPending} data-testid="hedge-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}
