"use client";
import * as React from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Label, PageHeader, Select } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { HEDGE_MODES, NEWS_ACTIONS, type HedgeMode, type HedgePolicy, type HedgePreview, type MarginCallRow, type NewsAction } from "@/lib/api";
import { NumField, SelectField, TextField } from "@/components/form";
import { useMfaOk } from "@/lib/queries";
import type { MessageKey } from "@/lib/i18n";

export default function RiskPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const exp = useApiQuery("exposure", [], { live: 5000 });
  const ccy = useApiQuery("currencyExposure", [], { live: 5000 });
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
              <thead className="text-xs text-muted-foreground"><tr><th className="text-left">{t("positions.symbol")}</th><th className="text-right">Net lots</th><th className="text-right">A</th><th className="text-right">B</th><th className="text-right">LP</th><th className="text-right">{t("risk.hedgeCol")}</th><th className="text-right">{t("risk.limitCol")}</th><th className="text-right">Notional</th><th className="text-right">σ 1d</th><th className="text-right">VaR</th></tr></thead>
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
                    <td className="text-right text-muted-foreground">{e.volDailyPct !== undefined && e.volDailyPct > 0 ? `${e.volDailyPct.toFixed(2)}%` : "—"}</td>
                    <td className="text-right">{e.varUsd ? Math.round(e.varUsd).toLocaleString() : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </CardContent>
        </Card>
        <Card data-testid="currency-exposure">
          <CardHeader><CardTitle>{t("risk.currencyExposure")}</CardTitle></CardHeader>
          <CardContent>
            <table className="w-full text-sm">
              <thead className="text-xs text-muted-foreground"><tr><th className="text-left">{t("risk.currency")}</th><th className="text-right">A</th><th className="text-right">B</th><th className="text-right">Net</th><th className="text-right">B USD</th><th className="text-right">{t("risk.limitCol")}</th></tr></thead>
              <tbody>
                {(ccy.data ?? []).map((r) => (
                  <tr key={r.currency} className="border-t border-border tabular-nums">
                    <td className="py-1 font-medium">{r.currency}</td>
                    <td className="text-right">{Math.round(r.aAmount).toLocaleString()}</td>
                    <td className="text-right">{Math.round(r.bAmount).toLocaleString()}</td>
                    <td className="text-right">{Math.round(r.netAmount).toLocaleString()}</td>
                    <td className="text-right">{r.bUsd === null ? "—" : Math.round(r.bUsd).toLocaleString()}</td>
                    <td className={`text-right ${r.overLimit ? "text-red-600 dark:text-red-400 font-semibold" : ""}`}>{r.limitUsd ?? "—"}</td>
                  </tr>
                ))}
                {(ccy.data ?? []).length === 0 && <tr><td colSpan={6} className="py-2 text-center text-muted-foreground">{t("common.noResults")}</td></tr>}
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
        <ManualHedgeCard symbols={(exp.data ?? []).map((r) => r.symbol)} />
        <TempMarkupCard groups={(groups.data ?? []).map((g) => g.name)} />
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
  const [ccyText, setCcyText] = React.useState(Object.entries(initial.currencyLimitsUsd ?? {}).map(([k, v]) => `${k} ${v}`).join("\n"));
  const mut = useApiMutation((v: HedgePolicy) => api().saveHedgePolicy(v, actor), () => toast(t("risk.hedgeSaved")));
  const [preview, setPreview] = React.useState<HedgePreview | null>(null);
  const prev = useApiMutation((v: HedgePolicy) => api().previewHedgePolicy(v, actor), setPreview);
  const set = <K extends keyof HedgePolicy>(k: K, v: HedgePolicy[K]) => setP({ ...p, [k]: v });
  const assemble = (): HedgePolicy => {
    const symbolLimits: Record<string, number> = {};
    for (const line of limitsText.split("\n")) {
      const [sym, lots] = line.trim().split(/[\s,=]+/);
      const n = Number(lots);
      if (sym && n > 0) symbolLimits[sym.toUpperCase()] = n;
    }
    const currencyLimitsUsd: Record<string, number> = {};
    for (const line of ccyText.split("\n")) {
      const [c, usd] = line.trim().split(/[\s,=]+/);
      const n = Number(usd);
      if (c && n > 0) currencyLimitsUsd[c.toUpperCase()] = Math.round(n);
    }
    return { ...p, symbolLimits, currencyLimitsUsd };
  };
  const save = () => mut.mutate(assemble());
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
          <NumField label={t("risk.sliceLots")} value={p.sliceLots ?? 0} onChange={(v) => set("sliceLots", v > 0 ? v : null)} step={0.1} disabled={!editable || p.mode !== "hedge_excess"} />
          <NumField label={t("risk.sliceInterval")} value={p.sliceIntervalS ?? 0} onChange={(v) => set("sliceIntervalS", Math.max(0, Math.round(v)))} step={10} disabled={!editable || p.mode !== "hedge_excess"} />
          <NumField label={`${t("risk.varLimit")}${p.varTotalUsd !== undefined ? ` (${t("risk.varNow")} ${Math.round(p.varTotalUsd)})` : ""}`} value={p.varLimitUsd ?? 0} onChange={(v) => set("varLimitUsd", v > 0 ? Math.round(v) : null)} step={1000} disabled={!editable || p.mode !== "hedge_excess"} />
        </div>
        <Label>
          {t("risk.symbolLimits")}
          <textarea className="min-h-16 rounded-md border border-border bg-background p-2 font-mono text-xs" value={limitsText} onChange={(e) => setLimitsText(e.target.value)} disabled={!editable} placeholder="XAUUSD 5" />
        </Label>
        <Label>
          {t("risk.currencyLimits")}
          <textarea className="min-h-16 rounded-md border border-border bg-background p-2 font-mono text-xs" value={ccyText} onChange={(e) => setCcyText(e.target.value)} disabled={!editable} placeholder="EUR 500000" data-testid="currency-limits" />
        </Label>
        <div className="grid gap-3 sm:grid-cols-3">
          <NumField label={t("risk.burstWindow")} value={p.burstWindowMin ?? 0} onChange={(v) => set("burstWindowMin", Math.min(1440, Math.max(0, Math.round(v))))} step={5} disabled={!editable} />
          <NumField label={t("risk.burstAccount")} value={p.burstAccountLots ?? 0} onChange={(v) => set("burstAccountLots", v > 0 ? v : null)} step={0.5} disabled={!editable} />
          <NumField label={t("risk.burstSymbol")} value={p.burstSymbolLots ?? 0} onChange={(v) => set("burstSymbolLots", v > 0 ? v : null)} step={0.5} disabled={!editable} />
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <NumField label={t("risk.newsWindow")} value={p.newsWindowMin ?? 0} onChange={(v) => set("newsWindowMin", Math.min(1440, Math.max(0, Math.round(v))))} step={5} disabled={!editable} />
          <Label>
            {t("risk.newsAction")}{p.inNewsWindow ? <span className="ml-2 text-xs text-amber-600 dark:text-amber-400">{t("risk.newsNow")}</span> : null}
            <select className="rounded-md border border-border bg-background p-2 text-sm" value={p.newsAction ?? "none"} onChange={(e) => set("newsAction", e.target.value as NewsAction)} disabled={!editable} data-testid="news-action">
              {NEWS_ACTIONS.map((a) => <option key={a} value={a}>{t(`risk.newsAction.${a}` as MessageKey)}</option>)}
            </select>
          </Label>
        </div>
        {preview && <HedgePreviewTable p={preview} />}
        {editable && <div className="flex gap-2"><Button variant="outline" onClick={() => prev.mutate(assemble())} disabled={prev.isPending} data-testid="hedge-preview">{t("risk.preview")}</Button><Button onClick={save} disabled={mut.isPending} data-testid="hedge-save">{t("common.save")}</Button></div>}
      </CardContent>
    </Card>
  );
}

