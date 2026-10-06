import type { IChartApi, ISeriesApi, ISeriesPrimitive, IPrimitivePaneRenderer, IPrimitivePaneView, SeriesAttachedParameter, SeriesType, Time, UTCTimestamp } from 'lightweight-charts';

type RenderTarget = Parameters<IPrimitivePaneRenderer['draw']>[0];
import type { ChartShape } from '@fxvps/trading-core';

/**
 * Trend lines, rectangles and Fibonacci retracements drawn over the candles (a series primitive).
 * Times are UTC seconds; points outside the loaded bars are projected with the
 * timeframe length, so a line keeps its slope when it runs into the future.
 */

export interface ShapeGeometry {
  /** Last bar time (s) and index of the loaded bars. */
  lastTime: number;
  lastIndex: number;
  tfSeconds: number;
}

/** x for a time, also right of the last bar (null = chart not ready). */
export function xAtTime(chart: IChartApi, g: ShapeGeometry, time: number): number | null {
  const ts = chart.timeScale();
  const direct = ts.timeToCoordinate(time as UTCTimestamp);
  if (direct !== null) return direct;
  if (!g.tfSeconds) return null;
  const logical = g.lastIndex + (time - g.lastTime) / g.tfSeconds;
  return ts.logicalToCoordinate(logical as never);
}

/** time (s) for an x, also right of the last bar. */
export function timeAtX(chart: IChartApi, g: ShapeGeometry, x: number): number | null {
  const ts = chart.timeScale();
  const direct = ts.coordinateToTime(x) as Time | null;
  if (typeof direct === 'number') return direct;
  const logical = ts.coordinateToLogical(x);
  if (logical === null || !g.tfSeconds) return null;
  return g.lastTime + (Number(logical) - g.lastIndex) * g.tfSeconds;
}

interface Px {
  shape: ChartShape;
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** Distance from a point to a segment (px). */
function segmentDistance(px: number, py: number, x1: number, y1: number, x2: number, y2: number): number {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / len2));
  return Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy));
}

/** Retracement levels, as a share of the a→b move. */
export const FIB_LEVELS = [0, 0.236, 0.382, 0.5, 0.618, 0.786, 1] as const;

/** Level lines across the drawn span (price at the right edge), plus the faint a→b diagonal. */
function drawFib(ctx: CanvasRenderingContext2D, p: Px, width: number, color: string, digits: number): void {
  const left = Math.min(p.x1, p.x2);
  const right = Math.max(p.x1, p.x2);
  ctx.save();
  ctx.setLineDash([3, 3]);
  ctx.globalAlpha = 0.5;
  ctx.beginPath();
  ctx.moveTo(p.x1, p.y1);
  ctx.lineTo(p.x2, p.y2);
  ctx.stroke();
  ctx.restore();
  ctx.font = '10px ui-sans-serif, system-ui, sans-serif';
  ctx.textBaseline = 'bottom';
  ctx.textAlign = 'left';
  for (const l of FIB_LEVELS) {
    const y = p.y1 + (p.y2 - p.y1) * l;
    const price = p.shape.a.price + (p.shape.b.price - p.shape.a.price) * l;
    ctx.lineWidth = l === 0 || l === 1 ? 1.5 : 1;
    ctx.beginPath();
    ctx.moveTo(left, y);
    ctx.lineTo(right, y);
    ctx.stroke();
    ctx.fillStyle = color;
    ctx.fillText(`${(l * 100).toFixed(1)}%  ${price.toFixed(digits)}`, Math.min(right + 4, width - 90), y - 1);
  }
}

