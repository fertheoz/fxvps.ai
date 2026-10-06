import { useEffect, useRef, useState } from 'react';
import {
  CandlestickSeries,
  ColorType,
  CrosshairMode,
  HistogramSeries,
  LineSeries,
  LineStyle,
  createChart,
  type IChartApi,
  type IPriceLine,
  type ISeriesApi,
  type UTCTimestamp,
} from 'lightweight-charts';
import { TIMEFRAMES, TIMEFRAME_SECONDS, type Bar, type Position } from '@fxvps/trading-core';
import { getApi, trade } from '../store/api';
import { selectOrders, selectPositions, useTerminal, type Indicators } from '../store/terminal';
import { useT } from '../hooks';
import { applyTick } from '@fxvps/trading-core';
import { bollinger, ema, rsi, sma } from '@fxvps/trading-core';
import { formatPrice, lotsToVolume, roundPrice, volumeToLots } from '@fxvps/trading-core';
import { isTauri, openChartWindow } from '../native';
import { dragProtection, hitLine } from '../lib/chartDrag';
import { ShapesPrimitive, timeAtX, type ShapeGeometry } from '../lib/chartShapes';
import type { ChartShape } from '@fxvps/trading-core';

type Line = ISeriesApi<'Line'>;

/**
 * Lightweight Charts™ (Apache 2.0) asks for the attribution notice and a link to
 * tradingview.com on a page available to users; the on-chart logo is one way to
 * satisfy it, the About / shortcuts screens are ours (see `ChartAttribution`).
 */
export function ChartAttribution({ className = '' }: { className?: string }) {
  return (
    <p className={`text-[11px] text-muted ${className}`} data-testid="chart-attribution">
      Charts:{' '}
      <a href="https://www.tradingview.com/" target="_blank" rel="noreferrer" className="underline hover:text-fg">
        TradingView Lightweight Charts™
      </a>{' '}
      © TradingView, Inc.
    </p>
  );
}

/** Height of the draft line's touch target (px). */
const DRAFT_HANDLE = 44;
/** Press-and-hold on a pending order line for this long to start moving it. */
const PRESS_MS = 450;
/** Pointer tolerance for hitting an order line / cancelling the press (px). */
const PRESS_HIT_PX = 18;
const PRESS_MOVE_PX = 8;

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#888';
}

const ts = (t: number) => t as UTCTimestamp;

interface IndicatorSeries {
  sma?: Line;
  ema?: Line;
  bbU?: Line;
  bbM?: Line;
  bbL?: Line;
  rsi?: Line;
}

function lineData(bars: Bar[], values: (number | null)[]) {
  const out: { time: UTCTimestamp; value: number }[] = [];
  values.forEach((v, i) => {
    if (v !== null) out.push({ time: ts(bars[i]!.time), value: v });
  });
  return out;
}

/** A price line the user drags to choose an order price (phone: place a limit order from the chart). */
export interface DraftLine {
  price: number;
  label: string;
  tone: 'up' | 'down';
  onMove: (price: number) => void;
}

/**
 * `bare`: canvas only (the mobile shell brings its own symbol / timeframe / trade controls).
 * `draft`: draggable order line drawn over the chart.
 */
