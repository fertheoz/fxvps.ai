"use client";
import * as React from "react";
import { Button, Dialog, FieldError, Input, Label, Tabs } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { usePrefs } from "@/lib/prefs";
import { BalanceOpRequest, type Client } from "@/lib/schemas";
import { formatMoney, parseToMinor } from "@/lib/money";
import { balancePermission, requiresSecondApproval } from "@/lib/rbac";
import { uuid } from "@/lib/utils";

type OpType = "deposit" | "withdraw" | "credit";

export function BalanceOps({ client }: { client: Client }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const { locale } = usePrefs();
  const toast = useToast();
  const [type, setType] = React.useState<OpType>("deposit");
  const [amount, setAmount] = React.useState("");
  const [reason, setReason] = React.useState("");
  const [errors, setErrors] = React.useState<Record<string, string>>({});
  const [pending, setPending] = React.useState<BalanceOpRequest | null>(null);
  const mut = useApiMutation((req: BalanceOpRequest) => api().balanceOp(req, actor), (r) => {
    toast(r.status === "applied" ? t("clients.applied") : t("clients.pending"));
    setPending(null);
    setAmount("");
    setReason("");
  });

  const allowed = actor.can(balancePermission(type));
  const anyAllowed = (["deposit", "withdraw", "credit"] as const).some((x) => actor.can(balancePermission(x)));

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    let minor: number;
    try {
      minor = parseToMinor(amount, client.currency, locale === "tr" ? "," : ".");
    } catch (err) {
      setErrors({ amount: err instanceof Error ? err.message : "invalid" });
      return;
    }
    const parsed = BalanceOpRequest.safeParse({ clientId: client.id, type, amount: minor, currency: client.currency, reason, idempotencyKey: uuid() });
    if (!parsed.success) {
      setErrors(Object.fromEntries(parsed.error.issues.map((i) => [String(i.path[0]), i.message])));
      return;
    }
    setErrors({});
    setPending(parsed.data);
  };

  return (
    <section className="grid gap-2">
      <h3 className="text-sm font-semibold">{t("clients.balanceOps")}</h3>
      {!anyAllowed ? (
        <p className="text-sm text-muted-foreground">{t("clients.noPermission")}</p>
      ) : (
        <form onSubmit={submit} className="grid gap-3 rounded-md border border-border p-3" data-testid="balance-form">
          <Tabs<OpType> value={type} onChange={setType} items={[
            { value: "deposit", label: t("clients.deposit") },
            { value: "withdraw", label: t("clients.withdraw") },
            { value: "credit", label: t("clients.creditOp") },
          ]} />
          <Label>
            {t("clients.amount")} ({client.currency})
            <Input inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder={locale === "tr" ? "1.000,00" : "1,000.00"} name="amount" />
            <FieldError msg={errors.amount} />
          </Label>
          <Label>
            {t("clients.reason")}
            <Input value={reason} onChange={(e) => setReason(e.target.value)} name="reason" />
            <FieldError msg={errors.reason} />
          </Label>
          <Button type="submit" disabled={!allowed}>{t("common.confirm")}</Button>
          {!allowed && <p className="text-xs text-muted-foreground">{t("clients.noPermission")}</p>}
        </form>
      )}
      <Dialog
        open={!!pending}
        onClose={() => setPending(null)}
        title={t("clients.confirmOp")}
        footer={<>
          <Button variant="outline" onClick={() => setPending(null)}>{t("common.cancel")}</Button>
          <Button onClick={() => pending && mut.mutate(pending)} disabled={mut.isPending} data-testid="confirm-balance">{t("common.confirm")}</Button>
        </>}
      >
        {pending && (
          <div className="grid gap-2 text-sm">
            <p><strong>{t(pending.type === "deposit" ? "clients.deposit" : pending.type === "withdraw" ? "clients.withdraw" : "clients.creditOp")}</strong> {formatMoney(pending.amount, pending.currency, f.intl)} → #{client.login}</p>
            <p className="text-muted-foreground">“{pending.reason}”</p>
            <p className="text-xs text-muted-foreground">{t("clients.confirmText")}</p>
            {requiresSecondApproval(pending.amount) && <p className="text-xs text-amber-600 dark:text-amber-400">{t("clients.fourEyes")}</p>}
          </div>
        )}
      </Dialog>
    </section>
  );
}
