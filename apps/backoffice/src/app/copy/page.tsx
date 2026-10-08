"use client";
import * as React from "react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label, PageHeader } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { CopyStrategy } from "@/lib/api/types";
import { Sparkline } from "@/components/charts";

/** Copy trading: strategy catalogue, follower subscriptions, performance fees. */
export default function CopyPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const { data } = useApiQuery("getCopyOverview", [], { live: 5000 });
  const unsub = useApiMutation((r: { follower: number; provider: number; close: boolean }) => api().copyUnsubscribe(r, actor), () => toast(t("common.saved")));
  const settle = useApiMutation((provider: number) => api().copySettle(provider, actor), (r) => toast(`${t("copy.feesCharged")}: ${f.money(r.fees, "USD")}`));
  const edit = actor.can("clients.edit");
  return (
    <div data-testid="page-copy" className="space-y-4">
      <PageHeader title={t("copy.title")} />
      <p className="text-sm text-muted-foreground">
        {t("copy.intro")} {data && <span>{t("copy.groups")}: <b>{data.groups.join(", ") || "—"}</b></span>}
      </p>
      <Card className="overflow-x-auto">
        <CardHeader><CardTitle>{t("copy.strategies")}</CardTitle></CardHeader>
        <table className="w-full text-sm tabular-nums">
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            <tr>{["#", t("copy.name"), t("copy.fee"), t("copy.pnl30"), t("copy.maxDd"), t("copy.curve"), t("copy.deals30"), t("copy.winRate"), t("copy.followers"), ""].map((h, i) => <th key={i} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr>
          </thead>
          <tbody>
            {data?.strategies.length === 0 && <tr><td colSpan={10} className="px-3 py-4 text-center text-muted-foreground">{t("copy.none")}</td></tr>}
            {data?.strategies.map((s) => (
              <tr key={s.account} className="border-t border-border" data-testid={`strategy-${s.account}`}>
                <td className="px-3 py-1">{s.account}</td>
                <td className="px-3">{s.name} {s.public ? <Badge tone="success">{t("copy.public")}</Badge> : <Badge tone="muted">{t("copy.hidden")}</Badge>}</td>
                <td className="px-3">{(s.perfFeeBps / 100).toFixed(1)}%</td>
                <td className={`px-3 ${s.pnl30d < 0 ? "text-red-600 dark:text-red-400" : "text-emerald-600 dark:text-emerald-400"}`}>{f.money(s.pnl30d, "USD")}</td>
                <td className="px-3" data-testid={`strategy-maxdd-${s.account}`}>
                  {s.maxDrawdown ? <span className="text-red-600 dark:text-red-400">{f.money(-s.maxDrawdown, "USD")}</span> : f.money(0, "USD")}
                  {s.maxDrawdownPct != null && <span className="ml-1 text-xs text-muted-foreground">({s.maxDrawdownPct.toFixed(1)}%)</span>}
                </td>
                <td className="px-3"><Sparkline points={s.equityCurve ?? []} width={110} height={24} label={t("copy.curve")} /></td>
                <td className="px-3">{s.deals30d}</td>
                <td className="px-3">{s.winRate == null ? "—" : `${Math.round(s.winRate * 100)}%`}</td>
                <td className="px-3">{s.followers}</td>
                <td className="px-3 text-right">
                  {actor.can("balance.approve") && (
                    <Button size="sm" variant="outline" disabled={!mfa || settle.isPending} onClick={() => settle.mutate(s.account)}>{t("copy.settle")}</Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
      {edit && <StrategyForm existing={data?.strategies ?? []} />}
      <Card className="overflow-x-auto">
        <CardHeader><CardTitle>{t("copy.subscriptions")}</CardTitle></CardHeader>
        <table className="w-full text-sm tabular-nums">
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            <tr>{[t("copy.follower"), t("copy.provider"), t("copy.ratio"), t("copy.equityStop"), t("copy.realized"), t("copy.feesPaid"), t("copy.state"), ""].map((h, i) => <th key={i} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr>
          </thead>
          <tbody>
            {data?.subscriptions.length === 0 && <tr><td colSpan={8} className="px-3 py-4 text-center text-muted-foreground">{t("copy.noSubs")}</td></tr>}
            {data?.subscriptions.map((s) => (
              <tr key={`${s.follower}-${s.provider}`} className="border-t border-border">
                <td className="px-3 py-1">{s.follower}</td>
                <td className="px-3">{s.provider}</td>
                <td className="px-3">{(s.ratioBps / 100).toFixed(0)}%</td>
                <td className="px-3">{s.equityStopPct ? `${s.equityStopPct}%` : "—"}</td>
                <td className="px-3">{f.money(s.realized, "USD")}</td>
                <td className="px-3">{f.money(s.feesPaid, "USD")}</td>
                <td className="px-3">{s.active ? <Badge tone="success">{t("copy.active")}</Badge> : <Badge tone="muted">{s.stoppedReason ?? t("copy.stopped")}</Badge>}</td>
                <td className="px-3 text-right">
                  {edit && s.active && (
                    <Button size="sm" variant="outline" disabled={!mfa || unsub.isPending}
                      onClick={() => { if (window.confirm(t("copy.unsubConfirm"))) unsub.mutate({ follower: s.follower, provider: s.provider, close: true }); }}>
                      {t("copy.stop")}
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </div>
  );
}

function StrategyForm({ existing }: { existing: CopyStrategy[] }) {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const [account, setAccount] = React.useState("");
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [fee, setFee] = React.useState("20");
  const [pub, setPub] = React.useState(true);
  const save = useApiMutation(
    () => api().saveCopyStrategy(Number(account), { name, description, perfFeeBps: Math.round(Number(fee) * 100), public: pub }, actor),
    () => { toast(t("common.saved")); setAccount(""); setName(""); setDescription(""); },
  );
  const pick = (v: string) => {
    setAccount(v);
    const s = existing.find((x) => String(x.account) === v);
    if (s) { setName(s.name); setDescription(s.description); setFee(String(s.perfFeeBps / 100)); setPub(s.public); }
  };
  return (
    <Card>
      <CardHeader><CardTitle>{t("copy.addStrategy")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 text-sm md:grid-cols-5">
        <div className="space-y-1"><Label>{t("copy.account")}</Label><Input value={account} onChange={(e) => pick(e.target.value)} inputMode="numeric" data-testid="strategy-account" /></div>
        <div className="space-y-1"><Label>{t("copy.name")}</Label><Input value={name} onChange={(e) => setName(e.target.value)} maxLength={60} /></div>
        <div className="space-y-1 md:col-span-2"><Label>{t("copy.description")}</Label><Input value={description} onChange={(e) => setDescription(e.target.value)} maxLength={500} /></div>
        <div className="space-y-1"><Label>{t("copy.fee")} (%)</Label><Input value={fee} onChange={(e) => setFee(e.target.value)} inputMode="decimal" /></div>
        <label className="flex items-center gap-2"><input type="checkbox" checked={pub} onChange={(e) => setPub(e.target.checked)} />{t("copy.public")}</label>
        <div className="md:col-span-4 flex justify-end">
          <Button disabled={!mfa || !account || !name.trim() || save.isPending} onClick={() => save.mutate(undefined)} data-testid="strategy-save">{t("common.save")}</Button>
        </div>
      </CardContent>
    </Card>
  );
}
