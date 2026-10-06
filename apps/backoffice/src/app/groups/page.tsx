"use client";
import * as React from "react";
import { createColumnHelper } from "@tanstack/react-table";
import { DataTable } from "@/components/ui/data-table";
import { Button, Dialog, FieldError, PageHeader } from "@/components/ui/primitives";
import { BookBadge } from "@/components/badges";
import { NumField, SelectField, TextField, useZodForm } from "@/components/form";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery } from "@/lib/queries";
import { useActor, useFormat, useT } from "@/lib/hooks";
import { Book, CommissionType, EsmaCap, Group, MarginMode } from "@/lib/schemas";

const col = createColumnHelper<Group>();

export default function GroupsPage() {
  const t = useT();
  const f = useFormat();
  const actor = useActor();
  const { data = [] } = useApiQuery("listGroups");
  const [editing, setEditing] = React.useState<Group | null>(null);

  const commission = (g: Group) =>
    g.commissionType === "per_lot" ? `${f.money(g.commissionValue, g.currency)} / lot`
      : g.commissionType === "per_million" ? `${f.money(g.commissionValue, g.currency)} / 1M`
        : `${g.commissionValue} bp`;

  const columns = [
    col.accessor("name", { header: "Name", cell: (c) => <span className="font-medium">{c.getValue()}</span> }),
    col.accessor("currency", { header: t("groups.currency") }),
    col.accessor("leverage", { header: t("groups.leverage"), cell: (c) => `1:${c.getValue()}` }),
    col.accessor("esma", { header: "ESMA", cell: (c) => (c.getValue() === "none" ? "—" : c.getValue()) }),
    col.accessor("marginMode", { header: t("groups.marginMode") }),
    col.accessor("marginCallPct", { header: t("groups.marginCall"), cell: (c) => `${c.getValue()}%` }),
    col.accessor("stopOutPct", { header: t("groups.stopOut"), cell: (c) => `${c.getValue()}%` }),
    col.display({ id: "comm", header: t("groups.commission"), cell: (c) => commission(c.row.original) }),
    col.accessor("markupPoints", { header: t("groups.markup") }),
    col.accessor("book", { header: t("groups.book"), cell: (c) => <BookBadge book={c.getValue()} /> }),
  ];

  return (
    <div data-testid="page-groups">
      <PageHeader title={t("groups.title")} />
      <DataTable data={data} columns={columns} onRowClick={actor.can("groups.edit") ? setEditing : undefined} getRowId={(g) => g.id} />
      {editing && <GroupDialog key={editing.id} group={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

function GroupDialog({ group, onClose }: { group: Group; onClose: () => void }) {
  const t = useT();
  const actor = useActor();
  const toast = useToast();
  const { draft, set, errors, validate } = useZodForm(Group, group);
  const mut = useApiMutation((g: Group) => api().saveGroup(g, actor), () => { toast(t("common.saved")); onClose(); });
  const save = () => { const v = validate(); if (v) mut.mutate(v); };
  return (
    <Dialog open wide onClose={onClose} title={`${t("groups.editTitle")}: ${group.name}`} footer={<>
      <Button variant="outline" onClick={onClose}>{t("common.cancel")}</Button>
      <Button onClick={save} disabled={mut.isPending}>{t("common.save")}</Button>
    </>}>
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField label="Name" value={draft.name} onChange={(v) => set("name", v)} error={errors.name} />
        <NumField label={t("groups.leverage")} value={draft.leverage} onChange={(v) => set("leverage", v)} error={errors.leverage} step={1} />
        <SelectField label={t("groups.marginMode")} value={draft.marginMode} options={MarginMode.options} onChange={(v) => set("marginMode", v)} />
        <SelectField label={t("groups.esma")} value={draft.esma} options={EsmaCap.options} onChange={(v) => set("esma", v)} />
        <SelectField label={t("groups.book")} value={draft.book} options={Book.options} onChange={(v) => set("book", v)} />
        <NumField label={t("groups.marginCall")} value={draft.marginCallPct} onChange={(v) => set("marginCallPct", v)} error={errors.marginCallPct} />
        <NumField label={t("groups.stopOut")} value={draft.stopOutPct} onChange={(v) => set("stopOutPct", v)} error={errors.stopOutPct} />
        <SelectField label={`${t("groups.commission")} type`} value={draft.commissionType} options={CommissionType.options} onChange={(v) => set("commissionType", v)} />
        <NumField label={`${t("groups.commission")} (minor units / bp)`} value={draft.commissionValue} onChange={(v) => set("commissionValue", v)} error={errors.commissionValue} step={1} />
        <NumField label={t("groups.markup")} value={draft.markupPoints} onChange={(v) => set("markupPoints", v)} error={errors.markupPoints} step={1} />
        <NumField label={t("groups.swapMult")} value={draft.swapMultiplier} onChange={(v) => set("swapMultiplier", v)} error={errors.swapMultiplier} step={0.1} />
      </div>
      <FieldError msg={errors._} />
    </Dialog>
  );
}
