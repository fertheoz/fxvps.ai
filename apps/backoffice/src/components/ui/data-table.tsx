"use client";
import * as React from "react";
import {
  flexRender, getCoreRowModel, getFilteredRowModel, getPaginationRowModel, getSortedRowModel, useReactTable,
  type ColumnDef, type ColumnFiltersState, type FilterFn, type RowSelectionState, type SortingState, type VisibilityState,
} from "@tanstack/react-table";
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Columns3, Filter, X } from "lucide-react";
import { Button, Input } from "./primitives";
import { cn } from "@/lib/utils";
import { useT } from "@/lib/hooks";

declare module "@tanstack/react-table" {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData, TValue> {
    /** Start hidden; the user turns it on in Columns and the choice is remembered. */
    defaultHidden?: boolean;
  }
}

const PAGE_SIZES = [15, 30, 50, 100, 200, 500];

function readPageSize(key: string | undefined, fallback: number): number {
  if (!key) return fallback;
  try {
    const raw = localStorage.getItem(`dt:${key}:pageSize`);
    const n = raw ? Number(raw) : NaN;
    return PAGE_SIZES.includes(n) ? n : fallback;
  } catch {
    return fallback;
  }
}

/** Columns holding a timestamp (ISO string) get a from/to filter instead of a text one. */
const isDateColumn = (id: string) => /(^|[a-z])(at|time|date)$/i.test(id) || /^(at|time|date)$/i.test(id);

type DateRange = { from?: string; to?: string };

/** Case-insensitive "contains" on the cell's string form (login numbers, symbols, names…). */
const textFilter: FilterFn<unknown> = (row, id, value: string) => {
  const v = row.getValue(id);
  if (v === null || v === undefined) return false;
  return String(v).toLowerCase().includes(value.toLowerCase());
};

/** Inclusive from/to on an ISO timestamp (values from `datetime-local` inputs, local time). */
const dateRangeFilter: FilterFn<unknown> = (row, id, value: DateRange) => {
  const raw = row.getValue(id);
  const ts = typeof raw === "string" || typeof raw === "number" ? new Date(raw).getTime() : NaN;
  if (Number.isNaN(ts)) return false;
  if (value.from && ts < new Date(value.from).getTime()) return false;
  if (value.to && ts > new Date(value.to).getTime()) return false;
  return true;
};

const columnId = (c: ColumnDef<unknown, unknown>): string => c.id ?? (c as { accessorKey?: string }).accessorKey ?? "";

function readVisibility(key: string | undefined): VisibilityState | null {
  if (!key) return null;
  try {
    const raw = localStorage.getItem(`dt:${key}:cols`);
    return raw ? (JSON.parse(raw) as VisibilityState) : null;
  } catch {
    return null;
  }
}

