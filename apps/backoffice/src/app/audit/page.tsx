"use client";
import { createColumnHelper } from "@tanstack/react-table";
import { Lock } from "lucide-react";
import { DataTable } from "@/components/ui/data-table";
import { Badge, PageHeader } from "@/components/ui/primitives";
import { useApiQuery } from "@/lib/queries";
import { useFormat, useT } from "@/lib/hooks";
import type { AuditEntry } from "@/lib/schemas";

const col = createColumnHelper<AuditEntry>();

export default function AuditPage() {
  const t = useT();
  const f = useFormat();
  const { data = [] } = useApiQuery("listAudit", [], { live: 5000 });
  const columns = [
    col.accessor("at", { header: t("audit.at"), cell: (c) => <span className="tabular-nums">{f.date(c.getValue())}</span> }),
    col.accessor("actor", { header: t("audit.actor") }),
    col.accessor("role", { header: t("common.role"), cell: (c) => <Badge tone="muted">{c.getValue()}</Badge> }),
    col.accessor("action", { header: t("audit.action"), cell: (c) => <span className="font-mono text-xs">{c.getValue()}</span> }),
    col.accessor("target", { header: t("audit.target") }),
    col.accessor("details", { header: t("audit.details"), cell: (c) => <span className="block max-w-md truncate" title={c.getValue()}>{c.getValue()}</span> }),
  ];
  return (
    <div data-testid="page-audit">
      <PageHeader title={t("audit.title")}>
        <span className="flex items-center gap-1 text-xs text-muted-foreground"><Lock className="h-3 w-3" />{t("audit.immutable")}</span>
      </PageHeader>
      <DataTable data={data} columns={columns} getRowId={(a) => a.id} pageSize={20} testId="audit-table" />
    </div>
  );
}
