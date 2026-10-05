"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { CornerDownRight } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Dialog, FieldError, Input, Label, PageHeader, Pnl, Select, Sheet, Tabs } from "@/components/ui/primitives";
import { BookBadge, KycBadge, marginLevel, SideBadge, StatusBadge } from "@/components/badges";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { usePrefs } from "@/lib/prefs";
import { BalanceOpRequest, KycStatus, type Client } from "@/lib/schemas";
import { formatMoney, parseToMinor } from "@/lib/money";
import { balancePermission, requiresSecondApproval } from "@/lib/rbac";
import { uuid } from "@/lib/utils";

const col = createColumnHelper<Client>();

export default function ClientsPage() {
  const t = useT();
  const f = useFormat();
  const { data = [] } = useApiQuery("listClients", [{}], { live: 5000 });
  const [kyc, setKyc] = React.useState<string>("all");
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [opening, setOpening] = React.useState(false);
  const actor = useActor();
  const mfaOk = useMfaOk();

  // Order masters followed by their sub-accounts (tree view).
  const ordered = React.useMemo(() => {
    const filtered = data.filter((c) => kyc === "all" || c.kyc === kyc);
    const masters = filtered.filter((c) => c.parentId === null);
    const out: Client[] = [];
    for (const m of masters) out.push(m, ...filtered.filter((c) => c.parentId === m.id));
    out.push(...filtered.filter((c) => c.parentId !== null && !masters.some((m) => m.id === c.parentId)));
    return out;
  }, [data, kyc]);

  const columns = React.useMemo(() => [
    col.accessor("login", { header: t("clients.login"), cell: (c) => <span className="tabular-nums font-medium">{c.getValue()}</span> }),
    col.accessor("name", {
      header: t("clients.name"),
      cell: (c) => c.row.original.parentId
        ? <span className="flex items-center gap-1 pl-3 text-muted-foreground"><CornerDownRight className="h-3 w-3" />{c.getValue()}</span>
        : c.getValue(),
    }),
    col.accessor("group", { header: t("clients.group"), cell: (c) => <span className="text-xs">{c.getValue()}</span> }),
    col.accessor("country", { header: t("clients.country") }),
    col.accessor("status", { header: t("common.status"), cell: (c) => <StatusBadge status={c.getValue()} /> }),
    col.accessor("kyc", { header: t("clients.kyc"), cell: (c) => <KycBadge kyc={c.getValue()} /> }),
    col.accessor("balance", { header: t("clients.balance"), cell: (c) => <span className="tabular-nums">{f.money(c.getValue(), c.row.original.currency)}</span> }),
    col.accessor("equity", { header: t("clients.equity"), cell: (c) => <span className="tabular-nums">{f.money(c.getValue(), c.row.original.currency)}</span> }),
    col.accessor((c) => marginLevel(c.equity, c.margin) ?? Number.POSITIVE_INFINITY, {
      id: "ml",
      header: t("clients.marginLevel"),
      cell: (c) => {
        const v = c.getValue();
        if (!Number.isFinite(v)) return <span className="text-muted-foreground">—</span>;
        return <Badge tone={v < 50 ? "danger" : v < 100 ? "warning" : "muted"}>{f.num(v, 0)}%</Badge>;
      },
    }),
  ], [t, f]);

  return (
    <div data-testid="page-clients">
      <PageHeader title={t("clients.title")}>{actor.can("clients.edit") && <Button onClick={() => setOpening(true)} disabled={!mfaOk} data-testid="open-account">{t("clients.open")}</Button>}</PageHeader>
      <DataTable
        data={ordered}
        columns={columns}
        onRowClick={(c) => setSelectedId(c.id)}
        getRowId={(c) => c.id}
        testId="clients-table"
        toolbar={
          <Select value={kyc} onChange={(e) => setKyc(e.target.value)} aria-label={t("clients.kyc")}>
            <option value="all">{t("clients.kyc")}: {t("common.all")}</option>
            {KycStatus.options.map((k) => <option key={k} value={k}>{k}</option>)}
          </Select>
        }
      />
      <ClientDrawer client={data.find((c) => c.id === selectedId) ?? null} all={data} onClose={() => setSelectedId(null)} />
      <OpenAccountDialog open={opening} onClose={() => setOpening(false)} onOpened={(id) => setSelectedId(id)} />
    </div>
  );
}

