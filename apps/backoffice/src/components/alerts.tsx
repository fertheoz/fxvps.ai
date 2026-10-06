"use client";
import * as React from "react";
import { Bell, Check } from "lucide-react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from "@/components/ui/primitives";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { Alert } from "@/lib/api";

const tone = (s: Alert["severity"]) => (s === "critical" ? "danger" : s === "warning" ? "warning" : "info");

function AlertRow({ a, onAck }: { a: Alert; onAck?: (id: string) => void }) {
  const t = useT();
  const f = useFormat();
  return (
    <div className="flex items-start justify-between gap-2 border-b border-border/60 py-1.5 text-sm last:border-0" data-testid={`alert-${a.id}`}>
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-1.5"><Badge tone={tone(a.severity)}>{a.severity}</Badge><span className="font-medium">{a.title}</span>{a.acked && !a.resolvedAt && <span className="text-xs text-muted-foreground">· {t("alerts.acked")}</span>}</div>
        <div className="truncate text-xs text-muted-foreground">{a.detail}</div>
        <div className="text-[11px] text-muted-foreground">{f.date(new Date(a.raisedAt / 1e6).toISOString())}{a.resolvedAt ? ` → ${t("alerts.resolved")} ${f.date(new Date(a.resolvedAt / 1e6).toISOString())}` : ""}</div>
      </div>
      {onAck && !a.acked && !a.resolvedAt && <Button size="sm" variant="outline" onClick={() => onAck(a.id)} aria-label={t("alerts.ack")}><Check className="h-3 w-3" />{t("alerts.ack")}</Button>}
    </div>
  );
}

/** Dashboard card: active alerts with acknowledge, plus the last resolved ones. */
export function AlertsCard() {
  const t = useT();
  const actor = useActor();
  const q = useApiQuery("listAlerts", [], { live: 10000 });
  const ack = useApiMutation((id: string) => api().ackAlert(id, actor));
  const active = q.data?.active ?? [];
  const recent = (q.data?.recent ?? []).slice(0, 5);
  return (
    <Card data-testid="dash-alerts">
      <CardHeader><CardTitle className="flex items-center gap-2"><Bell className="h-4 w-4" />{t("dash.alerts")}{active.length > 0 && <Badge tone="danger">{active.length}</Badge>}</CardTitle></CardHeader>
      <CardContent>
        {active.length === 0 && <div className="text-sm text-muted-foreground">{t("dash.noAlerts")}</div>}
        {active.map((a) => <AlertRow key={a.id} a={a} onAck={(id) => ack.mutate(id)} />)}
        {recent.length > 0 && (
          <>
            <div className="mt-3 text-[10px] uppercase text-muted-foreground">{t("alerts.recent")}</div>
            {recent.map((a) => <AlertRow key={a.id} a={a} />)}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/** Top-bar bell with the active count; opens a small panel. */
export function AlertBell() {
  const t = useT();
  const actor = useActor();
  const [open, setOpen] = React.useState(false);
  const q = useApiQuery("listAlerts", [], { live: 10000 });
  const ack = useApiMutation((id: string) => api().ackAlert(id, actor));
  const active = q.data?.active ?? [];
  const unacked = active.filter((a) => !a.acked).length;
  return (
    <div className="relative">
      <Button variant="ghost" size="icon" aria-label={t("alerts.title")} onClick={() => setOpen((v) => !v)} data-testid="alert-bell">
        <Bell className="h-4 w-4" />
        {unacked > 0 && <span className="absolute -right-0.5 -top-0.5 min-w-4 rounded-full bg-red-600 px-1 text-center text-[10px] font-semibold leading-4 text-white">{unacked}</span>}
      </Button>
      {open && (
        <div className="absolute right-0 z-40 mt-1 w-[min(92vw,380px)] rounded-md border border-border bg-background p-3 shadow-xl" onMouseLeave={() => setOpen(false)}>
          <div className="mb-1 text-xs font-medium uppercase text-muted-foreground">{t("alerts.active")} ({active.length})</div>
          {active.length === 0 && <div className="text-sm text-muted-foreground">{t("alerts.none")}</div>}
          {active.map((a) => <AlertRow key={a.id} a={a} onAck={(id) => ack.mutate(id)} />)}
        </div>
      )}
    </div>
  );
}
