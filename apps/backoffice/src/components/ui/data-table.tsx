"use client";
import * as React from "react";
import {
  flexRender, getCoreRowModel, getFilteredRowModel, getPaginationRowModel, getSortedRowModel, useReactTable,
  type ColumnDef, type RowSelectionState, type SortingState,
} from "@tanstack/react-table";
import { ArrowDown, ArrowUp } from "lucide-react";
import { Button, Input } from "./primitives";
import { cn } from "@/lib/utils";
import { useT } from "@/lib/hooks";

export function DataTable<T>({
  data, columns, onRowClick, pageSize = 15, toolbar, selectable, onSelectionChange, getRowId, searchable = true, testId,
}: {
  data: T[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  columns: ColumnDef<T, any>[];
  onRowClick?: (row: T) => void;
  pageSize?: number;
  toolbar?: React.ReactNode;
  selectable?: boolean;
  onSelectionChange?: (rows: T[]) => void;
  getRowId?: (row: T) => string;
  searchable?: boolean;
  testId?: string;
}) {
  const t = useT();
  const [sorting, setSorting] = React.useState<SortingState>([]);
  const [filter, setFilter] = React.useState("");
  const [rowSelection, setRowSelection] = React.useState<RowSelectionState>({});

  const cols = React.useMemo<ColumnDef<T>[]>(() => {
    if (!selectable) return columns;
    return [
      {
        id: "_select",
        header: ({ table }) => (
          <input type="checkbox" aria-label="select all" checked={table.getIsAllPageRowsSelected()} onChange={table.getToggleAllPageRowsSelectedHandler()} />
        ),
        cell: ({ row }) => (
          <input type="checkbox" aria-label="select row" checked={row.getIsSelected()} onClick={(e) => e.stopPropagation()} onChange={row.getToggleSelectedHandler()} />
        ),
        enableSorting: false,
      },
      ...columns,
    ];
  }, [columns, selectable]);

  // eslint-disable-next-line react-hooks/incompatible-library
  const table = useReactTable({
    data, columns: cols, getRowId,
    state: { sorting, globalFilter: filter, rowSelection },
    onSortingChange: setSorting,
    onGlobalFilterChange: setFilter,
    onRowSelectionChange: (updater) => {
      const next = typeof updater === "function" ? updater(rowSelection) : updater;
      setRowSelection(next);
      if (onSelectionChange) onSelectionChange(data.filter((d, i) => next[getRowId ? getRowId(d) : String(i)]));
    },
    enableRowSelection: !!selectable,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: { pagination: { pageSize } },
    autoResetPageIndex: false,
  });

  const rows = table.getRowModel().rows;
  return (
    <div className="grid gap-2" data-testid={testId}>
      {(searchable || toolbar) && (
        <div className="flex flex-wrap items-center gap-2">
          {searchable && <Input placeholder={t("common.search")} value={filter} onChange={(e) => setFilter(e.target.value)} className="max-w-xs" />}
          {toolbar}
        </div>
      )}
      <div className="overflow-x-auto rounded-lg border border-border bg-card">
        <table className="w-full text-sm">
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((h) => (
                  <th key={h.id} className="whitespace-nowrap px-3 py-2 text-left font-medium">
                    {h.isPlaceholder ? null : h.column.getCanSort() ? (
                      <button className="inline-flex items-center gap-1 cursor-pointer" onClick={h.column.getToggleSortingHandler()}>
                        {flexRender(h.column.columnDef.header, h.getContext())}
                        {h.column.getIsSorted() === "asc" ? <ArrowUp className="h-3 w-3" /> : h.column.getIsSorted() === "desc" ? <ArrowDown className="h-3 w-3" /> : null}
                      </button>
                    ) : flexRender(h.column.columnDef.header, h.getContext())}
                  </th>
                ))}
              </tr>
            ))}
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr><td colSpan={cols.length} className="px-3 py-8 text-center text-muted-foreground">{t("common.noResults")}</td></tr>
            )}
            {rows.map((row) => (
              <tr
                key={row.id}
                onClick={onRowClick ? () => onRowClick(row.original) : undefined}
                className={cn("border-t border-border", onRowClick && "cursor-pointer hover:bg-muted/40", row.getIsSelected() && "bg-primary/5")}
              >
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="whitespace-nowrap px-3 py-1.5">{flexRender(cell.column.columnDef.cell, cell.getContext())}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{table.getFilteredRowModel().rows.length} {t("common.rows")}</span>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={() => table.previousPage()} disabled={!table.getCanPreviousPage()}>‹</Button>
          <span>{table.getState().pagination.pageIndex + 1} / {Math.max(1, table.getPageCount())}</span>
          <Button size="sm" variant="outline" onClick={() => table.nextPage()} disabled={!table.getCanNextPage()}>›</Button>
        </div>
      </div>
    </div>
  );
}
