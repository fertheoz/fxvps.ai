import { useRef, type ReactNode } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';

interface Props<T> {
  columns: string;
  header: ReactNode;
  rows: T[];
  rowKey: (row: T) => string;
  renderRow: (row: T) => ReactNode;
  empty: ReactNode;
  rowHeight?: number;
  testId?: string;
}

/** Virtualized grid-based table: only visible rows are mounted. */
export function VirtualTable<T>({ columns, header, rows, rowKey, renderRow, empty, rowHeight = 26, testId }: Props<T>) {
  const ref = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({
    count: rows.length,
    getScrollElement: () => ref.current,
    estimateSize: () => rowHeight,
    overscan: 10,
    initialRect: { width: 1000, height: 300 },
  });
  return (
    <div className="flex flex-col h-full min-w-[930px]" role="table" data-testid={testId}>
      <div className="grid items-center px-2 h-6 text-[10px] uppercase text-muted border-b border-line shrink-0" style={{ gridTemplateColumns: columns }} role="row">
        {header}
      </div>
      <div ref={ref} className="flex-1 overflow-auto">
        {rows.length === 0 ? (
          <div className="p-3 text-muted">{empty}</div>
        ) : (
          <div style={{ height: v.getTotalSize(), position: 'relative' }}>
            {v.getVirtualItems().map((it) => {
              const row = rows[it.index]!;
              return (
                <div
                  key={rowKey(row)}
                  role="row"
                  className="grid items-center px-2 whitespace-nowrap border-b border-line/50 hover:bg-hover"
                  style={{ gridTemplateColumns: columns, position: 'absolute', top: 0, left: 0, right: 0, height: rowHeight, transform: `translateY(${it.start}px)` }}
                >
                  {renderRow(row)}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
