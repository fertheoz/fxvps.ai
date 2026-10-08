"use client";
import * as React from "react";
import { Check, Send, X } from "lucide-react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { FundingDecision, FundingRequest } from "@/lib/api";
import type { MessageKey } from "@/lib/i18n";

const tone = (s: FundingRequest["status"]) => (s === "requested" ? "warning" : s === "rejected" ? "danger" : "success");
const usdt = (micro: number) => (micro / 1e6).toFixed(6);
/** hazine bound a payment after the decision and nobody has reviewed it yet. */
const lateOpen = (r: FundingRequest) => !!r.paidAfterDecision && !r.lateHandledBy;

/** Client-initiated deposit / withdrawal requests awaiting a staff decision. */
export function FundingSection() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const [history, setHistory] = React.useState(false);
  const [notes, setNotes] = React.useState<Record<string, string>>({});
  const q = useApiQuery("listFunding", [history ? "all" : "open"], { live: 5000 });
  const decide = useApiMutation((a: { id: string; decision: FundingDecision }) => api().decideFunding(a.id, a.decision, notes[a.id]?.trim() || undefined, actor), () => toast(t("funding.decided")));
  const rows = q.data ?? [];
  const canDecide = (r: FundingRequest) => actor.can(r.kind === "deposit" ? "balance.deposit" : "balance.withdraw");
  return (
    <Card className="mt-6" data-testid="funding-section">
      <CardHeader>
        <CardTitle className="flex items-center justify-between gap-2">{t("funding.title")}
          <label className="flex items-center gap-1.5 text-xs font-normal text-muted-foreground"><input type="checkbox" checked={history} onChange={(e) => setHistory(e.target.checked)} /> {t("funding.history")}</label>
        </CardTitle>
      </CardHeader>
      <CardContent className="grid gap-3">
        <p className="text-xs text-muted-foreground">{t("funding.hint")}</p>
        {rows.length === 0 && <p className="text-sm text-muted-foreground" data-testid="funding-empty">{t("funding.empty")}</p>}
        {rows.map((r) => (
          <div key={r.id} className="flex flex-wrap items-center gap-x-6 gap-y-2 border-t border-border pt-3 first:border-0 first:pt-0" data-testid={`funding-${r.id}`}>
            <div className="grid gap-0.5">
              <span className="text-sm font-medium">{t(r.kind === "deposit" ? "clients.deposit" : "clients.withdraw")} {f.money(r.amount, r.currency)} · #{r.account} {r.clientName ? <span className="text-muted-foreground">{r.clientName}</span> : null}</span>
              <span className="text-xs text-muted-foreground">{t(`funding.method.${r.method}` as MessageKey)}{r.details ? ` · “${r.details}”` : ""}</span>
              {r.expectedMicro ? (
                <span className="text-xs">
                  {usdt(r.expectedMicro)} USDT ·{" "}
                  {r.txHash ? (
                    <>
                      <a className="text-emerald-600 underline dark:text-emerald-400" href={`https://tronscan.org/#/transaction/${r.txHash}`} target="_blank" rel="noreferrer" data-testid={`funding-onchain-${r.id}`}>{t("funding.onchain")}</a>
                      {" · "}
                      {/* hazine binds 1x..2x the invoice: show what really came */}
                      <span className={r.receivedMicro === r.expectedMicro ? "text-muted-foreground" : "font-medium text-amber-600 dark:text-amber-400"} data-testid={`funding-received-${r.id}`}>
                        {t("funding.received")} {r.receivedMicro != null ? usdt(r.receivedMicro) : "?"} USDT{r.receivedMicro === r.expectedMicro ? "" : ` · ${t("funding.amountDiffers")}`}
                      </span>
                    </>
                  ) : (
                    r.payUrl ? <a className="text-muted-foreground underline" href={r.payUrl} target="_blank" rel="noreferrer">{t("funding.waitingChain")}</a> : <span className="text-muted-foreground">{t("funding.waitingChain")}</span>
                  )}
                </span>
              ) : null}
              {r.paidAfterDecision ? (
                <span className={`text-xs ${r.lateHandledBy ? "text-muted-foreground" : "font-medium text-red-600 dark:text-red-400"}`} data-testid={`funding-late-${r.id}`}>
                  {t("funding.paidLate")}{r.lateHandledBy ? ` · ${t("funding.lateHandled")} ${r.lateHandledBy}` : ""}
                </span>
              ) : null}
              <span className="text-xs text-muted-foreground">{f.date(r.requestedAtIso ?? new Date(r.requestedAt / 1e6).toISOString())}{r.decidedBy ? ` · ${r.decidedBy}` : ""}{r.note ? ` · ${r.note}` : ""}</span>
            </div>
            <div className="ml-auto flex flex-wrap items-center gap-2">
              <Badge tone={tone(r.status)}>{r.status}</Badge>
              {r.status === "requested" && actor.can("clients.edit") && (
                <>
                  <Input className="w-44" placeholder={t("funding.note")} value={notes[r.id] ?? ""} onChange={(e) => setNotes({ ...notes, [r.id]: e.target.value })} />
                  <Button size="sm" variant="outline" onClick={() => decide.mutate({ id: r.id, decision: "reject" })} disabled={decide.isPending}><X className="h-3.5 w-3.5" />{t("funding.reject")}</Button>
                  <Button size="sm" onClick={() => decide.mutate({ id: r.id, decision: "approve" })} disabled={!canDecide(r) || decide.isPending} data-testid={`funding-approve-${r.id}`}><Check className="h-3.5 w-3.5" />{t("funding.approve")}</Button>
                </>
              )}
              {r.status === "approved" && r.kind === "withdraw" && actor.can("clients.edit") && (
                <Button size="sm" variant="outline" onClick={() => decide.mutate({ id: r.id, decision: "paid" })} disabled={decide.isPending}><Send className="h-3.5 w-3.5" />{t("funding.paid")}</Button>
              )}
              {r.status !== "requested" && lateOpen(r) && actor.can("clients.edit") && (
                <>
                  <Input className="w-44" placeholder={t("funding.handledNote")} value={notes[r.id] ?? ""} onChange={(e) => setNotes({ ...notes, [r.id]: e.target.value })} />
                  <Button size="sm" variant="outline" onClick={() => decide.mutate({ id: r.id, decision: "handled" })} disabled={!notes[r.id]?.trim() || decide.isPending} data-testid={`funding-handled-${r.id}`}><Check className="h-3.5 w-3.5" />{t("funding.handled")}</Button>
                </>
              )}
            </div>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}
