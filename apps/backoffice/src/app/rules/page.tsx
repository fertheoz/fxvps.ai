"use client";
import * as React from "react";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Dialog, PageHeader } from "@/components/ui/primitives";
import { BookBadge } from "@/components/badges";
import { NumField, SelectField, TextField } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useT } from "@/lib/hooks";
import type { RoutingRule } from "@/lib/api";

const KINDS = ["any", "market", "pending"] as const;
const ROUTES = ["group", "ABook", "BBook", "split"] as const;
const PARTIALS = ["group", "cancel", "retry", "all_or_none"] as const;

const list = (v: string) => v.split(/[,\s]+/).map((x) => x.trim()).filter(Boolean);
const uid = () => Math.random().toString(36).slice(2, 8);

function blank(): RoutingRule {
  return { id: uid(), name: "", enabled: true, groups: [], accounts: [], symbols: [], minLots: null, maxLots: null, kind: "any", hoursUtc: null, routing: null, aBookPct: null, markupPoints: null, maxSlippagePoints: null, partialFill: null, minToxicity: null, maxToxicity: null };
}

/** Partial-fill override as a select value (+ attempts). */
function partialOf(r: RoutingRule): { kind: (typeof PARTIALS)[number]; attempts: number } {
  const p = r.partialFill;
  if (!p) return { kind: "group", attempts: 3 };
  if (p === "CancelRemainder") return { kind: "cancel", attempts: 3 };
  if (p === "AllOrNone") return { kind: "all_or_none", attempts: 3 };
  return { kind: "retry", attempts: p.Retry.max_attempts };
}