export function DataTable<T>({
  data, columns, onRowClick, pageSize = 15, toolbar, selectable, onSelectionChange, getRowId, searchable = true, testId, renderDetail, storageKey,
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
  /** Rows expand on click into this panel (full detail of the row under the tab's heading). */
  renderDetail?: (row: T) => React.ReactNode;
  /** Remembers column visibility and page size under this key (defaults to testId). */
  storageKey?: string;
}) {
  const key = storageKey ?? testId;
  const t = useT();
  const [sorting, setSorting] = React.useState<SortingState>([]);
  const [filter, setFilter] = React.useState("");
  const [columnFilters, setColumnFilters] = React.useState<ColumnFiltersState>([]);
  const [columnVisibility, setColumnVisibility] = React.useState<VisibilityState>(() => {
    const saved = readVisibility(key);
    if (saved) return saved;
    const hidden: VisibilityState = {};
    for (const c of columns) {
      const id = columnId(c as ColumnDef<unknown, unknown>);
      if (id && c.meta?.defaultHidden) hidden[id] = false;
    }
    return hidden;
  });
  const [size, setSize] = React.useState<number>(() => readPageSize(key, pageSize));
  const [rowSelection, setRowSelection] = React.useState<RowSelectionState>({});
  /** Column whose inline filter editor is open (double-click on its header). */
  const [editing, setEditing] = React.useState<string | null>(null);
  const [colsOpen, setColsOpen] = React.useState(false);
  const [open, setOpen] = React.useState<Record<string, boolean>>({});

  React.useEffect(() => {
    if (!key) return;
    try {
      localStorage.setItem(`dt:${key}:cols`, JSON.stringify(columnVisibility));
    } catch {
      /* private mode */
    }
  }, [columnVisibility, key]);
  React.useEffect(() => {
    if (!key) return;
    try {
      localStorage.setItem(`dt:${key}:pageSize`, String(size));
    } catch {
      /* private mode */
    }
  }, [size, key]);

  const cols = React.useMemo<ColumnDef<T>[]>(() => {
    // Every column gets a filter: a date range on timestamp columns, "contains" elsewhere.
    const withFilters = columns.map((c) => ({
      ...c,
      filterFn: c.filterFn ?? (isDateColumn(columnId(c as ColumnDef<unknown, unknown>)) ? "dateRange" : "text"),
    })) as ColumnDef<T>[];
    const withDetail: ColumnDef<T>[] = renderDetail
      ? [
          {
            id: "_detail",
            header: () => null,
            cell: ({ row }) => (open[row.id] ? <ChevronDown className="h-4 w-4 text-muted-foreground" /> : <ChevronRight className="h-4 w-4 text-muted-foreground" />),
            enableSorting: false,
            enableColumnFilter: false,
            enableHiding: false,
          },
          ...withFilters,
        ]
      : withFilters;
    if (!selectable) return withDetail;
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
        enableColumnFilter: false,
        enableHiding: false,
      },
      ...withDetail,
    ];
  }, [columns, selectable, renderDetail, open]);

  // eslint-disable-next-line react-hooks/incompatible-library
  const table = useReactTable({
    data, columns: cols, getRowId,
    state: { sorting, globalFilter: filter, columnFilters, columnVisibility, rowSelection },
    filterFns: { text: textFilter as FilterFn<T>, dateRange: dateRangeFilter as FilterFn<T> },
    onSortingChange: setSorting,
    onGlobalFilterChange: setFilter,
    onColumnFiltersChange: setColumnFilters,
    onColumnVisibilityChange: setColumnVisibility,
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
    initialState: { pagination: { pageSize: size } },
    autoResetPageIndex: false,
  });
  React.useEffect(() => {
    table.setPageSize(size);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size]);

  const headerText = (id: string) => {
    const h = table.getColumn(id)?.columnDef.header;
    return typeof h === "string" ? h : id;
  };
  const activeFilters = columnFilters.filter((f) => {
    const v = f.value as string | DateRange | undefined;
    return typeof v === "string" ? v !== "" : !!(v && (v.from || v.to));
  });
  const describe = (v: unknown) => {
    if (typeof v === "string") return v;
    const r = v as DateRange;
    return `${r.from ? r.from.replace("T", " ") : "…"} → ${r.to ? r.to.replace("T", " ") : "…"}`;
  };

  const rows = table.getRowModel().rows;
  // Density follows the number of visible columns: roomy when few, tight when many.
  const ncols = table.getVisibleLeafColumns().length;
  const pad = ncols <= 8 ? "px-3" : ncols <= 14 ? "px-2" : "px-1";
  const textCls = ncols <= 14 ? "text-sm" : "text-[13px]";
  return (
    <div className="grid gap-2" data-testid={testId}>
      {(searchable || toolbar) && (
        <div className="flex flex-wrap items-center gap-2">
          {searchable && <Input placeholder={t("common.search")} value={filter} onChange={(e) => setFilter(e.target.value)} className="max-w-xs" />}
          {toolbar}
          <div className="relative ml-auto">
            <Button size="sm" variant="outline" onClick={() => setColsOpen((o) => !o)} data-testid="dt-columns" title={t("common.columns")}>
              <Columns3 className="h-4 w-4" />
              {t("common.columns")}
            </Button>
            {colsOpen && (
              <div className="absolute right-0 z-20 mt-1 grid min-w-[12rem] gap-1 rounded-md border border-border bg-card p-2 text-sm shadow-lg" onMouseLeave={() => setColsOpen(false)}>
                {table.getAllLeafColumns().filter((c) => c.getCanHide()).map((c) => (
                  <label key={c.id} className="flex items-center gap-2 px-1 py-0.5">
                    <input type="checkbox" checked={c.getIsVisible()} onChange={c.getToggleVisibilityHandler()} />
                    {headerText(c.id)}
                  </label>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
      {activeFilters.length > 0 && (
        <div className="flex flex-wrap items-center gap-1 text-xs" data-testid="dt-filters">
          <Filter className="h-3 w-3 text-muted-foreground" />
          {activeFilters.map((f) => (
            <button key={f.id} className="inline-flex items-center gap-1 rounded-full border border-border bg-muted/50 px-2 py-0.5 hover:bg-muted" onClick={() => setEditing(f.id)} title={t("common.editFilter")}>
              <span className="text-muted-foreground">{headerText(f.id)}:</span> {describe(f.value)}
              <X
                className="h-3 w-3"
                onClick={(e) => {
                  e.stopPropagation();
                  table.getColumn(f.id)?.setFilterValue(undefined);
                }}
              />
            </button>
          ))}
          <button className="px-1 text-muted-foreground hover:text-foreground" onClick={() => table.resetColumnFilters()}>{t("common.clearFilters")}</button>
        </div>
      )}
      <div className="overflow-x-auto rounded-lg border border-border bg-card">
        <table className={cn("w-full", textCls)}>
          <thead className="bg-muted/50 text-xs text-muted-foreground">
            {table.getHeaderGroups().map((hg) => (
              <tr key={hg.id}>
                {hg.headers.map((h) => {
                  const canFilter = h.column.getCanFilter() && !h.column.id.startsWith("_");
                  const canSort = h.column.getCanSort() && !h.column.id.startsWith("_");
                  const value = h.column.getFilterValue();
                  const active = typeof value === "string" ? value !== "" : !!(value && ((value as DateRange).from || (value as DateRange).to));
                  const sorted = h.column.getIsSorted();
                  return (
                    <th
                      key={h.id}
                      className={cn("relative whitespace-nowrap py-1.5 text-left font-medium", pad, active && "text-foreground")}
                      title={canFilter || canSort ? t("common.clickFilter") : undefined}
                    >
                      {h.isPlaceholder ? null : canFilter || canSort ? (
                        <span className="inline-flex items-center gap-1">
                          <button
                            className="inline-flex items-center gap-1 cursor-pointer"
                            onClick={() => setEditing(editing === h.column.id ? null : h.column.id)}
                            data-testid={`dt-head-${h.column.id}`}
                          >
                            {flexRender(h.column.columnDef.header, h.getContext())}
                            {active && <Filter className="h-3 w-3 text-primary" />}
                          </button>
                          {canSort && (
                            <button
                              className={cn("cursor-pointer rounded px-0.5", sorted ? "text-foreground" : "text-muted-foreground/60 hover:text-foreground")}
                              onClick={(e) => {
                                e.stopPropagation();
                                h.column.toggleSorting(sorted === "asc");
                              }}
                              title={t(sorted === "asc" ? "common.sortDesc" : "common.sortAsc")}
                              aria-label={t("common.sort")}
                            >
                              {sorted === "asc" ? <ArrowUp className="h-3 w-3" /> : sorted === "desc" ? <ArrowDown className="h-3 w-3" /> : <ArrowUp className="h-3 w-3 opacity-50" />}
                            </button>
                          )}
                        </span>
                      ) : flexRender(h.column.columnDef.header, h.getContext())}
                      {editing === h.column.id && (
                        <div className="absolute left-0 top-full z-20 mt-1 grid gap-1 rounded-md border border-border bg-card p-2 shadow-lg" data-testid={`dt-filter-${h.column.id}`}>
                          {canSort && (
                            <div className="flex gap-1 text-[11px]">
                              <button className={cn("rounded border border-border px-1.5 py-0.5", sorted === "asc" && "bg-primary/10 text-primary")} onClick={() => h.column.toggleSorting(false)}>↑ {t("common.sortAsc")}</button>
                              <button className={cn("rounded border border-border px-1.5 py-0.5", sorted === "desc" && "bg-primary/10 text-primary")} onClick={() => h.column.toggleSorting(true)}>↓ {t("common.sortDesc")}</button>
                              {sorted && <button className="px-1 text-muted-foreground hover:text-foreground" onClick={() => h.column.clearSorting()}>{t("common.sortNone")}</button>}
                            </div>
                          )}
                          {canFilter && (isDateColumn(h.column.id) ? (
                            <>
                              <label className="grid gap-0.5 text-[11px]">
                                {t("common.from")}
                                <input type="datetime-local" className="rounded border border-border bg-background px-1 py-0.5 text-xs" value={(value as DateRange | undefined)?.from ?? ""} onChange={(e) => h.column.setFilterValue({ ...((value as DateRange) ?? {}), from: e.target.value })} />
                              </label>
                              <label className="grid gap-0.5 text-[11px]">
                                {t("common.to")}
                                <input type="datetime-local" className="rounded border border-border bg-background px-1 py-0.5 text-xs" value={(value as DateRange | undefined)?.to ?? ""} onChange={(e) => h.column.setFilterValue({ ...((value as DateRange) ?? {}), to: e.target.value })} />
                              </label>
                            </>
                          ) : (
                            <input
                              autoFocus
                              className="w-40 rounded border border-border bg-background px-1 py-0.5 text-xs"
                              placeholder={t("common.filterValue")}
                              value={(value as string | undefined) ?? ""}
                              onChange={(e) => h.column.setFilterValue(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter" || e.key === "Escape") setEditing(null);
                              }}
                            />
                          ))}
                          <div className="flex justify-between gap-2 text-[11px]">
                            <button className="text-muted-foreground hover:text-foreground" onClick={() => h.column.setFilterValue(undefined)}>{t("common.clear")}</button>
                            <button className="text-primary" onClick={() => setEditing(null)}>{t("common.done")}</button>
                          </div>
                        </div>
                      )}
                    </th>
                  );
                })}
              </tr>
            ))}
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr><td colSpan={cols.length} className="px-3 py-8 text-center text-muted-foreground">{t("common.noResults")}</td></tr>
            )}
            {rows.map((row) => (
              <React.Fragment key={row.id}>
                <tr
                  onClick={renderDetail || onRowClick ? () => {
                    if (renderDetail) setOpen((o) => ({ ...o, [row.id]: !o[row.id] }));
                    onRowClick?.(row.original);
                  } : undefined}
                  className={cn("border-t border-border", (onRowClick || renderDetail) && "cursor-pointer hover:bg-muted/40", row.getIsSelected() && "bg-primary/5", open[row.id] && "bg-muted/30")}
                  data-testid={renderDetail ? `dt-row-${row.id}` : undefined}
                >
                  {row.getVisibleCells().map((cell) => (
                    <td key={cell.id} className={cn("whitespace-nowrap py-1", pad)}>{flexRender(cell.column.columnDef.cell, cell.getContext())}</td>
                  ))}
                </tr>
                {renderDetail && open[row.id] && (
                  <tr className="border-t border-border bg-muted/20" data-testid={`dt-detail-${row.id}`}>
                    <td colSpan={row.getVisibleCells().length} className="px-4 py-3">{renderDetail(row.original)}</td>
                  </tr>
                )}
              </React.Fragment>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span className="flex items-center gap-2">
          {table.getFilteredRowModel().rows.length} {t("common.rows")}
          <label className="flex items-center gap-1">
            {t("common.perPage")}
            <select className="rounded border border-border bg-background px-1 py-0.5 text-xs" value={size} onChange={(e) => setSize(Number(e.target.value))} data-testid="dt-page-size">
              {PAGE_SIZES.map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </label>
        </span>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={() => table.previousPage()} disabled={!table.getCanPreviousPage()}>‹</Button>
          <span>{table.getState().pagination.pageIndex + 1} / {Math.max(1, table.getPageCount())}</span>
          <Button size="sm" variant="outline" onClick={() => table.nextPage()} disabled={!table.getCanNextPage()}>›</Button>
        </div>
      </div>
    </div>
  );
}
