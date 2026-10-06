"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { CornerDownRight } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Dialog, FieldError, Input, Label, PageHeader, Select } from "@/components/ui/primitives";
import { KycBadge, marginLevel, StatusBadge } from "@/components/badges";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useRouter } from "next/navigation";
import { useToast } from "@/components/shell/providers";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { KycStatus, type Client } from "@/lib/schemas";

const col = createColumnHelper<Client>();

export default function ClientsPage() {
  const t = useT();
  const f = useFormat();
  const { data = [] } = useApiQuery("listClients", [{}], { live: 5000 });
  const [kyc, setKyc] = React.useState<string>("all");
  const router = useRouter();
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
        onRowClick={(c) => router.push(`/clients/card/?login=${c.login}`)}
        getRowId={(c) => c.id}
        testId="clients-table"
        toolbar={
          <Select value={kyc} onChange={(e) => setKyc(e.target.value)} aria-label={t("clients.kyc")}>
            <option value="all">{t("clients.kyc")}: {t("common.all")}</option>
            {KycStatus.options.map((k) => <option key={k} value={k}>{k}</option>)}
          </Select>
        }
      />
      <OpenAccountDialog open={opening} onClose={() => setOpening(false)} onOpened={() => setOpening(false)} />
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