export function ChartPanel({ index, detached = false, bare = false, draft }: { index: number; detached?: boolean; bare?: boolean; draft?: DraftLine }) {
  const t = useT();
  const slot = useTerminal((s) => s.charts[index]);
  const spec = useTerminal((s) => (slot ? s.symbols[slot.symbol] : undefined));
  const quote = useTerminal((s) => (slot ? s.quotes[slot.symbol] : undefined));
  const isActive = useTerminal((s) => s.activeChart === index);
  const theme = useTerminal((s) => s.theme);
  const indicators = useTerminal((s) => s.indicators);
  const indicatorSettings = useTerminal((s) => s.indicatorSettings);
  const positions = useTerminal(selectPositions);
  const orders = useTerminal(selectOrders);
  const objects = useTerminal((s) => (s.activeAccountId ? s.objects[s.activeAccountId] : undefined));
  const chartTool = useTerminal((s) => s.chartTool);
  const setChartTool = useTerminal((s) => s.setChartTool);
  const setObjects = useTerminal((s) => s.setObjects);
  const oneClickVolume = useTerminal((s) => s.oneClickVolume);
  const setOneClickVolume = useTerminal((s) => s.setOneClickVolume);
  const setActive = useTerminal((s) => s.setActiveChart);
  const setTf = useTerminal((s) => s.setChartTimeframe);
  const setSym = useTerminal((s) => s.setChartSymbol);
  const symbolOrder = useTerminal((s) => s.symbolOrder);

  const host = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const candleRef = useRef<ISeriesApi<'Candlestick'> | null>(null);
  const volRef = useRef<ISeriesApi<'Histogram'> | null>(null);
  const indRef = useRef<IndicatorSeries>({});
  const barsRef = useRef<Bar[]>([]);
  const linesRef = useRef<IPriceLine[]>([]);
  /** Draggable SL/TP lines of open positions. */
  const protRef = useRef<{ line: IPriceLine; positionId: string; kind: 'sl' | 'tp' }[]>([]);
  const dragRef = useRef<{ line: IPriceLine; positionId: string; kind: 'sl' | 'tp'; price: number } | null>(null);
  const draftEl = useRef<HTMLDivElement>(null);
  /** Pending order being moved after a long press (price = where the line is now). */
  // 'pos': the position's own line was picked up; where it is dropped decides SL or TP.
  const [edit, setEdit] = useState<{ kind: 'order' | 'line' | 'alert' | 'sl' | 'tp' | 'pos'; id: string; price: number } | null>(null);
  /** Shapes layer (trend lines, rectangles) and the one being drawn / moved. */
  const shapesRef = useRef<ShapesPrimitive | null>(null);
  const [drawing, setDrawing] = useState<ChartShape | null>(null);
  const [selectedShape, setSelectedShape] = useState<string | null>(null);
  const shapeDragRef = useRef<{ id: string; lastX: number; lastY: number } | null>(null);
  const showAskLine = useTerminal((s) => s.showAskLine);
  const askLineRef = useRef<IPriceLine | null>(null);
  /** Price of the line being placed with the armed chart tool. */
  const [toolPrice, setToolPrice] = useState<number | null>(null);
  const pressRef = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number } | null>(null);
  const activeAccountId = useTerminal((s) => s.activeAccountId);
  const toast = useTerminal((s) => s.toast);
  const [loadedKey, setLoadedKey] = useState('');
  const [lotsText, setLotsText] = useState(volumeToLots(oneClickVolume));
  const [busy, setBusy] = useState(false);

  // create chart once
  useEffect(() => {
    if (!host.current) return;
    const chart = createChart(host.current, {
      autoSize: true,
      layout: { background: { type: ColorType.Solid, color: 'transparent' }, attributionLogo: false, fontSize: 11 },
      crosshair: { mode: CrosshairMode.Normal },
      timeScale: { timeVisible: true, secondsVisible: false, rightOffset: 6 },
      rightPriceScale: { borderVisible: false },
      localization: { locale: 'en-US' },
    });
    chartRef.current = chart;
    candleRef.current = chart.addSeries(CandlestickSeries, { borderVisible: false });
    const shapes = new ShapesPrimitive();
    candleRef.current.attachPrimitive(shapes);
    shapesRef.current = shapes;
    return () => {
      chart.remove();
      chartRef.current = null;
      candleRef.current = null;
      volRef.current = null;
      indRef.current = {};
      linesRef.current = [];
    };
  }, []);

  const lang = useTerminal((s) => s.lang);
  useEffect(() => {
    // explicit locale: navigator.language can be an invalid BCP-47 tag in some environments
    chartRef.current?.applyOptions({ localization: { locale: lang === 'tr' ? 'tr-TR' : 'en-US' } });
  }, [lang]);

  // theme
  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    const up = cssVar('--up');
    const down = cssVar('--down');
    chart.applyOptions({
      layout: { textColor: cssVar('--muted') },
      grid: { vertLines: { color: cssVar('--border') + '66' }, horzLines: { color: cssVar('--border') + '66' } },
    });
    candleRef.current?.applyOptions({ upColor: up, downColor: down, wickUpColor: up, wickDownColor: down });
  }, [theme]);

  const refreshIndicators = (ind: Indicators) => {
    const chart = chartRef.current;
    if (!chart) return;
    const bars = barsRef.current;
    const closes = bars.map((b) => b.close);
    const r = indRef.current;
    const cfg = useTerminal.getState().indicatorSettings;
    const ensure = (key: keyof IndicatorSeries, on: boolean, color: string, pane = 0, style: LineStyle = LineStyle.Solid) => {
      if (on && !r[key]) r[key] = chart.addSeries(LineSeries, { color, lineWidth: 1, lineStyle: style, priceLineVisible: false, lastValueVisible: false, crosshairMarkerVisible: false }, pane);
      else if (on) r[key]!.applyOptions({ color });
      if (!on && r[key]) {
        chart.removeSeries(r[key]!);
        delete r[key];
      }
    };
    ensure('sma', ind.sma, cfg.sma.color);
    ensure('ema', ind.ema, cfg.ema.color);
    ensure('bbU', ind.bollinger, cfg.bollinger.color, 0, LineStyle.Dashed);
    ensure('bbM', ind.bollinger, cfg.bollinger.color, 0, LineStyle.Dotted);
    ensure('bbL', ind.bollinger, cfg.bollinger.color, 0, LineStyle.Dashed);
    ensure('rsi', ind.rsi, cfg.rsi.color, 1);
    r.sma?.setData(lineData(bars, sma(closes, cfg.sma.period)));
    r.ema?.setData(lineData(bars, ema(closes, cfg.ema.period)));
    if (ind.bollinger) {
      const bb = bollinger(closes, cfg.bollinger.period, cfg.bollinger.dev);
      r.bbU?.setData(lineData(bars, bb.map((p) => p?.upper ?? null)));
      r.bbM?.setData(lineData(bars, bb.map((p) => p?.middle ?? null)));
      r.bbL?.setData(lineData(bars, bb.map((p) => p?.lower ?? null)));
    }
    r.rsi?.setData(lineData(bars, rsi(closes, cfg.rsi.period)));
    if (ind.rsi) {
      const pane = chart.panes()[1];
      pane?.setHeight(90);
    }
    // volume
    if (ind.volume && !volRef.current) {
      volRef.current = chart.addSeries(HistogramSeries, { priceScaleId: 'vol', priceFormat: { type: 'volume' }, priceLineVisible: false, lastValueVisible: false });
      chart.priceScale('vol').applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
    } else if (!ind.volume && volRef.current) {
      chart.removeSeries(volRef.current);
      volRef.current = null;
    }
    if (volRef.current) {
      const up = cssVar('--up') + '55';
      const down = cssVar('--down') + '55';
      volRef.current.setData(bars.map((b) => ({ time: ts(b.time), value: b.volume, color: b.close >= b.open ? up : down })));
    }
  };

  // load bars on symbol/timeframe change
  const symbol = slot?.symbol;
  const timeframe = slot?.timeframe;
  const loading = loadedKey !== `${symbol}|${timeframe}`;
  useEffect(() => {
    if (!symbol || !timeframe || !spec) return;
    let cancelled = false;
    candleRef.current?.applyOptions({ priceFormat: { type: 'price', precision: spec.digits, minMove: 1 / 10 ** spec.digits } });
    void getApi()
      .getBars(symbol, timeframe, 500)
      .then((bars) => {
        if (cancelled || !candleRef.current) return;
        barsRef.current = bars;
        candleRef.current.setData(bars.map((b) => ({ time: ts(b.time), open: b.open, high: b.high, low: b.low, close: b.close })));
        refreshIndicators(useTerminal.getState().indicators);
        chartRef.current?.timeScale().scrollToRealTime();
        setLoadedKey(`${symbol}|${timeframe}`);
      });
    return () => {
      cancelled = true;
    };
  }, [symbol, timeframe, spec]);

  // indicator toggles
  useEffect(() => {
    if (!loading) refreshIndicators(indicators);
  }, [indicators, indicatorSettings, loading]);

  // live ticks -> last bar (store already batches per animation frame)
  useEffect(() => {
    if (!quote || !timeframe || loading || !candleRef.current) return;
    const bars = barsRef.current;
    const last = bars[bars.length - 1];
    const bar = applyTick(last, quote.bid, Math.floor(quote.time / 1000), TIMEFRAME_SECONDS[timeframe]);
    if (last && bar.time < last.time) return;
    if (last && bar.time === last.time) bars[bars.length - 1] = bar;
    else bars.push(bar);
    candleRef.current.update({ time: ts(bar.time), open: bar.open, high: bar.high, low: bar.low, close: bar.close });
    volRef.current?.update({ time: ts(bar.time), value: bar.volume, color: (bar.close >= bar.open ? cssVar('--up') : cssVar('--down')) + '55' });
    const r = indRef.current;
    if (r.sma || r.ema || r.rsi || r.bbM) {
      const closes = bars.slice(-200).map((b) => b.close);
      const lastOf = <T,>(a: T[]) => a[a.length - 1];
      const cfg = useTerminal.getState().indicatorSettings;
      const time = ts(bar.time);
      const s = lastOf(sma(closes, cfg.sma.period));
      if (r.sma && s != null) r.sma.update({ time, value: s });
      const e = lastOf(ema(bars.map((b) => b.close), cfg.ema.period));
      if (r.ema && e != null) r.ema.update({ time, value: e });
      const b = lastOf(bollinger(closes, cfg.bollinger.period, cfg.bollinger.dev));
      if (b) {
        r.bbU?.update({ time, value: b.upper });
        r.bbM?.update({ time, value: b.middle });
        r.bbL?.update({ time, value: b.lower });
      }
      const rv = lastOf(rsi(closes, cfg.rsi.period));
      if (r.rsi && rv != null) r.rsi.update({ time, value: rv });
    }
  }, [quote, timeframe, loading]);

  // position & order lines
  useEffect(() => {
    const series = candleRef.current;
    if (!series || !spec) return;
    for (const l of linesRef.current) series.removePriceLine(l);
    linesRef.current = [];
    protRef.current = [];
    const up = cssVar('--up');
    const down = cssVar('--down');
    const add = (price: number, color: string, title: string, style = LineStyle.Solid) =>
      linesRef.current.push(series.createPriceLine({ price, color, lineWidth: 1, lineStyle: style, axisLabelVisible: true, title }));
    for (const p of positions) {
      if (p.symbol !== spec.name) continue;
      add(p.openPrice, p.side === 'buy' ? up : down, `${p.side.toUpperCase()} ${volumeToLots(p.volume)}`);
      if (p.sl !== undefined) {
        add(p.sl, down, `SL #${p.id}`, LineStyle.Dashed);
        protRef.current.push({ line: linesRef.current[linesRef.current.length - 1]!, positionId: p.id, kind: 'sl' });
      }
      if (p.tp !== undefined) {
        add(p.tp, up, `TP #${p.id}`, LineStyle.Dashed);
        protRef.current.push({ line: linesRef.current[linesRef.current.length - 1]!, positionId: p.id, kind: 'tp' });
      }
    }
    for (const l of objects?.lines ?? []) {
      if (l.symbol === spec.name) add(l.price, cssVar('--accent'), l.note ?? '', LineStyle.Dashed);
    }
    for (const a of objects?.alerts ?? []) {
      if (a.symbol === spec.name) add(a.price, a.firedAt ? cssVar('--muted') : '#f59e0b', a.firedAt ? '🔔 ✓' : '🔔', LineStyle.LargeDashed);
    }
    for (const o of orders) {
      if (o.symbol !== spec.name) continue;
      add(o.price, '#f59e0b', `${o.side.toUpperCase()} ${o.type.replace('_', ' ').toUpperCase()} ${volumeToLots(o.volume)}`, LineStyle.Dotted);
      if (o.limitPrice !== undefined) add(o.limitPrice, '#f59e0b', `LMT #${o.id}`, LineStyle.SparseDotted);
    }
  }, [positions, orders, objects, spec, loading, theme]);

  // Drag SL/TP lines -> modifyPosition (server-side protection).
  const lineAt = (clientY: number) => {
    const series = candleRef.current;
    const el = host.current;
    if (!series || !el) return undefined;
    const y = clientY - el.getBoundingClientRect().top;
    const lines = protRef.current.flatMap((l) => {
      const c = series.priceToCoordinate(l.line.options().price);
      return c === null ? [] : [{ ...l, y: c }];
    });
    return hitLine(lines, y);
  };
  const onPointerDown = (e: React.MouseEvent) => {
    const hit = lineAt(e.clientY);
    if (!hit || !spec) return;
    e.preventDefault();
    e.stopPropagation();
    const chart = chartRef.current;
    chart?.applyOptions({ handleScroll: false, handleScale: false });
    dragRef.current = { line: hit.line, positionId: hit.positionId, kind: hit.kind, price: hit.line.options().price };
    const move = (ev: MouseEvent) => {
      const d = dragRef.current;
      const el = host.current;
      if (!d || !el || !candleRef.current) return;
      const p = candleRef.current.coordinateToPrice(ev.clientY - el.getBoundingClientRect().top);
      if (p === null) return;
      d.price = p;
      d.line.applyOptions({ price: p, title: `${d.kind.toUpperCase()} #${d.positionId} ${formatPrice(p, spec.digits)}` });
    };
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      chart?.applyOptions({ handleScroll: true, handleScale: true });
      const d = dragRef.current;
      dragRef.current = null;
      if (!d) return;
      const st = useTerminal.getState();
      const acc = st.activeAccountId;
      const pos = acc ? (st.positions[acc] ?? []).find((x) => x.id === d.positionId) : undefined;
      const q = st.quotes[spec.name];
      const prot = pos && q ? dragProtection(pos, d.kind, d.price, spec.digits, q) : undefined;
      const restore = () => d.line.applyOptions({ price: pos?.[d.kind] ?? d.price, title: `${d.kind.toUpperCase()} #${d.positionId}` });
      if (!acc || !pos || !prot) {
        restore();
        if (pos) st.toast('error', t('toast.rejected', { error: `invalid ${d.kind.toUpperCase()}` }));
        return;
      }
      void getApi()
        .modifyPosition(acc, pos.id, prot.sl, prot.tp, prot.trailing)
        .then((r) => {
          if (!r.ok) {
            restore();
            st.toast('error', t('toast.rejected', { error: r.error }));
          }
        });
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  const onHover = (e: React.MouseEvent<HTMLDivElement>) => {
    if (dragRef.current) return;
    e.currentTarget.style.cursor = lineAt(e.clientY) ? 'ns-resize' : '';
  };

  // Shapes layer: the account's trend lines / rectangles of this symbol plus the one being drawn.
  const shapeGeometry = (): ShapeGeometry => {
    const bars = barsRef.current;
    const last = bars[bars.length - 1];
    return { lastTime: last?.time ?? 0, lastIndex: Math.max(0, bars.length - 1), tfSeconds: timeframe ? TIMEFRAME_SECONDS[timeframe] : 0 };
  };
  useEffect(() => {
    const layer = shapesRef.current;
    if (!layer || !spec) return;
    layer.setTheme(cssVar('--accent'));
    const list = (objects?.shapes ?? []).filter((x) => x.symbol === spec.name);
    layer.update(drawing ? [...list, drawing] : list, drawing?.id ?? selectedShape, shapeGeometry(), spec?.digits ?? 5);
  });
  const drawTool = chartTool === 'trend' || chartTool === 'rect' || chartTool === 'fib';
  /** Point (time s, price) under a pointer, projected beyond the last bar when needed. */
  const pointAt = (e: { clientX: number; clientY: number }) => {
    const chart = chartRef.current;
    const series = candleRef.current;
    const el = host.current;
    if (!chart || !series || !el || !spec) return null;
    const r = el.getBoundingClientRect();
    const time = timeAtX(chart, shapeGeometry(), e.clientX - r.left);
    const price = series.coordinateToPrice(e.clientY - r.top);
    return time === null || price === null ? null : { time: Math.round(time), price: roundPrice(price, spec.digits) };
  };
  const shapeDown = (e: React.PointerEvent<HTMLDivElement>): boolean => {
    if (!drawTool || !isActive || !spec || chartTool === null) return false;
    const p = pointAt(e);
    if (!p) return true;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrawing({ id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, symbol: spec.name, kind: chartTool as 'trend' | 'rect' | 'fib', a: p, b: p });
    return true;
  };
  const shapeMove = (e: React.PointerEvent<HTMLDivElement>): boolean => {
    if (drawing) {
      const p = pointAt(e);
      if (p) setDrawing({ ...drawing, b: p });
      return true;
    }
    const d = shapeDragRef.current;
    if (d && objects && activeAccountId) {
      // Move the whole shape by the pointer delta (time and price).
      const chart = chartRef.current;
      const series = candleRef.current;
      const el = host.current;
      if (!chart || !series || !el) return true;
      const r = el.getBoundingClientRect();
      const g = shapeGeometry();
      const t0 = timeAtX(chart, g, d.lastX - r.left);
      const t1 = timeAtX(chart, g, e.clientX - r.left);
      const p0 = series.coordinateToPrice(d.lastY - r.top);
      const p1 = series.coordinateToPrice(e.clientY - r.top);
      if (t0 === null || t1 === null || p0 === null || p1 === null) return true;
      const dt = Math.round(t1 - t0);
      const dp = p1 - p0;
      shapeDragRef.current = { ...d, lastX: e.clientX, lastY: e.clientY };
      setObjects(
        activeAccountId,
        { ...objects, shapes: objects.shapes.map((x) => (x.id === d.id ? { ...x, a: { time: x.a.time + dt, price: x.a.price + dp }, b: { time: x.b.time + dt, price: x.b.price + dp } } : x)) },
        false,
      );
      return true;
    }
    return false;
  };
  const shapeUp = (): boolean => {
    if (drawing) {
      const s = drawing;
      setDrawing(null);
      setChartTool(null);
      if (activeAccountId && (s.a.time !== s.b.time || s.a.price !== s.b.price)) {
        const cur = objects ?? { lines: [], alerts: [], shapes: [] };
        setObjects(activeAccountId, { ...cur, shapes: [...(cur.shapes ?? []), s] });
        setSelectedShape(s.id);
      }
      return true;
    }
    if (shapeDragRef.current) {
      shapeDragRef.current = null;
      if (activeAccountId && objects) setObjects(activeAccountId, objects); // persist the moved shape
      return true;
    }
    return false;
  };
  const shapeAt = (e: { clientX: number; clientY: number }) => {
    const el = host.current;
    if (!el) return undefined;
    const r = el.getBoundingClientRect();
    return shapesRef.current?.shapeAt(e.clientX - r.left, e.clientY - r.top);
  };

  // Ask line (the candles and the last-price marker follow the bid).
  useEffect(() => {
    const series = candleRef.current;
    if (!series) return;
    if (!showAskLine || !quote || !spec) {
      if (askLineRef.current) series.removePriceLine(askLineRef.current);
      askLineRef.current = null;
      return;
    }
    if (askLineRef.current) askLineRef.current.applyOptions({ price: quote.ask });
    else askLineRef.current = series.createPriceLine({ price: quote.ask, color: cssVar('--muted'), lineWidth: 1, lineStyle: LineStyle.SparseDotted, axisLabelVisible: true, title: 'ask' });
  }, [showAskLine, quote, spec, loading]);

  // The movable line: an object picked up by a long press, a tool being placed, else the caller's draft.
  const editOrder = edit?.kind === 'order' ? orders.find((o) => o.id === edit.id) : undefined;
  const editObject = edit && (edit.kind === 'line' || edit.kind === 'alert') ? (edit.kind === 'line' ? objects?.lines : objects?.alerts)?.find((x) => x.id === edit.id) : undefined;
  const editPosition = edit && (edit.kind === 'sl' || edit.kind === 'tp' || edit.kind === 'pos') ? positions.find((p) => p.id === edit.id) : undefined;
  /** Dragging the position line: above the open price is TP for a buy and SL for a sell (and the reverse below). */
  const posLeg = (p: Position, price: number): 'sl' | 'tp' | null =>
    price === p.openPrice ? null : (price > p.openPrice) === (p.side === 'buy') ? 'tp' : 'sl';
  const editLeg = edit?.kind === 'pos' && editPosition ? posLeg(editPosition, edit.price) : edit?.kind === 'sl' || edit?.kind === 'tp' ? edit.kind : null;
  // An armed tool starts its line at the market (toolPrice follows the drag).
  const toolArmed = (chartTool === 'hline' || chartTool === 'alert') && isActive && !!spec && !!quote;
  const toolLinePrice = toolArmed ? (toolPrice ?? roundPrice(quote!.bid, spec!.digits)) : null;
  let line: DraftLine | undefined = draft;
  if (edit && spec && (editOrder || editObject || editPosition)) {
    const label = editOrder
      ? `${editOrder.side.toUpperCase()} ${editOrder.type.replace('_', ' ').toUpperCase()} ${volumeToLots(editOrder.volume)}`
      : editPosition
        ? `${(editLeg ?? 'sl/tp').toUpperCase()} #${editPosition.id}`
        : edit.kind === 'line'
          ? t('obj.hline')
          : t('obj.alert');
    line = {
      price: edit.price,
      label: `${label} ${formatPrice(edit.price, spec.digits)}`,
      tone: editOrder ? (editOrder.side === 'buy' ? 'up' : 'down') : editLeg === 'sl' ? 'down' : 'up',
      onMove: (price) => setEdit({ ...edit, price }),
    };
  } else if (toolLinePrice !== null && spec) {
    line = {
      price: toolLinePrice,
      label: `${chartTool === 'hline' ? t('obj.hline') : t('obj.alert')} ${formatPrice(toolLinePrice, spec.digits)}`,
      tone: 'up',
      onMove: setToolPrice,
    };
  }
  /** Confirms the armed tool: the line becomes a stored object of the active account. */
  const placeTool = () => {
    if (!chartTool || !spec || toolLinePrice === null || !activeAccountId || !quote) return;
    const cur = objects ?? { lines: [], alerts: [], shapes: [] };
    const id = `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;
    const next =
      chartTool === 'hline'
        ? { ...cur, lines: [...cur.lines, { id, symbol: spec.name, price: toolLinePrice }] }
        : {
            ...cur,
            alerts: [...cur.alerts, { id, symbol: spec.name, price: toolLinePrice, direction: toolLinePrice >= quote.bid ? ('above' as const) : ('below' as const), createdAt: Date.now() }],
          };
    setObjects(activeAccountId, next);
    setChartTool(null);
    setToolPrice(null);
  };
  const cancelTool = () => {
    setChartTool(null);
    setToolPrice(null);
  };
  // The line follows the price scale: placed after every render (ticks re-render the panel).
  useEffect(() => {
    const el = draftEl.current;
    const series = candleRef.current;
    if (!el || !series || !line) return;
    const y = series.priceToCoordinate(line.price);
    el.style.display = y === null ? 'none' : '';
    if (y !== null) el.style.top = `${y - DRAFT_HANDLE / 2}px`;
  });
  useEffect(() => {
    // Moving or placing a line must not scroll the chart; Esc puts it back / cancels the tool.
    const hold = !!edit || (!!chartTool && isActive);
    chartRef.current?.applyOptions({ handleScroll: !hold, handleScale: !hold });
    const esc = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape') return;
      setEdit(null);
      setChartTool(null);
      setToolPrice(null);
    };
    if (hold) window.addEventListener('keydown', esc);
    return () => {
      window.removeEventListener('keydown', esc);
      if (pressRef.current) clearTimeout(pressRef.current.timer);
    };
  }, [edit, chartTool, isActive, setChartTool]);
  const dragDraft = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = host.current;
    const series = candleRef.current;
    if (!line || !spec || !el || !series || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const p = series.coordinateToPrice(e.clientY - el.getBoundingClientRect().top);
    if (p !== null && p > 0) line.onMove(roundPrice(p, spec.digits));
  };
  /** Releasing a moved line: a pending order gets its new trigger price, an object is re-saved. */
  const dropEdit = async () => {
    const e = edit;
    setEdit(null);
    if (!e || !activeAccountId) return;
    if (e.kind === 'order') {
      if (!editOrder || e.price === editOrder.price) return;
      const r = await getApi().modifyOrder(activeAccountId, e.id, { price: e.price });
      if (!r.ok) toast('error', t('toast.rejected', { error: r.error ?? '' }));
      return;
    }
    if (e.kind === 'sl' || e.kind === 'tp' || e.kind === 'pos') {
      // Same rules as the desktop mouse drag: the server keeps the other leg.
      if (!editPosition || !quote || !spec) return;
      const leg = e.kind === 'pos' ? posLeg(editPosition, e.price) : e.kind;
      if (!leg) return; // dropped back on the position line
      const prot = dragProtection(editPosition, leg, e.price, spec.digits, quote);
      if (!prot) return toast('error', t('toast.rejected', { error: `invalid ${leg.toUpperCase()}` }));
      const r = await getApi().modifyPosition(activeAccountId, editPosition.id, prot.sl, prot.tp, prot.trailing);
      if (!r.ok) toast('error', t('toast.rejected', { error: r.error ?? '' }));
      return;
    }
    if (!objects || !quote) return;
    if (e.kind === 'line') setObjects(activeAccountId, { ...objects, lines: objects.lines.map((l) => (l.id === e.id ? { ...l, price: e.price } : l)) });
    else
      setObjects(activeAccountId, {
        ...objects,
        alerts: objects.alerts.map((a) => (a.id === e.id ? { ...a, price: e.price, direction: e.price >= quote.bid ? 'above' : 'below', firedAt: undefined } : a)),
      });
  };
  /** The movable line (pending order, drawn line, alert) within reach of `clientY`. */
  const lineObjectAt = (clientY: number): { kind: 'order' | 'line' | 'alert' | 'sl' | 'tp' | 'pos'; id: string; price: number } | undefined => {
    const series = candleRef.current;
    const el = host.current;
    if (edit || draft || chartTool || !series || !el || !spec) return undefined;
    const y = clientY - el.getBoundingClientRect().top;
    const near = (price: number) => {
      const c = series.priceToCoordinate(price);
      return c !== null && Math.abs(c - y) <= PRESS_HIT_PX;
    };
    for (const p of positions) {
      if (p.symbol !== spec.name) continue;
      if (p.sl !== undefined && near(p.sl)) return { kind: 'sl', id: p.id, price: p.sl };
      if (p.tp !== undefined && near(p.tp)) return { kind: 'tp', id: p.id, price: p.tp };
    }
    // The position line itself: drag it away to set the leg on that side.
    const pos = positions.find((p) => p.symbol === spec.name && near(p.openPrice));
    if (pos) return { kind: 'pos', id: pos.id, price: pos.openPrice };
    const o = orders.find((x) => x.symbol === spec.name && near(x.price));
    if (o) return { kind: 'order', id: o.id, price: o.price };
    const a = objects?.alerts.find((x) => x.symbol === spec.name && near(x.price));
    if (a) return { kind: 'alert', id: a.id, price: a.price };
    const l = objects?.lines.find((x) => x.symbol === spec.name && near(x.price));
    if (l) return { kind: 'line', id: l.id, price: l.price };
    return undefined;
  };
  /** Double-click (desktop) picks a line up or selects a shape. */
  const pickDouble = (e: React.MouseEvent<HTMLDivElement>) => {
    const hit = lineObjectAt(e.clientY);
    if (hit) return setEdit(hit);
    setSelectedShape(shapeAt(e)?.id ?? null);
  };
  /** Long press on a line or shape picks it up (touch and mouse). */
  const pressStart = (e: React.PointerEvent<HTMLDivElement>) => {
    if (shapeDown(e)) return;
    const hit = lineObjectAt(e.clientY);
    if (!hit) {
      const shape = shapeAt(e);
      if (!shape || edit || draft) {
        setSelectedShape(null);
        return;
      }
      const timer = setTimeout(() => {
        pressRef.current = null;
        if (typeof navigator !== 'undefined' && 'vibrate' in navigator) navigator.vibrate(15);
        setSelectedShape(shape.id);
        shapeDragRef.current = { id: shape.id, lastX: e.clientX, lastY: e.clientY };
        chartRef.current?.applyOptions({ handleScroll: false, handleScale: false });
      }, PRESS_MS);
      pressRef.current = { timer, x: e.clientX, y: e.clientY };
      return;
    }
    const timer = setTimeout(() => {
      pressRef.current = null;
      if (typeof navigator !== 'undefined' && 'vibrate' in navigator) navigator.vibrate(15);
      setEdit(hit);
    }, PRESS_MS);
    pressRef.current = { timer, x: e.clientX, y: e.clientY };
  };
  const pressMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (shapeMove(e)) return;
    const p = pressRef.current;
    if (p && Math.hypot(e.clientX - p.x, e.clientY - p.y) > PRESS_MOVE_PX) pressEnd();
  };
  const pressEnd = () => {
    if (shapeUp()) {
      chartRef.current?.applyOptions({ handleScroll: true, handleScale: true });
      return;
    }
    if (pressRef.current) clearTimeout(pressRef.current.timer);
    pressRef.current = null;
  };

  if (!slot) return null;

  const oneClick = async (side: 'buy' | 'sell') => {
    const v = lotsToVolume(lotsText);
    if (v === null || !spec) return;
    setOneClickVolume(v);
    setBusy(true);
    try {
      await trade.market(spec.name, side, v);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className={`relative flex flex-col h-full bg-panel ${isActive ? 'outline outline-1 outline-accent/60 -outline-offset-1' : ''}`}
      onMouseDown={() => setActive(index)}
      data-testid={`chart-${index}`}
    >
      <div className={`${bare ? 'hidden' : 'flex'} items-center gap-1 h-8 px-2 border-b border-line shrink-0`}>
        <select
          aria-label={t('ticket.symbol')}
          className="bg-transparent font-semibold text-[13px] pr-1"
          value={slot.symbol}
          onChange={(e) => setSym(e.target.value, index)}
        >
          {symbolOrder.map((s) => (
            <option key={s}>{s}</option>
          ))}
        </select>
        <div className="flex ml-1">
          {TIMEFRAMES.map((tf) => (
            <button
              key={tf}
              className={`px-1.5 py-0.5 rounded text-[11px] ${tf === slot.timeframe ? 'bg-accent text-white' : 'text-muted hover:text-fg'}`}
              onClick={() => setTf(tf, index)}
            >
              {tf}
            </button>
          ))}
        </div>
        {isTauri() && !detached && (
          <button
            className="ml-auto px-1.5 py-0.5 rounded text-[11px] border border-line text-muted hover:text-fg"
            title={t('chart.detach')}
            data-testid={`chart-detach-${index}`}
            onClick={() => void openChartWindow(slot.symbol, slot.timeframe)}
          >
            ⧉ {t('chart.detach')}
          </button>
        )}
      </div>
      <div className="relative flex-1 min-h-0">
        <div
          ref={host}
          className={`absolute inset-0 ${drawTool && isActive ? 'touch-none cursor-crosshair' : ''}`}
          onMouseDownCapture={onPointerDown}
          onMouseMove={onHover}
          onDoubleClick={pickDouble}
          onPointerDown={pressStart}
          onPointerMove={pressMove}
          onPointerUp={pressEnd}
          onPointerCancel={pressEnd}
          data-testid={`chart-canvas-${index}`}
        />
        {loading && <div className="absolute inset-0 grid place-items-center text-muted">{t('chart.loading')}</div>}
        {drawTool && isActive && !drawing && (
          <div className="absolute top-2 left-1/2 -translate-x-1/2 z-10 px-3 h-8 rounded-full bg-accent text-white text-[12px] font-medium grid place-items-center shadow-lg pointer-events-none" data-testid={`chart-draw-hint-${index}`}>
            {chartTool === 'trend' ? t('obj.trend') : t('obj.rect')} · {t('obj.drawHint')}
          </div>
        )}
        {line && (
          <div
            ref={draftEl}
            className="absolute left-0 right-0 z-20 flex items-center touch-none cursor-ns-resize select-none"
            style={{ height: DRAFT_HANDLE }}
            onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
            onPointerMove={dragDraft}
            onPointerUp={() => void dropEdit()}
            onPointerCancel={() => void dropEdit()}
            data-testid={edit ? `chart-move-${index}` : chartTool ? `chart-tool-${index}` : `chart-draft-${index}`}
          >
            <span className={`h-7 px-3 ml-2 rounded-full grid place-items-center text-[12px] font-semibold text-white shadow-lg ${chartTool ? 'bg-accent' : line.tone === 'up' ? 'bg-up' : 'bg-down'}`}>
              ↕ {line.label}
            </span>
            <span className={`flex-1 border-t-2 border-dashed ${chartTool ? 'border-accent' : line.tone === 'up' ? 'border-up' : 'border-down'}`} />
            {chartTool && !edit && (
              <span className="flex gap-1 mr-2" onPointerDown={(e) => e.stopPropagation()}>
                <button className="h-9 px-3 rounded-full bg-accent text-white text-[13px] font-semibold shadow-lg" onClick={placeTool} data-testid={`chart-tool-place-${index}`}>
                  ✓ {t('obj.place')}
                </button>
                <button className="h-9 w-9 rounded-full bg-panel border border-line text-muted shadow-lg" onClick={cancelTool} aria-label={t('tb.cancel')}>
                  ✕
                </button>
              </span>
            )}
          </div>
        )}
        {spec && quote && !bare && (
          <div className="absolute top-2 left-2 z-10 flex items-stretch rounded-md overflow-hidden shadow-lg border border-line text-[12px] select-none" data-testid={`oneclick-${index}`}>
            <button
              disabled={busy}
              onClick={() => void oneClick('sell')}
              className="flex flex-col items-start px-3 py-1 bg-down text-white hover:brightness-110 disabled:opacity-60"
              data-testid={`oneclick-sell-${index}`}
            >
              <span className="text-[10px] opacity-80">{t('chart.sell')}</span>
              <span className="num font-semibold">{formatPrice(quote.bid, spec.digits)}</span>
            </button>
            <label className="flex flex-col items-center justify-center bg-panel px-1">
              <span className="text-[9px] text-muted">{t('chart.lots')}</span>
              <input
                aria-label={t('chart.lots')}
                value={lotsText}
                onChange={(e) => setLotsText(e.target.value)}
                className="num w-12 text-center bg-transparent outline-none"
                inputMode="decimal"
              />
            </label>
            <button
              disabled={busy}
              onClick={() => void oneClick('buy')}
              className="flex flex-col items-end px-3 py-1 bg-up text-white hover:brightness-110 disabled:opacity-60"
              data-testid={`oneclick-buy-${index}`}
            >
              <span className="text-[10px] opacity-80">{t('chart.buy')}</span>
              <span className="num font-semibold">{formatPrice(quote.ask, spec.digits)}</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