function OpenAccountDialog({ open, onClose, onOpened }: { open: boolean; onClose: () => void; onOpened: (id: string) => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const groups = useApiQuery("listGroups", [], { enabled: open });
  const [name, setName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [group, setGroup] = React.useState("");
  const [error, setError] = React.useState<string | undefined>();
  const g = group || groups.data?.[0]?.name || "";
  const mut = useApiMutation(
    () => api().openAccount({ name, email, group: g }, actor),
    (c) => { toast(`${t("clients.opened")} #${c.login}`); onClose(); onOpened(c.id); },
  );
  const submit = () => {
    if (name.trim().length < 2 || !email.includes("@") || !g) return setError(t("clients.openInvalid"));
    setError(undefined);
    mut.mutate();
  };
  return (
    <Dialog open={open} onClose={onClose} title={t("clients.open")} footer={<Button onClick={submit} disabled={mut.isPending}>{t("clients.open")}</Button>}>
      <div className="grid gap-3" data-testid="open-account-form">
        <Label>{t("clients.name")}<Input value={name} onChange={(e) => setName(e.target.value)} /></Label>
        <Label>E-mail<Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} /></Label>
        <Label>{t("clients.group")}
          <Select value={g} onChange={(e) => setGroup(e.target.value)}>
            {(groups.data ?? []).map((x) => <option key={x.id} value={x.name}>{x.name} ({x.currency}, 1:{x.leverage})</option>)}
          </Select>
        </Label>
        <p className="text-xs text-muted-foreground">{t("clients.openHint")}</p>
        <FieldError msg={error} />
      </div>
    </Dialog>
  );
}

function ClientDrawer({ client, all, onClose }: { client: Client | null; all: Client[]; onClose: () => void }) {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const positions = useApiQuery("listPositions", [], { live: client ? 5000 : undefined });
  const kycMut = useApiMutation((k: Client["kyc"]) => api().setKyc(client!.id, k, actor));
  const groups = useApiQuery("listGroups", [], { enabled: !!client });
  const groupMut = useApiMutation((g: string) => api().setGroup(client!.id, g, actor));
  if (!client) return null;
  const subs = all.filter((c) => c.parentId === client.id);
  const ml = marginLevel(client.equity, client.margin);
  const rows: [string, React.ReactNode][] = [
    [t("clients.login"), client.login],
    [t("users.email"), client.email],
    [t("clients.group"), client.group],
    [t("groups.leverage"), `1:${client.leverage}`],
    [t("clients.balance"), f.money(client.balance, client.currency)],
    [t("clients.credit"), f.money(client.credit, client.currency)],
    [t("clients.equity"), f.money(client.equity, client.currency)],
    [t("clients.marginLevel"), ml === null ? "—" : `${f.num(ml, 1)}%`],
    [t("clients.lastIp"), client.lastIp],
    [t("common.status"), <StatusBadge key="s" status={client.status} />],
  ];
  const ps = (positions.data ?? []).filter((p) => p.clientId === client.id);

  return (
    <Sheet open onClose={onClose} title={`${t("clients.detail")} — ${client.name}`}>
      <div className="grid gap-5" data-testid="client-drawer">
        <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          {rows.map(([k, v]) => (
            <React.Fragment key={k}>
              <dt className="text-muted-foreground">{k}</dt>
              <dd className="tabular-nums">{v}</dd>
            </React.Fragment>
          ))}
        </dl>

        <section className="grid gap-2">
          <h3 className="text-sm font-semibold">{t("clients.kyc")}</h3>
          <div className="flex items-center gap-2">
            <KycBadge kyc={client.kyc} />
            <Select
              aria-label={t("clients.setKyc")}
              value={client.kyc}
              disabled={!actor.can("clients.edit") || kycMut.isPending}
              onChange={(e) => kycMut.mutate(e.target.value as Client["kyc"])}
            >
              {KycStatus.options.map((k) => <option key={k} value={k}>{k}</option>)}
            </Select>
          </div>
        </section>

        <section className="grid gap-2">
          <h3 className="text-sm font-semibold">{t("clients.group")}</h3>
          <div className="flex items-center gap-2">
            <Select
              aria-label={t("clients.setGroup")}
              value={client.group}
              disabled={!actor.can("clients.edit") || groupMut.isPending}
              onChange={(e) => groupMut.mutate(e.target.value)}
              data-testid="client-group"
            >
              {(groups.data ?? []).map((g) => <option key={g.id} value={g.name}>{g.name} · {g.marginMode === "retail_netting" ? "netting" : "hedging"} · 1:{g.leverage}</option>)}
            </Select>
          </div>
          <p className="text-xs text-muted-foreground">{t("clients.groupHint")}</p>
          {groupMut.error && <p className="text-xs text-red-600 dark:text-red-400">{String((groupMut.error as Error).message ?? groupMut.error)}</p>}
        </section>

        <BalanceOps client={client} />

        <section className="grid gap-2">
          <h3 className="text-sm font-semibold">{t("clients.positions")} ({ps.length})</h3>
          {ps.map((p) => (
            <div key={p.id} className="flex items-center justify-between gap-2 rounded border border-border px-2 py-1.5 text-sm">
              <span className="flex items-center gap-2"><SideBadge side={p.side} /> {p.symbol} <span className="text-muted-foreground">{p.lots}</span> <BookBadge book={p.book} /></span>
              <Pnl value={p.pnl}>{f.money(p.pnl)}</Pnl>
            </div>
          ))}
        </section>

        {subs.length > 0 && (
          <section className="grid gap-1">
            <h3 className="text-sm font-semibold">{t("clients.subs")}</h3>
            {subs.map((s) => (
              <div key={s.id} className="flex justify-between text-sm"><span>#{s.login}</span><span className="tabular-nums">{f.money(s.balance, s.currency)}</span></div>
            ))}
          </section>
        )}
      </div>
    </Sheet>
  );
}

type OpType = "deposit" | "withdraw" | "credit";

function BalanceOps({ client }: { client: Client }) {
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
