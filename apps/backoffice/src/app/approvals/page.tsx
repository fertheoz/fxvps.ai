"use client";
import * as React from "react";
import { Check, X } from "lucide-react";
import { Badge, Button, Card, CardContent, Dialog, Input, Label, PageHeader } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import type { ApprovalRequest } from "@/lib/api";
import { FundingSection } from "@/components/funding";

export default function ApprovalsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const toast = useToast();
  const [history, setHistory] = React.useState(false);
  const { data = [] } = useApiQuery("listApprovals", [history ? "all" : "pending_approval"], { live: 5000 });
  const [rejecting, setRejecting] = React.useState<ApprovalRequest | null>(null);
  const [reason, setReason] = React.useState("");
  const approve = useApiMutation((id: string) => api().approve(id, actor), () => toast(t("approvals.approved")));
  const reject = useApiMutation((a: { id: string; reason: string }) => api().reject(a.id, a.reason, actor), () => {
    toast(t("approvals.rejected"));
    setRejecting(null);
    setReason("");
  });
  const mine = (a: ApprovalRequest) => (actor.sub && a.requestedBySub ? a.requestedBySub === actor.sub : a.requestedBy === actor.name);
  const opLabel = (a: ApprovalRequest) => t(a.type === "deposit" ? "clients.deposit" : a.type === "withdraw" ? "clients.withdraw" : "clients.creditOp");

  return (
    <div data-testid="page-approvals">
      <PageHeader title={t("approvals.title")}>
        <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <input type="checkbox" checked={history} onChange={(e) => setHistory(e.target.checked)} /> {t("approvals.history")}
        </label>
      </PageHeader>
      <p className="mb-4 text-sm text-muted-foreground">{t("approvals.body")}</p>
      {data.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="approvals-empty">{t("approvals.empty")}</p>
      ) : (
        <div className="grid gap-3" data-testid="approvals-list">
          {data.map((a) => (
            <Card key={a.id} data-testid={`approval-${a.id}`}>
              <CardContent className="flex flex-wrap items-center gap-x-6 gap-y-2">
                <div className="grid gap-0.5">
                  <span className="text-sm font-medium">{opLabel(a)} {f.money(a.amount, a.currency)} → #{a.login}</span>
                  <span className="text-xs text-muted-foreground">“{a.reason}”</span>
                </div>
                <div className="grid gap-0.5 text-xs text-muted-foreground">
                  <span>{t("approvals.requestedBy")}: <strong className="text-foreground">{a.requestedBy}</strong> ({a.requestedByRole})</span>
                  <span>{t("approvals.requestedAt")}: {f.date(a.requestedAt)}</span>
                </div>
                <div className="ml-auto flex items-center gap-2">
                  {a.status !== "pending_approval" ? (
                    <Badge tone={a.status === "applied" ? "success" : "danger"}>{a.status}{a.decidedBy ? ` · ${a.decidedBy}` : ""}</Badge>
                  ) : mine(a) ? (
                    <>
                      <Badge tone="warning">{t("approvals.own")}</Badge>
                      <Button size="sm" variant="outline" onClick={() => setRejecting(a)} disabled={!actor.can("balance.approve")} data-testid="withdraw"><X className="h-3.5 w-3.5" />{t("approvals.withdraw")}</Button>
                    </>
                  ) : (
                    <>
                      <Button size="sm" variant="outline" onClick={() => setRejecting(a)} disabled={!actor.can("balance.approve")}><X className="h-3.5 w-3.5" />{t("approvals.reject")}</Button>
                      <Button size="sm" onClick={() => approve.mutate(a.id)} disabled={!actor.can("balance.approve") || approve.isPending} data-testid="approve"><Check className="h-3.5 w-3.5" />{t("approvals.approve")}</Button>
                    </>
                  )}
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
      <FundingSection />
      <Dialog
        open={!!rejecting}
        onClose={() => setRejecting(null)}
        title={t("approvals.reject")}
        footer={<>
          <Button variant="outline" onClick={() => setRejecting(null)}>{t("common.cancel")}</Button>
          <Button onClick={() => rejecting && reject.mutate({ id: rejecting.id, reason })} disabled={reject.isPending || reason.trim().length < 3} data-testid="confirm-reject">{t("approvals.reject")}</Button>
        </>}
      >
        <Label>
          {t("approvals.rejectReason")}
          <Input value={reason} onChange={(e) => setReason(e.target.value)} name="reject-reason" />
        </Label>
      </Dialog>
    </div>
  );
}
