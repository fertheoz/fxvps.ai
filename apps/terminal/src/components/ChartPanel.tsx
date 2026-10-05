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
import { TIMEFRAMES, TIMEFRAME_SECONDS, type Bar } from '@fxvps/trading-core';
import { getApi, trade } from '../store/api';
import { selectOrders, selectPositions, useTerminal, type Indicators } from '../store/terminal';
import { useT } from '../hooks';
import { applyTick } from '@fxvps/trading-core';
import { bollinger, ema, rsi, sma } from '@fxvps/trading-core';
import { formatPrice, lotsToVolume, roundPrice, volumeToLots } from '@fxvps/trading-core';
import { isTauri, openChartWindow } from '../native';
import { dragProtection, hitLine } from '../lib/chartDrag';

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
  const positions = useTerminal(selectPositions);
  const orders = useTerminal(selectOrders);
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
  const [edit, setEdit] = useState<{ id: string; price: number } | null>(null);
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
    const ensure = (key: keyof IndicatorSeries, on: boolean, color: string, pane = 0, style: LineStyle = LineStyle.Solid) => {
      if (on && !r[key]) r[key] = chart.addSeries(LineSeries, { color, lineWidth: 1, lineStyle: style, priceLineVisible: false, lastValueVisible: false, crosshairMarkerVisible: false }, pane);
      if (!on && r[key]) {
        chart.removeSeries(r[key]!);
        delete r[key];
      }
    };
    ensure('sma', ind.sma, '#f59e0b');
    ensure('ema', ind.ema, '#8b5cf6');
    ensure('bbU', ind.bollinger, '#06b6d4', 0, LineStyle.Dashed);
    ensure('bbM', ind.bollinger, '#06b6d4', 0, LineStyle.Dotted);
    ensure('bbL', ind.bollinger, '#06b6d4', 0, LineStyle.Dashed);
    ensure('rsi', ind.rsi, '#ec4899', 1);
    r.sma?.setData(lineData(bars, sma(closes, 20)));
    r.ema?.setData(lineData(bars, ema(closes, 50)));
    if (ind.bollinger) {
      const bb = bollinger(closes, 20, 2);
      r.bbU?.setData(lineData(bars, bb.map((p) => p?.upper ?? null)));
      r.bbM?.setData(lineData(bars, bb.map((p) => p?.middle ?? null)));
      r.bbL?.setData(lineData(bars, bb.map((p) => p?.lower ?? null)));
    }
    r.rsi?.setData(lineData(bars, rsi(closes, 14)));
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
  }, [indicators, loading]);

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
      const time = ts(bar.time);
      const s = lastOf(sma(closes, 20));
      if (r.sma && s != null) r.sma.update({ time, value: s });
      const e = lastOf(ema(bars.map((b) => b.close), 50));
      if (r.ema && e != null) r.ema.update({ time, value: e });
      const b = lastOf(bollinger(closes, 20, 2));
      if (b) {
        r.bbU?.update({ time, value: b.upper });
        r.bbM?.update({ time, value: b.middle });
        r.bbL?.update({ time, value: b.lower });
      }
      const rv = lastOf(rsi(closes, 14));
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
    for (const o of orders) {
      if (o.symbol !== spec.name) continue;
      add(o.price, '#f59e0b', `${o.side.toUpperCase()} ${o.type.replace('_', ' ').toUpperCase()} ${volumeToLots(o.volume)}`, LineStyle.Dotted);
      if (o.limitPrice !== undefined) add(o.limitPrice, '#f59e0b', `LMT #${o.id}`, LineStyle.SparseDotted);
    }
  }, [positions, orders, spec, loading, theme]);

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

  // The movable line: a pending order picked up by a long press, else the caller's draft.
  const editOrder = edit ? orders.find((o) => o.id === edit.id) : undefined;
  const line: DraftLine | undefined =
    edit && editOrder && spec
      ? {
          price: edit.price,
          label: `${editOrder.side.toUpperCase()} ${editOrder.type.replace('_', ' ').toUpperCase()} ${volumeToLots(editOrder.volume)} ${formatPrice(edit.price, spec.digits)}`,
          tone: editOrder.side === 'buy' ? 'up' : 'down',
          onMove: (price) => setEdit({ id: edit.id, price }),
        }
      : draft;
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
    // Moving a line must not scroll the chart; Esc puts it back; the press timer dies with the panel.
    chartRef.current?.applyOptions({ handleScroll: !edit, handleScale: !edit });
    const esc = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') setEdit(null);
    };
    if (edit) window.addEventListener('keydown', esc);
    return () => {
      window.removeEventListener('keydown', esc);
      if (pressRef.current) clearTimeout(pressRef.current.timer);
    };
  }, [edit]);
  const dragDraft = (e: React.PointerEvent<HTMLDivElement>) => {
    const el = host.current;
    const series = candleRef.current;
    if (!line || !spec || !el || !series || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    const p = series.coordinateToPrice(e.clientY - el.getBoundingClientRect().top);
    if (p !== null && p > 0) line.onMove(roundPrice(p, spec.digits));
  };
  /** Releasing a moved pending order sends the new trigger price. */
  const dropEdit = async () => {
    const e = edit;
    setEdit(null);
    if (!e || !editOrder || !activeAccountId || e.price === editOrder.price) return;
    const r = await getApi().modifyOrder(activeAccountId, e.id, { price: e.price });
    if (!r.ok) toast('error', t('toast.rejected', { error: r.error ?? '' }));
  };
  /** The pending order whose line is within reach of `clientY`. */
  const orderAt = (clientY: number) => {
    const series = candleRef.current;
    const el = host.current;
    if (edit || draft || !series || !el || !spec) return undefined;
    const y = clientY - el.getBoundingClientRect().top;
    return orders.find((o) => {
      if (o.symbol !== spec.name) return false;
      const c = series.priceToCoordinate(o.price);
      return c !== null && Math.abs(c - y) <= PRESS_HIT_PX;
    });
  };
  /** Double-click (desktop) picks a pending order line up. */
  const pickDouble = (e: React.MouseEvent<HTMLDivElement>) => {
    const hit = orderAt(e.clientY);
    if (hit) setEdit({ id: hit.id, price: hit.price });
  };
  /** Long press on a pending order line picks it up (touch and mouse). */
  const pressStart = (e: React.PointerEvent<HTMLDivElement>) => {
    const hit = orderAt(e.clientY);
    if (!hit) return;
    const timer = setTimeout(() => {
      pressRef.current = null;
      if (typeof navigator !== 'undefined' && 'vibrate' in navigator) navigator.vibrate(15);
      setEdit({ id: hit.id, price: hit.price });
    }, PRESS_MS);
    pressRef.current = { timer, x: e.clientX, y: e.clientY };
  };
  const pressMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const p = pressRef.current;
    if (p && Math.hypot(e.clientX - p.x, e.clientY - p.y) > PRESS_MOVE_PX) pressEnd();
  };
  const pressEnd = () => {
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
          className="absolute inset-0"
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
        {line && (
          <div
            ref={draftEl}
            className="absolute left-0 right-0 z-20 flex items-center touch-none cursor-ns-resize select-none"
            style={{ height: DRAFT_HANDLE }}
            onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
            onPointerMove={dragDraft}
            onPointerUp={() => void dropEdit()}
            onPointerCancel={() => void dropEdit()}
            data-testid={edit ? `chart-move-${index}` : `chart-draft-${index}`}
          >
            <span className={`h-7 px-3 ml-2 rounded-full grid place-items-center text-[12px] font-semibold text-white shadow-lg ${line.tone === 'up' ? 'bg-up' : 'bg-down'}`}>
              ↕ {line.label}
            </span>
            <span className={`flex-1 border-t-2 border-dashed ${line.tone === 'up' ? 'border-up' : 'border-down'}`} />
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
