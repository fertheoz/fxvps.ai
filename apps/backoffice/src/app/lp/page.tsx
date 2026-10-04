"use client";
import { RefreshCw } from "lucide-react";
import { Button, Card, PageHeader } from "@/components/ui/primitives";
import { FixBadge } from "@/components/badges";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";

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
                <td className="px-3"><FixBadge status={s.status} /></td>
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
    </div>
  );
}
