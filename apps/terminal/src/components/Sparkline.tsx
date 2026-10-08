import { sparkPath } from '../lib/sparkline';

/** Compact trend line (strategy showcase): green when it ends at or above its start. */
export function Sparkline({ points, width = 120, height = 28, label }: { points: number[]; width?: number; height?: number; label?: string }) {
  const d = sparkPath(points, width, height);
  if (!d) return null;
  const up = (points[points.length - 1] ?? 0) >= (points[0] ?? 0);
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} data-testid="sparkline" className={up ? 'text-up' : 'text-down'}>
      <path d={d} fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinejoin="round" />
    </svg>
  );
}
