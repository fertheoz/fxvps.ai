"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { Check, Minus } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Dialog, PageHeader } from "@/components/ui/primitives";
import { SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { AdminUser } from "@/lib/schemas";
import { PERMISSIONS, ROLES, can } from "@/lib/rbac";

const col = createColumnHelper<AdminUser>();

export default function UsersPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const { data = [] } = useApiQuery("listUsers");
  const [editing, setEditing] = React.useState<AdminUser | null>(null);
  const columns = [
    col.accessor("name", { header: t("clients.name"), cell: (c) => <span className="font-medium">{c.getValue()}</span> }),
    col.accessor("email", { header: t("users.email") }),
    col.accessor("role", { header: t("common.role"), cell: (c) => <Badge tone="info">{c.getValue()}</Badge> }),
    col.accessor("mfa", { header: t("users.mfa"), cell: (c) => c.getValue() ? <Badge tone="success">on</Badge> : <Badge tone="danger">off</Badge> }),
    col.accessor("active", { header: t("users.active"), cell: (c) => c.getValue() ? "✓" : "—" }),
    col.accessor("lastLogin", { header: t("users.lastLogin"), cell: (c) => { const v = c.getValue(); return v ? f.date(v) : "—"; } }),
  ];
  const editable = actor.can("users.edit");
  return (
    <div data-testid="page-users">
      <PageHeader title={t("users.title")}>
        {editable && <Button onClick={() => setEditing({ id: `u${Date.now()}`, name: "", email: "", role: "readonly", mfa: true, active: true, lastLogin: null })}>{t("users.add")}</Button>}
      </PageHeader>
      <DataTable data={data} columns={columns} getRowId={(u) => u.id} onRowClick={editable ? setEditing : undefined} searchable={false} />
      <Card className="mt-4">
        <CardHeader><CardTitle>{t("users.matrix")}</CardTitle></CardHeader>
        <CardContent className="overflow-x-auto p-0">
          <table className="w-full text-xs" data-testid="perm-matrix">
            <thead className="bg-muted/50 text-muted-foreground">
              <tr><th className="px-3 py-2 text-left font-medium">Permission</th>{ROLES.map((r) => <th key={r} className="px-3 py-2 font-medium">{r}</th>)}</tr>
            </thead>
            <tbody>
              {PERMISSIONS.map((p) => (
                <tr key={p} className="border-t border-border">
                  <td className="px-3 py-1 font-mono">{p}</td>
                  {ROLES.map((r) => (
                    <td key={r} className="px-3 py-1 text-center">
                      {can(r, p) ? <Check className="mx-auto h-3.5 w-3.5 text-emerald-500" /> : <Minus className="mx-auto h-3.5 w-3.5 text-muted-foreground/40" />}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </CardContent>
      </Card>
      {editing && <UserDialog key={editing.id} user={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function UserDialog({ user, onClose }: { user: AdminUser; onClose: () => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const { draft, set, errors, validate } = useZodForm(AdminUser, user);
  const mut = useApiMutation((u: AdminUser) => api().saveUser(u, actor), () => { toast(t("common.saved")); onClose(); });
  const save = () => { const v = validate(); if (v) mut.mutate(v); };
  return (
    <Dialog open onClose={onClose} title={user.name || t("users.add")} footer={<>
      <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
      <Button onClick={save} disabled={mut.isPending}>{t("common.save")}</Button>
    </>}>
      <div className="grid gap-3">
        <TextField label={t("clients.name")} value={draft.name} onChange={(v) => set("name", v)} error={errors.name} />
        <TextField label={t("users.email")} value={draft.email} onChange={(v) => set("email", v)} error={errors.email} />
        <SelectField label={t("common.role")} value={draft.role} options={ROLES} onChange={(v) => set("role", v)} />
        <SelectField label={t("users.mfa")} value={draft.mfa ? "on" : "off"} options={["on", "off"] as const} onChange={(v) => set("mfa", v === "on")} />
        <SelectField label={t("users.active")} value={draft.active ? "yes" : "no"} options={["yes", "no"] as const} onChange={(v) => set("active", v === "yes")} />
      </div>
    </Dialog>
  );
}