export default function RulesPage() {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const rules = useApiQuery("listRules", [], { live: 10000 });
  const dry = useApiQuery("rulesDryRun", [], { live: 10000 });
  const versions = useApiQuery("ruleVersions", [], { live: 15000 });
  const restore = useApiMutation((id: string) => api().restoreRuleVersion(id, actor), () => toast(t("rules.restored")));
  const [draft, setDraft] = React.useState<RoutingRule[] | null>(null);
  const [editing, setEditing] = React.useState<RoutingRule | null>(null);
  const current = draft ?? rules.data ?? [];
  const dirty = draft !== null;
  const save = useApiMutation((rs: RoutingRule[]) => api().saveRules(rs, actor), () => {
    toast(t("common.saved"));
    setDraft(null);
  });
  const canEdit = actor.can("groups.edit");
  const update = (rs: RoutingRule[]) => setDraft(rs);
  const move = (i: number, d: -1 | 1) => {
    const rs = [...current];
    const j = i + d;
    if (j < 0 || j >= rs.length) return;
    [rs[i], rs[j]] = [rs[j]!, rs[i]!];
    update(rs);
  };
  const describe = (r: RoutingRule) => {
    const parts: string[] = [];
    if (r.groups.length) parts.push(`${t("rules.groups")}: ${r.groups.join(", ")}`);
    if (r.accounts.length) parts.push(`${t("rules.accounts")}: ${r.accounts.join(", ")}`);
    if (r.symbols.length) parts.push(`${t("rules.symbols")}: ${r.symbols.join(", ")}`);
    if (r.minLots !== null || r.maxLots !== null) parts.push(`${r.minLots ?? 0}–${r.maxLots ?? "∞"} lot`);
    if (r.kind !== "any") parts.push(t(`rules.kind.${r.kind}`));
    if (r.hoursUtc) parts.push(`${r.hoursUtc[0]}:00–${r.hoursUtc[1]}:00 UTC`);
    if (r.minToxicity !== null || r.maxToxicity !== null) parts.push(`${t("flow.toxicity")} ${r.minToxicity ?? 0}–${r.maxToxicity ?? 100}`);
    return parts.length ? parts.join(" · ") : t("rules.matchAll");
  };
  const action = (r: RoutingRule) => {
    const parts: React.ReactNode[] = [];
    if (r.routing) parts.push(<BookBadge key="b" book={r.routing === "ABook" ? "A" : "B"} />);
    else if (r.aBookPct !== null) parts.push(<Badge key="s" tone="muted">A {r.aBookPct}% / B {100 - r.aBookPct}%</Badge>);
    if (r.markupPoints !== null) parts.push(<Badge key="m" tone="muted">markup {r.markupPoints}</Badge>);
    if (r.maxSlippagePoints !== null) parts.push(<Badge key="x" tone="muted">slip ≤ {r.maxSlippagePoints}</Badge>);
    const p = partialOf(r);
    if (p.kind !== "group") parts.push(<Badge key="p" tone="muted">{p.kind}{p.kind === "retry" ? ` ×${p.attempts}` : ""}</Badge>);
    return parts.length ? <span className="flex flex-wrap gap-1">{parts}</span> : <span className="text-muted-foreground">{t("rules.noAction")}</span>;
  };
  const hits = new Map((dry.data?.rules ?? []).map((x) => [x.id, x]));

  return (
    <div data-testid="page-rules">
      <PageHeader title={t("rules.title")}>
        {canEdit && <Button variant="outline" onClick={() => setEditing(blank())}><Plus className="h-4 w-4" />{t("rules.add")}</Button>}
        {canEdit && dirty && <Button onClick={() => save.mutate(current)} disabled={save.isPending} data-testid="rules-save">{t("rules.save")}</Button>}
        {canEdit && dirty && <Button variant="outline" onClick={() => setDraft(null)}>{t("common.cancel")}</Button>}
      </PageHeader>
      <p className="mb-3 text-sm text-muted-foreground">{t("rules.intro")}</p>
      <div className="grid gap-4 xl:grid-cols-3">
        <div className="grid gap-2 xl:col-span-2">
          {current.length === 0 && <Card><CardContent className="pt-4 text-sm text-muted-foreground">{t("rules.empty")}</CardContent></Card>}
          {current.map((r, i) => {
            const h = hits.get(r.id);
            return (
              <Card key={r.id} className={r.enabled ? "" : "opacity-60"} data-testid={`rule-${r.id}`}>
                <CardContent className="flex flex-wrap items-center gap-3 pt-4">
                  <span className="w-6 text-center text-xs text-muted-foreground tabular-nums">{i + 1}</span>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-medium">{r.name || t("rules.unnamed")}</span>
                      {!r.enabled && <Badge tone="muted">{t("rules.disabled")}</Badge>}
                      {h && <Badge tone={h.orders ? "default" : "muted"}>{h.orders} {t("rules.hits24h")}</Badge>}
                    </div>
                    <div className="text-xs text-muted-foreground">{describe(r)}</div>
                    <div className="mt-1">{action(r)}</div>
                  </div>
                  {canEdit && (
                    <div className="flex items-center gap-1">
                      <Button size="sm" variant="ghost" onClick={() => move(i, -1)} disabled={i === 0} aria-label="up"><ArrowUp className="h-4 w-4" /></Button>
                      <Button size="sm" variant="ghost" onClick={() => move(i, 1)} disabled={i === current.length - 1} aria-label="down"><ArrowDown className="h-4 w-4" /></Button>
                      <Button size="sm" variant="outline" onClick={() => update(current.map((x) => (x.id === r.id ? { ...x, enabled: !x.enabled } : x)))}>{r.enabled ? t("rules.disable") : t("rules.enable")}</Button>
                      <Button size="sm" variant="outline" onClick={() => setEditing(r)}>{t("common.edit")}</Button>
                      <Button size="sm" variant="ghost" onClick={() => update(current.filter((x) => x.id !== r.id))} aria-label="delete"><Trash2 className="h-4 w-4" /></Button>
                    </div>
                  )}
                </CardContent>
              </Card>
            );
          })}
        </div>
        <Card data-testid="rule-versions">
          <CardHeader><CardTitle>{t("rules.versions")}</CardTitle></CardHeader>
          <CardContent className="text-sm">
            <p className="mb-2 text-xs text-muted-foreground">{t("rules.versionsHint")}</p>
            {(versions.data ?? []).length === 0 && <p className="text-muted-foreground">—</p>}
            {(versions.data ?? []).slice(0, 10).map((v) => (
              <div key={v.id} className="flex items-center justify-between gap-2 border-t border-border/60 py-1 first:border-0">
                <span className="tabular-nums">{new Date(v.at).toLocaleString()} · {v.actor} · {v.count}</span>
                {actor.can("groups.edit") && <Button size="sm" variant="outline" onClick={() => restore.mutate(v.id)} disabled={restore.isPending}>{t("rules.restore")}</Button>}
              </div>
            ))}
          </CardContent>
        </Card>
        <Card>
          <CardHeader><CardTitle>{t("rules.dryRun")}</CardTitle></CardHeader>
          <CardContent className="text-sm">
            <p className="mb-2 text-xs text-muted-foreground">{t("rules.dryRunHint")}</p>
            {dry.data && (
              <table className="w-full tabular-nums">
                <tbody>
                  {dry.data.rules.map((x) => (
                    <tr key={x.id} className="border-t border-border/60 first:border-0">
                      <td className="py-1">{x.name}</td>
                      <td className="py-1 text-right">{x.orders}</td>
                      <td className="py-1 text-right text-muted-foreground">{x.lots.toFixed(2)} lot</td>
                    </tr>
                  ))}
                  <tr className="border-t border-border/60">
                    <td className="py-1 text-muted-foreground">{t("rules.unmatched")}</td>
                    <td className="py-1 text-right">{dry.data.unmatched.orders}</td>
                    <td className="py-1 text-right text-muted-foreground">{dry.data.unmatched.lots.toFixed(2)} lot</td>
                  </tr>
                </tbody>
              </table>
            )}
            {dry.data && dry.data.samples.length > 0 && (
              <div className="mt-3 grid gap-1 text-xs">
                <div className="text-muted-foreground">{t("rules.samples")}</div>
                {dry.data.samples.map((s) => (
                  <div key={s.orderId} className="flex justify-between gap-2"><span>#{s.login} {s.symbol} {s.lots}</span><span className="text-muted-foreground">{s.rule ?? "—"} → {s.routing}</span></div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
      {editing && (
        <RuleDialog
          rule={editing}
          onClose={() => setEditing(null)}
          onSave={(r) => {
            const exists = current.some((x) => x.id === r.id);
            update(exists ? current.map((x) => (x.id === r.id ? r : x)) : [...current, r]);
            setEditing(null);
          }}
        />
      )}
    </div>
  );
}

function RuleDialog({ rule, onClose, onSave }: { rule: RoutingRule; onClose: () => void; onSave: (r: RoutingRule) => void }) {
  const t = useT();
  const [r, setR] = React.useState<RoutingRule>(rule);
  const p = partialOf(r);
  const set = <K extends keyof RoutingRule>(k: K, v: RoutingRule[K]) => setR({ ...r, [k]: v });
  const routeValue: (typeof ROUTES)[number] = r.routing ?? (r.aBookPct !== null ? "split" : "group");
  const setRoute = (v: (typeof ROUTES)[number]) => setR({ ...r, routing: v === "ABook" || v === "BBook" ? v : null, aBookPct: v === "split" ? (r.aBookPct ?? 50) : null });
  const setPartial = (kind: (typeof PARTIALS)[number], attempts = p.attempts) =>
    set("partialFill", kind === "group" ? null : kind === "cancel" ? "CancelRemainder" : kind === "all_or_none" ? "AllOrNone" : { Retry: { max_attempts: attempts } });
  const hours = r.hoursUtc ?? [0, 24];
  return (
    <Dialog open wide onClose={onClose} title={t("rules.edit")} footer={<>
      <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
      <Button onClick={() => onSave(r)} data-testid="rule-apply">{t("rules.apply")}</Button>
    </>}>
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField label={t("rules.name")} value={r.name} onChange={(v) => set("name", v)} />
        <SelectField label={t("rules.enabled")} value={r.enabled ? "on" : "off"} options={["on", "off"] as const} onChange={(v) => set("enabled", v === "on")} />
        <TextField label={`${t("rules.groups")} (${t("rules.listHint")})`} value={r.groups.join(", ")} onChange={(v) => set("groups", list(v))} />
        <TextField label={`${t("rules.accounts")} (${t("rules.listHint")})`} value={r.accounts.join(", ")} onChange={(v) => set("accounts", list(v).map(Number).filter((n) => Number.isInteger(n)))} />
        <TextField label={`${t("rules.symbols")} (${t("rules.listHint")})`} value={r.symbols.join(", ")} onChange={(v) => set("symbols", list(v).map((s) => s.toUpperCase()))} />
        <SelectField label={t("rules.kindLabel")} value={r.kind} options={KINDS} onChange={(v) => set("kind", v)} />
        <NumField label={t("rules.minLots")} value={r.minLots ?? 0} onChange={(v) => set("minLots", v > 0 ? v : null)} step={0.01} />
        <NumField label={t("rules.maxLots")} value={r.maxLots ?? 0} onChange={(v) => set("maxLots", v > 0 ? v : null)} step={0.01} />
        <NumField label={t("rules.hourFrom")} value={hours[0]} onChange={(v) => set("hoursUtc", v === 0 && hours[1] === 24 ? null : [Math.min(23, Math.max(0, v)), hours[1]])} step={1} />
        <NumField label={t("rules.hourTo")} value={hours[1]} onChange={(v) => set("hoursUtc", hours[0] === 0 && v === 24 ? null : [hours[0], Math.min(24, Math.max(1, v))])} step={1} />
        <SelectField label={t("rules.route")} value={routeValue} options={ROUTES} onChange={setRoute} />
        <NumField label={t("rules.aBookPct")} value={r.aBookPct ?? 0} onChange={(v) => set("aBookPct", Math.min(100, Math.max(0, v)))} step={5} disabled={routeValue !== "split"} />
        <NumField label={t("rules.markup")} value={r.markupPoints ?? -1} onChange={(v) => set("markupPoints", v >= 0 ? v : null)} step={1} />
        <NumField label={t("rules.maxSlippage")} value={r.maxSlippagePoints ?? -1} onChange={(v) => set("maxSlippagePoints", v >= 0 ? v : null)} step={1} />
        <NumField label={t("rules.minToxicity")} value={r.minToxicity ?? -1} onChange={(v) => set("minToxicity", v >= 0 ? Math.min(100, v) : null)} step={5} />
        <NumField label={t("rules.maxToxicity")} value={r.maxToxicity ?? -1} onChange={(v) => set("maxToxicity", v >= 0 ? Math.min(100, v) : null)} step={5} />
        <SelectField label={t("groups.partialFillShort")} value={p.kind} options={PARTIALS} onChange={(v) => setPartial(v)} />
        <NumField label={t("groups.maxAttempts")} value={p.attempts} onChange={(v) => setPartial("retry", Math.min(10, Math.max(1, v)))} step={1} disabled={p.kind !== "retry"} />
      </div>
      <p className="mt-2 text-xs text-muted-foreground">{t("rules.overrideHint")}</p>
    </Dialog>
  );
}