/** Parça 10b: what the policy in the form would do right now, before saving. */
function HedgePreviewTable({ p }: { p: HedgePreview }) {
  const t = useT();
  const f = useFormat();
  const moves = p.symbols.filter((r) => r.firstOrder);
  return (
    <div className="rounded-md border border-border p-2 text-xs" data-testid="hedge-preview-table">
      <div className="mb-1 font-medium">{t("risk.previewTitle")}</div>
      <div className="mb-2 text-muted-foreground">
        {t("risk.varNow")} {f.num(p.varTotalUsd, 0)} USD{p.varLimitUsd != null && <> / {f.num(p.varLimitUsd, 0)} {p.varOver && <Badge tone="danger">{t("risk.over")}</Badge>}</>}
        {p.currency.filter((c) => c.over).map((c) => <Badge key={c.currency} tone="danger" className="ml-1">{c.currency} {f.num(c.usd, 0)} &gt; {f.num(c.limitUsd ?? 0, 0)}</Badge>)}
      </div>
      {p.symbols.length === 0 && <div className="text-muted-foreground">{t("risk.previewNone")}</div>}
      {p.symbols.length > 0 && (
        <table className="w-full">
          <thead><tr>{[t("risk.symbol"), t("risk.bBookNet"), t("risk.hedgeNow"), t("risk.hedgeTarget"), t("risk.delta"), t("risk.firstOrder"), "VaR"].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
          <tbody>
            {p.symbols.map((r) => (
              <tr key={r.symbol} className="border-t border-border/60 tabular-nums">
                <td className="px-2 py-1 font-medium">{r.symbol}</td>
                <td className="px-2 py-1">{f.num(r.bBookNetLots)}</td>
                <td className="px-2 py-1">{f.num(r.hedgeLots)}</td>
                <td className="px-2 py-1">{f.num(r.targetLots)}</td>
                <td className="px-2 py-1">{r.deltaLots ? <Badge tone={Math.abs(r.deltaLots) >= 1 ? "warning" : "muted"}>{r.deltaLots > 0 ? "+" : ""}{f.num(r.deltaLots)}</Badge> : "—"}</td>
                <td className="px-2 py-1">{r.firstOrder ? `${r.firstOrder.side.toUpperCase()} ${f.num(r.firstOrder.lots)}` : "—"}</td>
                <td className="px-2 py-1">{r.varUsd == null ? "—" : f.num(r.varUsd, 0)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-1 text-muted-foreground">{t("risk.previewMoves", { n: moves.length })}</div>
    </div>
  );
}

/** Parça 12: temporary markup through the real-time pricing API (MFA). */
function TempMarkupCard({ groups }: { groups: string[] }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("groups.edit") && mfaOk;
  const q = useApiQuery("tempMarkups", [], { live: 10000 });
  const [group, setGroup] = React.useState("");
  const [symbol, setSymbol] = React.useState("");
  const [points, setPoints] = React.useState(10);
  const [ttl, setTtl] = React.useState(30);
  const [reason, setReason] = React.useState("");
  const set = useApiMutation((v: Parameters<ReturnType<typeof api>["setTempMarkup"]>[0]) => api().setTempMarkup(v, actor), () => { toast(t("risk.markupSet")); setReason(""); });
  const clear = useApiMutation((id: string) => api().clearTempMarkup(id, actor), () => toast(t("risk.markupCleared")));
  return (
    <Card data-testid="temp-markup">
      <CardHeader><CardTitle>{t("risk.tempMarkup")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("risk.tempMarkupHint")} <code className="rounded bg-muted px-1">PUT /v1/pricing/markup</code></p>
        {(q.data ?? []).length > 0 && (
          <table className="w-full text-xs">
            <thead><tr>{[t("clients.group"), t("risk.symbol"), t("groups.markup"), t("risk.until"), t("risk.reason"), ""].map((h, i) => <th key={i} className="px-2 py-1 text-left font-medium text-muted-foreground">{h}</th>)}</tr></thead>
            <tbody>
              {(q.data ?? []).map((m) => (
                <tr key={m.id} className="border-t border-border/60">
                  <td className="px-2 py-1">{m.group || <span className="text-muted-foreground">{t("risk.allGroups")}</span>}</td>
                  <td className="px-2 py-1 font-mono">{m.symbol ?? <span className="text-muted-foreground">{t("risk.allSymbols")}</span>}</td>
                  <td className="px-2 py-1 tabular-nums"><Badge tone={m.points > 0 ? "warning" : "info"}>{m.points > 0 ? "+" : ""}{m.points}</Badge></td>
                  <td className="whitespace-nowrap px-2 py-1 tabular-nums">{f.date(m.until)}</td>
                  <td className="px-2 py-1 text-muted-foreground">{m.reason}</td>
                  <td className="px-2 py-1">{editable && <Button size="sm" variant="ghost" onClick={() => clear.mutate(m.id)} data-testid={`markup-clear-${m.id}`}>{t("risk.markupClear")}</Button>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <div className="grid gap-3 sm:grid-cols-5">
          <Label>
            {t("clients.group")}
            <select className="rounded-md border border-border bg-background p-2 text-sm" value={group} onChange={(e) => setGroup(e.target.value)} disabled={!editable} data-testid="markup-group">
              <option value="">{t("risk.allGroups")}</option>
              {groups.map((g) => <option key={g} value={g}>{g}</option>)}
            </select>
          </Label>
          <TextField label={`${t("risk.symbol")} (${t("risk.allSymbols")})`} value={symbol} onChange={(v) => setSymbol(v.toUpperCase())} disabled={!editable} />
          <NumField label={t("risk.markupPoints")} value={points} onChange={(v) => setPoints(Math.max(-1000, Math.min(1000, Math.round(v))))} step={1} disabled={!editable} />
          <NumField label={t("risk.ttlMin")} value={ttl} onChange={(v) => setTtl(Math.max(1, Math.min(1440, Math.round(v))))} step={5} disabled={!editable} />
          <TextField label={t("risk.reason")} value={reason} onChange={setReason} disabled={!editable} />
        </div>
        {editable && <div><Button onClick={() => set.mutate({ group, symbol: symbol.trim() || null, points, ttlS: ttl * 60, reason })} disabled={set.isPending || points === 0} data-testid="markup-set">{t("risk.markupApply")}</Button></div>}
      </CardContent>
    </Card>
  );
}

/** Parça 10b: dealer sends a broker hedge order to the LP by hand (MFA). */
function ManualHedgeCard({ symbols }: { symbols: string[] }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const mfaOk = useMfaOk();
  const editable = actor.can("risk.edit") && mfaOk;
  const [symbol, setSymbol] = React.useState("");
  const [side, setSide] = React.useState<"buy" | "sell">("sell");
  const [lots, setLots] = React.useState(0.1);
  const [confirm, setConfirm] = React.useState(false);
  const mut = useApiMutation((v: { symbol: string; side: "buy" | "sell"; lots: number }) => api().manualHedge(v, actor), () => { toast(t("risk.manualSent")); setConfirm(false); });
  const sym = symbol || symbols[0] || "";
  return (
    <Card data-testid="manual-hedge">
      <CardHeader><CardTitle>{t("risk.manual")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("risk.manualHint")}</p>
        <div className="grid gap-3 sm:grid-cols-3">
          <Label>
            {t("risk.symbol")}
            <input className="rounded-md border border-border bg-background p-2 font-mono text-sm uppercase" list="manual-hedge-symbols" value={sym} onChange={(e) => setSymbol(e.target.value.toUpperCase())} disabled={!editable} data-testid="manual-hedge-symbol" />
            <datalist id="manual-hedge-symbols">{symbols.map((s) => <option key={s} value={s} />)}</datalist>
          </Label>
          <SelectField label={t("risk.side")} value={side} options={["buy", "sell"] as const} onChange={setSide} disabled={!editable} />
          <NumField label={t("positions.lots")} value={lots} onChange={(v) => setLots(Math.max(0.01, Math.round(v * 100) / 100))} step={0.1} disabled={!editable} />
        </div>
        {editable && !confirm && <div><Button variant="outline" onClick={() => setConfirm(true)} disabled={!sym || lots < 0.01} data-testid="manual-hedge-arm">{t("risk.manualArm")}</Button></div>}
        {editable && confirm && (
          <div className="flex flex-wrap items-center gap-2 rounded-md border border-amber-500/50 bg-amber-500/10 p-2 text-sm">
            <span>{t("risk.manualConfirm", { side: side.toUpperCase(), lots, symbol: sym })}</span>
            <Button onClick={() => mut.mutate({ symbol: sym, side, lots })} disabled={mut.isPending} data-testid="manual-hedge-send">{t("risk.manualSend")}</Button>
            <Button variant="ghost" onClick={() => setConfirm(false)}>{t("common.cancel")}</Button>
          </div>
        )}
        {!mfaOk && actor.can("risk.edit") && <p className="text-xs text-amber-600 dark:text-amber-400">{t("risk.manualMfa")}</p>}
      </CardContent>
    </Card>
  );
}