export class ShapesPrimitive implements ISeriesPrimitive<Time> {
  private shapes: ChartShape[] = [];
  private selected: string | null = null;
  private geometry: ShapeGeometry = { lastTime: 0, lastIndex: 0, tfSeconds: 0 };
  private chart: IChartApi | null = null;
  private series: ISeriesApi<SeriesType> | null = null;
  private requestUpdate: (() => void) | null = null;
  private placed: Px[] = [];
  private colors = { trend: '#4c8dff', rect: '#4c8dff', fib: '#4c8dff', selected: '#f59e0b' };
  private digits = 5;
  private readonly view: IPrimitivePaneView = {
    zOrder: () => 'normal',
    renderer: (): IPrimitivePaneRenderer => ({
      draw: (target: RenderTarget) => {
        const placed = this.placed;
        const colors = this.colors;
        const selected = this.selected;
        target.useMediaCoordinateSpace((scope) => {
          const ctx = scope.context;
          for (const p of placed) {
            const sel = p.shape.id === selected;
            ctx.lineWidth = sel ? 2 : 1.5;
            ctx.strokeStyle = sel ? colors.selected : colors[p.shape.kind];
            if (p.shape.kind === 'fib') {
              drawFib(ctx, p, scope.mediaSize.width, sel ? colors.selected : colors.fib, this.digits);
            } else if (p.shape.kind === 'rect') {
              const x = Math.min(p.x1, p.x2);
              const y = Math.min(p.y1, p.y2);
              const w = Math.abs(p.x2 - p.x1);
              const h = Math.abs(p.y2 - p.y1);
              ctx.fillStyle = (sel ? colors.selected : colors.rect) + '22';
              ctx.fillRect(x, y, w, h);
              ctx.strokeRect(x, y, w, h);
            } else {
              ctx.beginPath();
              ctx.moveTo(p.x1, p.y1);
              ctx.lineTo(p.x2, p.y2);
              ctx.stroke();
            }
            if (sel) {
              ctx.fillStyle = colors.selected;
              for (const [x, y] of [
                [p.x1, p.y1],
                [p.x2, p.y2],
              ] as const) {
                ctx.beginPath();
                ctx.arc(x, y, 4, 0, Math.PI * 2);
                ctx.fill();
              }
            }
          }
        });
      },
    }),
  };

  attached(param: SeriesAttachedParameter<Time>): void {
    this.chart = param.chart;
    this.series = param.series;
    this.requestUpdate = param.requestUpdate;
  }

  detached(): void {
    this.chart = null;
    this.series = null;
    this.requestUpdate = null;
  }

  setTheme(accent: string): void {
    if (this.colors.trend === accent) return;
    this.colors = { ...this.colors, trend: accent, rect: accent, fib: accent };
    this.requestUpdate?.();
  }

  /** New shape list / selection / bar geometry; redraws. */
  update(shapes: ChartShape[], selected: string | null, geometry: ShapeGeometry, digits = 5): void {
    this.shapes = shapes;
    this.selected = selected;
    this.geometry = geometry;
    this.digits = digits;
    this.requestUpdate?.();
  }

  updateAllViews(): void {
    const chart = this.chart;
    const series = this.series;
    if (!chart || !series) {
      this.placed = [];
      return;
    }
    const out: Px[] = [];
    for (const s of this.shapes) {
      const x1 = xAtTime(chart, this.geometry, s.a.time);
      const x2 = xAtTime(chart, this.geometry, s.b.time);
      const y1 = series.priceToCoordinate(s.a.price);
      const y2 = series.priceToCoordinate(s.b.price);
      if (x1 === null || x2 === null || y1 === null || y2 === null) continue;
      out.push({ shape: s, x1, y1, x2, y2 });
    }
    this.placed = out;
  }

  paneViews(): readonly IPrimitivePaneView[] {
    return [this.view];
  }

  /** The shape under (x, y) in chart-media pixels, if any. */
  shapeAt(x: number, y: number, tolerance = 12): ChartShape | undefined {
    this.updateAllViews();
    for (const p of [...this.placed].reverse()) {
      if (p.shape.kind === 'trend') {
        if (segmentDistance(x, y, p.x1, p.y1, p.x2, p.y2) <= tolerance) return p.shape;
      } else if (p.shape.kind === 'fib') {
        // the diagonal, or any level line within the drawn time span
        if (segmentDistance(x, y, p.x1, p.y1, p.x2, p.y2) <= tolerance) return p.shape;
        const inX = x >= Math.min(p.x1, p.x2) - tolerance && x <= Math.max(p.x1, p.x2) + tolerance;
        if (inX && FIB_LEVELS.some((l) => Math.abs(y - (p.y1 + (p.y2 - p.y1) * l)) <= tolerance)) return p.shape;
      } else {
        const inX = x >= Math.min(p.x1, p.x2) - tolerance && x <= Math.max(p.x1, p.x2) + tolerance;
        const inY = y >= Math.min(p.y1, p.y2) - tolerance && y <= Math.max(p.y1, p.y2) + tolerance;
        if (inX && inY) return p.shape;
      }
    }
    return undefined;
  }
}
