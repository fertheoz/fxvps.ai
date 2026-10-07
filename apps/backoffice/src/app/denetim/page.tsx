"use client";
import * as React from "react";
import { ShieldAlert, ShieldCheck } from "lucide-react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label, PageHeader } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useT } from "@/lib/hooks";
import type { AuditSettings } from "@/lib/api/types";

/** Denetçi: independent LP <-> core reconciliation, order quality, auto-correction. */
export default function DenetimPage() {
  const t = useT();
  const { data } = useApiQuery("getAudit", [], { live: 2000 });
  const st = data?.status ?? null;
  // freshness by the server clock (ageMs); the browser clock may be off
  const fresh = st ? (data?.ageMs ?? 0) < 15_000 : false;
  return (
    <div data-testid="page-denetim" className="space-y-4">
      <PageHeader title={t("denetim.title")} />
      <p className="text-sm text-muted-foreground">{t("denetim.intro")}</p>
      <Card>
        <CardContent className="flex flex-wrap items-center gap-4 py-4 text-sm">
          {!st || !fresh ? (
            <Badge tone="warning">{t("denetim.silent")}</Badge>
          ) : st.ok ? (
            <span className="flex items-center gap-2 text-emerald-600 dark:text-emerald-400"><ShieldCheck className="h-5 w-5" />{t("denetim.ok")}</span>
          ) : (
            <span className="flex items-center gap-2 text-red-600 dark:text-red-400"><ShieldAlert className="h-5 w-5" />{t("denetim.mismatch")}</span>
          )}
          {st && <span>{t("denetim.inFlight")}: <b>{st.inFlight}</b></span>}
          {st && <span>{t("denetim.incidents")}: <b>{st.incidentsTotal}</b></span>}
          {st && <span>{t("denetim.corrections")}: <b>{st.correctionsSent}</b></span>}
          {st && <span>{t("denetim.autoheal")}: <Badge tone={st.autoheal ? "success" : "muted"}>{st.autoheal ? t("denetim.on") : t("denetim.off")}</Badge></span>}
          {st?.resetMs && <span>{t("denetim.zeroAt")}: <b>{new Date(st.resetMs).toLocaleString()}</b></span>}
          <ZeroPointButton />
        </CardContent>
      </Card>

      {st && st.openMismatches.length > 0 && (
        <Card className="border-red-500/60">
          <CardHeader><CardTitle>{t("denetim.open")}</CardTitle></CardHeader>
          <CardContent className="space-y-1 font-mono text-sm">
            {st.openMismatches.map((m) => (
              <div key={m.lp + m.symbol}>{m.lp} {m.symbol}: {t("denetim.toTrade")} <b>{m.diff}</b></div>
            ))}
          </CardContent>
        </Card>
      )}

      {st && (
        <Card className="overflow-x-auto">
          <CardHeader><CardTitle>{t("denetim.quality")}</CardTitle></CardHeader>
          <table className="w-full text-sm tabular-nums">
            <thead className="bg-muted/50 text-xs text-muted-foreground">
              <tr>{["LP", t("denetim.orders"), t("denetim.fills"), t("denetim.rejects"), t("denetim.partial"), "ack p50", "ack p99", "fill p50", "fill p99"].map((h) => <th key={h} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr>
            </thead>
            <tbody>
              {Object.entries(st.lps).map(([lp, s]) => (
                <tr key={lp} className="border-t border-border">
                  <td className="px-3 py-2 font-medium">{lp}</td>
                  <td className="px-3">{s.orders}</td>
                  <td className="px-3">{s.fills}</td>
                  <td className="px-3">{s.rejects}</td>
                  <td className="px-3">{s.partial}</td>
                  <td className="px-3">{s.ack_ms_p50} ms</td>
                  <td className="px-3">{s.ack_ms_p99} ms</td>
                  <td className="px-3">{s.fill_ms_p50} ms</td>
                  <td className="px-3">{s.fill_ms_p99} ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      <SettingsCard />

      {st && (
        <Card className="overflow-x-auto">
          <CardHeader><CardTitle>{t("denetim.log")}</CardTitle></CardHeader>
          <table className="w-full text-sm">
            <thead className="bg-muted/50 text-xs text-muted-foreground">
              <tr>{[t("denetim.time"), t("denetim.kind"), "LP", "Symbol", "cl_ord_id", t("denetim.detail")].map((h) => <th key={h} className="px-3 py-2 text-left font-medium">{h}</th>)}</tr>
            </thead>
            <tbody>
              {st.incidents.length === 0 && <tr><td colSpan={6} className="px-3 py-4 text-center text-muted-foreground">{t("denetim.none")}</td></tr>}
              {st.incidents.map((i, k) => (
                <tr key={k} className="border-t border-border">
                  <td className="px-3 py-1 whitespace-nowrap">{new Date(i.ts_ms).toLocaleString()}</td>
                  <td className="px-3"><Badge tone={i.kind === "net_mismatch" || i.kind === "foreign_fill" ? "danger" : "warning"}>{i.kind}</Badge></td>
                  <td className="px-3">{i.lp}</td>
                  <td className="px-3">{i.symbol}</td>
                  <td className="px-3 font-mono text-xs">{i.cl_ord_id ?? "—"}</td>
                  <td className="px-3 text-xs">{i.detail}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {data && data.corrections.length > 0 && (
        <Card>
          <CardHeader><CardTitle>{t("denetim.correctionLog")}</CardTitle></CardHeader>
          <CardContent><pre className="max-h-72 overflow-auto rounded bg-muted p-3 text-xs">{data.corrections.map((c) => JSON.stringify(c)).join("\n")}</pre></CardContent>
        </Card>
      )}
    </div>
  );
}

function ZeroPointButton() {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const reset = useApiMutation(() => api().resetAudit(actor), (r) => toast(`${t("denetim.zeroDone")} ${new Date(r.resetAt).toLocaleString()}`));
  if (!actor.can("lp.manage")) return null;
  return (
    <Button size="sm" variant="outline" className="ml-auto" disabled={!mfa || reset.isPending} data-testid="zero-point"
      onClick={() => { if (window.confirm(t("denetim.zeroConfirm"))) reset.mutate(undefined); }}>
      {t("denetim.zero")}
    </Button>
  );
}

function SettingsCard() {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const { data } = useApiQuery("getAudit", []);
  const [s, setS] = React.useState<AuditSettings | null>(null);
  const cur = s ?? data?.settings ?? { autoheal: false, maxLots: 5, maxPerMin: 5 };
  const save = useApiMutation((v: AuditSettings) => api().saveAuditSettings(v, actor), () => { toast(t("common.saved")); setS(null); });
  return (
    <Card>
      <CardHeader><CardTitle>{t("denetim.settings")}</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p className="text-muted-foreground">{t("denetim.autohealHint")}</p>
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={cur.autoheal} onChange={(e) => setS({ ...cur, autoheal: e.target.checked })} data-testid="autoheal" />
          {t("denetim.autoheal")}
        </label>
        <div className="grid max-w-md gap-3 sm:grid-cols-2">
          <div className="space-y-1"><Label>{t("denetim.maxLots")}</Label><Input type="number" value={cur.maxLots} onChange={(e) => setS({ ...cur, maxLots: Number(e.target.value) })} /></div>
          <div className="space-y-1"><Label>{t("denetim.maxPerMin")}</Label><Input type="number" value={cur.maxPerMin} onChange={(e) => setS({ ...cur, maxPerMin: Number(e.target.value) })} /></div>
        </div>
        <Button disabled={!mfa || !s || save.isPending} onClick={() => s && save.mutate(s)}>{t("common.save")}</Button>
      </CardContent>
    </Card>
  );
}
