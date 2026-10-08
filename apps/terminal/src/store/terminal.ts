import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import type {
  Account,
  ChartObjects,
  ChartTemplate,
  ConnectionState,
  Deal,
  JournalEntry,
  OrderHistoryEntry,
  OrderType,
  PendingOrder,
  Position,
  Quote,
  Side,
  SymbolSpec,
  Timeframe,
  TradingEvent,
} from '@fxvps/trading-core';
import { translate, type Lang } from '../i18n';
import { DEFAULT_MW_COLUMNS, type MwColumnId } from '../components/mwColumns';

export type Theme = 'dark' | 'light';
/** Charts on screen at once; 6 is the ceiling (every chart costs the client CPU and memory). */
export type ChartLayout = 1 | 2 | 4 | 6;
export type ChartStyle = 'candles' | 'heikin' | 'renko' | 'bars' | 'line' | 'area';
/** Up/down colours: the theme's green/red, blue/orange, or a single muted tone. */
export type ColorScheme = 'classic' | 'blueOrange' | 'mono';
export const MAX_CHARTS = 6;
/** Drawing tool armed on the chart. */
export type ChartTool = 'hline' | 'alert' | 'trend' | 'rect' | 'fib' | null;
export type ToolboxTab = 'positions' | 'orders' | 'history' | 'journal';

export interface ChartSlot {
  symbol: string;
  timeframe: Timeframe;
}

export interface Indicators {
  sma: boolean;
  ema: boolean;
  bollinger: boolean;
  rsi: boolean;
  volume: boolean;
}

/** Parameters of the built-in indicators. */
export interface IndicatorSettings {
  sma: { period: number; color: string };
  ema: { period: number; color: string };
  bollinger: { period: number; dev: number; color: string };
  rsi: { period: number; color: string };
}

export const DEFAULT_INDICATOR_SETTINGS: IndicatorSettings = {
  sma: { period: 20, color: '#f59e0b' },
  ema: { period: 50, color: '#8b5cf6' },
  bollinger: { period: 20, dev: 2, color: '#06b6d4' },
  rsi: { period: 14, color: '#ec4899' },
};

export interface Toast {
  id: number;
  kind: 'ok' | 'error';
  text: string;
}

export interface TicketPreset {
  symbol: string;
  side: Side;
  type: OrderType;
  price?: number;
}

const JOURNAL_LIMIT = 2000;

export interface TerminalState {
  connection: ConnectionState;
  latencyMs?: number;
  symbols: Record<string, SymbolSpec>;
  symbolOrder: string[];
  quotes: Record<string, Quote>;
  /** +1 uptick, -1 downtick of the last bid change. */
  tickDir: Record<string, 1 | -1>;
  accounts: Account[];
  activeAccountId: string | null;
  positions: Record<string, Position[]>;
  orders: Record<string, PendingOrder[]>;
  history: Record<string, Deal[]>;
  /** Finished orders per account (loaded on demand, newest first). */
  orderHistory: Record<string, OrderHistoryEntry[]>;
  /** Chart lines and price alerts per account (stored on the server). */
  objects: Record<string, ChartObjects>;
  /** Tool armed on the chart: the next placed line becomes a line or an alert. */
  chartTool: ChartTool;
  journal: JournalEntry[];
  // UI (persisted)
  theme: Theme;
  lang: Lang;
  favorites: string[];
  layout: ChartLayout;
  charts: ChartSlot[];
  activeChart: number;
  /** Chart slots taken out of the grid: minimised ones show as chips, closed ones do not. */
  hiddenCharts: Record<number, 'min' | 'closed'>;
  /** Chart slot filling the whole chart area (Windows-style maximise); null = grid. */
  maximizedChart: number | null;
  /** Bottom panel: normal split, maximised over the charts, or collapsed to its tab strip. */
  toolboxMode: 'normal' | 'max' | 'min';
  /** Side panels in the split (pinned) or folded into an edge strip that opens on hover. */
  sidePinned: { left: boolean; right: boolean };
  /** Market Watch columns shown (catalogue in components/mwColumns.ts). */
  mwColumns: MwColumnId[];
  /** Main series style and colours of every chart (right-click menu). */
  chartStyle: ChartStyle;
  colorScheme: ColorScheme;
  showGrid: boolean;
  indicators: Indicators;
  indicatorSettings: IndicatorSettings;
  /** Draw the ask price as a second line (candles follow the bid). */
  showAskLine: boolean;
  oneClickVolume: number;
  /** Auto-reconnect interval (ms) after the quick retries; 0 = manual (lightning button). */
  reconnectEveryMs: number;
  /** Max slippage for market orders in pips (0 = no client limit). */
  maxDeviationPips: number;
  // UI (transient)
  toolboxTab: ToolboxTab;
  ticket: TicketPreset | null;
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  /** Funding / verification dialog (client self-service). */
  accountOpen: boolean;
  toasts: Toast[];

  // actions
  applyQuotes(quotes: Quote[]): void;
  applyEvent(e: TradingEvent): void;
  setReference(symbols: SymbolSpec[], accounts: Account[]): void;
  setHistory(accountId: string, deals: Deal[]): void;
  setOrderHistory(accountId: string, orders: OrderHistoryEntry[]): void;
  /** Replaces an account's objects (from the server or after an edit) and schedules the save. */
  setObjects(accountId: string, objects: ChartObjects, persist?: boolean): void;
  setChartTool(tool: ChartTool): void;
  setActiveAccount(id: string): void;
  setTheme(t: Theme): void;
  toggleTheme(): void;
  setLang(l: Lang): void;
  toggleFavorite(symbol: string): void;
  setLayout(l: ChartLayout): void;
  hideChart(index: number, how: 'min' | 'closed'): void;
  restoreChart(index: number): void;
  toggleMaximize(index: number): void;
  /** Swaps the chart with its visual neighbour in the grid. */
  moveChart(index: number, dir: 'left' | 'right' | 'up' | 'down'): void;
  /** Swaps two chart slots (a chart header dragged onto another chart). */
  swapCharts(a: number, b: number): void;
  /**
   * Opens `symbol` in a free slot: a closed/minimised one, else by growing the
   * layout up to 6. At the ceiling the active chart switches symbol instead and
   * a toast says why.
   */
  openChart(symbol: string): void;
  setToolboxMode(mode: 'normal' | 'max' | 'min'): void;
  setSidePinned(side: 'left' | 'right', pinned: boolean): void;
  toggleMwColumn(id: MwColumnId): void;
  setChartStyle(style: ChartStyle): void;
  setColorScheme(scheme: ColorScheme): void;
  toggleGrid(): void;
  setChartSymbol(symbol: string, index?: number): void;
  setChartTimeframe(tf: Timeframe, index?: number): void;
  setActiveChart(i: number): void;
  toggleIndicator(k: keyof Indicators): void;
  setIndicatorSettings(patch: Partial<IndicatorSettings>): void;
  /** Templates live in the account's server-side chart objects. */
  saveTemplate(name: string): void;
  applyTemplate(id: string): void;
  deleteTemplate(id: string): void;
  toggleAskLine(): void;
  setOneClickVolume(v: number): void;
  setReconnectEvery(ms: number): void;
  setMaxDeviationPips(v: number): void;
  setToolboxTab(t: ToolboxTab): void;
  openTicket(preset?: Partial<TicketPreset>): void;
  closeTicket(): void;
  setPaletteOpen(o: boolean): void;
  setShortcutsOpen(o: boolean): void;
  setAccountOpen(o: boolean): void;
  toast(kind: Toast['kind'], text: string): void;
  dismissToast(id: number): void;
}

let toastSeq = 0;

/** Objects are saved to the server a moment after the last change (one write per burst). */
let objectsSaveTimer: ReturnType<typeof setTimeout> | undefined;
let objectsSaver: ((accountId: string, json: string) => Promise<void>) | null = null;
export function setObjectsSaver(fn: ((accountId: string, json: string) => Promise<void>) | null): void {
  objectsSaver = fn;
}
function scheduleObjectsSave(accountId: string, objects: ChartObjects): void {
  clearTimeout(objectsSaveTimer);
  objectsSaveTimer = setTimeout(() => {
    void objectsSaver?.(accountId, JSON.stringify(objects)).catch(() => undefined);
  }, 600);
}

const defaultCharts: ChartSlot[] = [
  { symbol: 'EURUSD', timeframe: 'M5' },
  { symbol: 'XAUUSD', timeframe: 'M15' },
  { symbol: 'GBPUSD', timeframe: 'H1' },
  { symbol: 'BTCUSD', timeframe: 'M5' },
  { symbol: 'USDJPY', timeframe: 'M15' },
  { symbol: 'XAGUSD', timeframe: 'H1' },
];

export const useTerminal = create<TerminalState>()(
  persist(
    (set, get) => ({
      connection: 'connecting',
      symbols: {},
      symbolOrder: [],
      quotes: {},
      tickDir: {},
      accounts: [],
      activeAccountId: null,
      positions: {},
      orders: {},
      history: {},
      orderHistory: {},
      objects: {},
      chartTool: null,
      journal: [],
      theme: 'dark',
      lang: 'en',
      favorites: ['EURUSD', 'XAUUSD', 'BTCUSD'],
      layout: 1,
      charts: defaultCharts,
      activeChart: 0,
      hiddenCharts: {},
      maximizedChart: null,
      toolboxMode: 'normal',
      sidePinned: { left: true, right: true },
      mwColumns: DEFAULT_MW_COLUMNS,
      chartStyle: 'candles',
      colorScheme: 'classic',
      showGrid: true,
      indicators: { sma: false, ema: true, bollinger: false, rsi: false, volume: true },
      indicatorSettings: DEFAULT_INDICATOR_SETTINGS,
      showAskLine: false,
      oneClickVolume: 10,
      reconnectEveryMs: 3000,
      maxDeviationPips: 0,
      toolboxTab: 'positions',
      ticket: null,
      paletteOpen: false,
      shortcutsOpen: false,
      accountOpen: false,
      toasts: [],

      applyQuotes(batch) {
        if (!batch.length) return;
        const { quotes, tickDir } = get();
        const nq = { ...quotes };
        const nd = { ...tickDir };
        for (const q of batch) {
          const prev = quotes[q.symbol];
          if (prev && prev.bid !== q.bid) nd[q.symbol] = q.bid > prev.bid ? 1 : -1;
          nq[q.symbol] = q;
        }
        set({ quotes: nq, tickDir: nd });
        // Price alerts are checked while the terminal is open (no server push yet).
        const acc = get().activeAccountId;
        const objs = acc ? get().objects[acc] : undefined;
        if (!acc || !objs || !objs.alerts.some((a) => !a.firedAt)) return;
        let fired = false;
        const alerts = objs.alerts.map((a) => {
          const q = nq[a.symbol];
          if (a.firedAt || !q) return a;
          const hit = a.direction === 'above' ? q.bid >= a.price : q.bid <= a.price;
          if (!hit) return a;
          fired = true;
          get().toast('ok', translate(get().lang, 'alert.fired', { symbol: a.symbol, price: a.price }));
          try {
            if (typeof navigator !== 'undefined' && 'vibrate' in navigator) navigator.vibrate([60, 40, 60]);
          } catch {
            /* no haptics */
          }
          return { ...a, firedAt: Date.now() };
        });
        if (fired) get().setObjects(acc, { ...objs, alerts });
      },

      applyEvent(e) {
        switch (e.type) {
          case 'connection':
            set({ connection: e.state, latencyMs: e.latencyMs ?? get().latencyMs });
            return;
          case 'account':
            set({ accounts: get().accounts.map((a) => (a.id === e.account.id ? e.account : a)) });
            return;
          case 'positions':
            set({ positions: { ...get().positions, [e.accountId]: e.positions } });
            return;
          case 'orders':
            set({ orders: { ...get().orders, [e.accountId]: e.orders } });
            return;
          case 'deal': {
            const h = get().history;
            if ((h[e.deal.accountId] ?? []).some((d) => d.id === e.deal.id)) return;
            set({ history: { ...h, [e.deal.accountId]: [...(h[e.deal.accountId] ?? []), e.deal] } });
            return;
          }
          case 'journal': {
            const j = get().journal;
            const next = j.length >= JOURNAL_LIMIT ? j.slice(j.length - JOURNAL_LIMIT + 1) : j.slice();
            next.push(e.entry);
            set({ journal: next });
            return;
          }
        }
      },

      setReference(symbols, accounts) {
        const map: Record<string, SymbolSpec> = {};
        for (const s of symbols) map[s.name] = s;
        const active = get().activeAccountId;
        set({
          symbols: map,
          symbolOrder: symbols.map((s) => s.name),
          accounts,
          activeAccountId: active && accounts.some((a) => a.id === active) ? active : (accounts[0]?.id ?? null),
          charts: get().charts.map((c) => (map[c.symbol] ? c : { ...c, symbol: symbols[0]?.name ?? c.symbol })),
        });
      },
      setHistory(accountId, deals) {
        set({ history: { ...get().history, [accountId]: deals } });
      },
      setOrderHistory(accountId, orders) {
        set({ orderHistory: { ...get().orderHistory, [accountId]: orders } });
      },
      setObjects(accountId, objects, persist = true) {
        set({ objects: { ...get().objects, [accountId]: objects } });
        if (persist) scheduleObjectsSave(accountId, objects);
      },
      setChartTool(chartTool) {
        set({ chartTool });
      },
      setActiveAccount(id) {
        set({ activeAccountId: id });
      },
      setTheme(theme) {
        set({ theme });
      },
      toggleTheme() {
        set({ theme: get().theme === 'dark' ? 'light' : 'dark' });
      },
      setLang(lang) {
        set({ lang });
      },
      toggleFavorite(symbol) {
        const f = get().favorites;
        set({ favorites: f.includes(symbol) ? f.filter((x) => x !== symbol) : [...f, symbol] });
      },
      setLayout(layout) {
        // Picking a layout brings every slot back.
        set({ layout, activeChart: Math.min(get().activeChart, layout - 1), hiddenCharts: {}, maximizedChart: null });
      },
      hideChart(index, how) {
        if (get().maximizedChart === index) set({ maximizedChart: null });
        const hidden = { ...get().hiddenCharts, [index]: how };
        const visible = Array.from({ length: get().layout }, (_, i) => i).filter((i) => !hidden[i]);
        set({ hiddenCharts: hidden, activeChart: visible.includes(get().activeChart) ? get().activeChart : (visible[0] ?? 0) });
      },
      restoreChart(index) {
        const hidden = { ...get().hiddenCharts };
        delete hidden[index];
        set({ hiddenCharts: hidden, activeChart: index });
      },
      setSidePinned(side, pinned) {
        set({ sidePinned: { ...get().sidePinned, [side]: pinned } });
      },
      setChartStyle(chartStyle) {
        set({ chartStyle });
      },
      setColorScheme(colorScheme) {
        set({ colorScheme });
      },
      toggleGrid() {
        set({ showGrid: !get().showGrid });
      },
      toggleMwColumn(id) {
        const cur = get().mwColumns;
        set({ mwColumns: cur.includes(id) ? cur.filter((c) => c !== id) : [...cur, id] });
      },
      toggleMaximize(index) {
        set({ maximizedChart: get().maximizedChart === index ? null : index, activeChart: index });
      },
      moveChart(index, dir) {
        const { layout, hiddenCharts } = get();
        const visible = Array.from({ length: layout }, (_, i) => i).filter((i) => !hiddenCharts[i]);
        const pos = visible.indexOf(index);
        if (pos < 0) return;
        // same grid as ChartGrid: one row up to 2 charts, then rows of 2 (3-4) or 3 (5-6)
        const perRow = visible.length <= 2 ? visible.length : visible.length > 4 ? 3 : 2;
        const col = pos % perRow;
        const target =
          dir === 'left' ? (col > 0 ? pos - 1 : -1)
          : dir === 'right' ? (col < perRow - 1 ? pos + 1 : -1)
          : dir === 'up' ? pos - perRow
          : pos + perRow;
        const other = visible[target];
        if (target < 0 || other === undefined) return;
        get().swapCharts(index, other);
      },
      swapCharts(a, b) {
        if (a === b) return;
        const charts = [...get().charts];
        const [ca, cb] = [charts[a], charts[b]];
        if (!ca || !cb) return;
        charts[a] = cb;
        charts[b] = ca;
        const hidden = { ...get().hiddenCharts };
        const [ha, hb] = [hidden[a], hidden[b]];
        delete hidden[a];
        delete hidden[b];
        if (hb) hidden[a] = hb;
        if (ha) hidden[b] = ha;
        const active = get().activeChart === a ? b : get().activeChart === b ? a : get().activeChart;
        set({ charts, hiddenCharts: hidden, activeChart: active });
      },
      openChart(symbol) {
        const { layout, hiddenCharts, charts, activeChart } = get();
        const free = Object.keys(hiddenCharts).map(Number).find((i) => i < layout);
        if (free !== undefined) {
          const hidden = { ...hiddenCharts };
          delete hidden[free];
          set({ charts: charts.map((c, k) => (k === free ? { ...c, symbol } : c)), hiddenCharts: hidden, activeChart: free });
          return;
        }
        const next = ([2, 4, 6] as ChartLayout[]).find((l) => l > layout);
        if (next !== undefined) {
          set({ layout: next, charts: charts.map((c, k) => (k === layout ? { ...c, symbol } : c)), activeChart: layout });
          return;
        }
        get().toast('error', translate(get().lang, 'chart.max', { n: MAX_CHARTS }));
        set({ charts: charts.map((c, k) => (k === activeChart ? { ...c, symbol } : c)) });
      },
      setToolboxMode(toolboxMode) {
        set({ toolboxMode });
      },
      setChartSymbol(symbol, index) {
        const i = index ?? get().activeChart;
        set({ charts: get().charts.map((c, k) => (k === i ? { ...c, symbol } : c)), activeChart: i });
      },
      setChartTimeframe(timeframe, index) {
        const i = index ?? get().activeChart;
        set({ charts: get().charts.map((c, k) => (k === i ? { ...c, timeframe } : c)) });
      },
      setActiveChart(activeChart) {
        set({ activeChart });
      },
      setIndicatorSettings(patch) {
        set({ indicatorSettings: { ...get().indicatorSettings, ...patch } });
      },
      saveTemplate(name) {
        const acc = get().activeAccountId;
        if (!acc) return;
        const cur = get().objects[acc] ?? { lines: [], alerts: [], shapes: [] };
        const tpl: ChartTemplate = {
          id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
          name,
          indicators: { ...get().indicators },
          settings: { ...get().indicatorSettings },
        };
        get().setObjects(acc, { ...cur, templates: [...(cur.templates ?? []), tpl] });
      },
      applyTemplate(id) {
        const acc = get().activeAccountId;
        const tpl = acc ? get().objects[acc]?.templates?.find((x) => x.id === id) : undefined;
        if (!tpl) return;
        const d = DEFAULT_INDICATOR_SETTINGS;
        const s = tpl.settings;
        set({
          indicators: { ...get().indicators, ...(tpl.indicators as Partial<Indicators>) },
          indicatorSettings: {
            sma: { ...d.sma, ...s.sma },
            ema: { ...d.ema, ...s.ema },
            bollinger: { ...d.bollinger, ...s.bollinger },
            rsi: { ...d.rsi, ...s.rsi },
          },
        });
      },
      deleteTemplate(id) {
        const acc = get().activeAccountId;
        const cur = acc ? get().objects[acc] : undefined;
        if (!acc || !cur) return;
        get().setObjects(acc, { ...cur, templates: (cur.templates ?? []).filter((x) => x.id !== id) });
      },
      toggleAskLine() {
        set({ showAskLine: !get().showAskLine });
      },
      toggleIndicator(k) {
        set({ indicators: { ...get().indicators, [k]: !get().indicators[k] } });
      },
      setOneClickVolume(oneClickVolume) {
        set({ oneClickVolume });
      },
      setReconnectEvery(reconnectEveryMs) {
        set({ reconnectEveryMs: Math.max(0, reconnectEveryMs) });
      },
      setMaxDeviationPips(v) {
        set({ maxDeviationPips: Number.isFinite(v) && v > 0 ? v : 0 });
      },
      setToolboxTab(toolboxTab) {
        set({ toolboxTab });
      },
      openTicket(preset) {
        const s = get();
        const symbol = preset?.symbol ?? s.charts[s.activeChart]?.symbol ?? s.symbolOrder[0] ?? 'EURUSD';
        set({ ticket: { symbol, side: preset?.side ?? 'buy', type: preset?.type ?? 'market', price: preset?.price } });
      },
      closeTicket() {
        set({ ticket: null });
      },
      setPaletteOpen(paletteOpen) {
        set({ paletteOpen });
      },
      setShortcutsOpen(shortcutsOpen) {
        set({ shortcutsOpen });
      },
      setAccountOpen(accountOpen) {
        set({ accountOpen });
      },
      toast(kind, text) {
        const id = ++toastSeq;
        set({ toasts: [...get().toasts.slice(-4), { id, kind, text }] });
        setTimeout(() => get().dismissToast(id), 4000);
      },
      dismissToast(id) {
        set({ toasts: get().toasts.filter((t) => t.id !== id) });
      },
    }),
    {
      name: 'fxvps-terminal',
      version: 1,
      // Older saves hold four chart slots; the six-chart layout needs the rest.
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<TerminalState>;
        const charts = Array.isArray(p.charts) ? [...p.charts, ...defaultCharts.slice(p.charts.length)] : current.charts;
        return { ...current, ...p, charts };
      },
      storage: createJSONStorage(() => {
        try {
          return localStorage;
        } catch {
          return undefined as unknown as Storage;
        }
      }),
      partialize: (s) => ({
        theme: s.theme,
        lang: s.lang,
        favorites: s.favorites,
        layout: s.layout,
        charts: s.charts,
        hiddenCharts: s.hiddenCharts,
        toolboxMode: s.toolboxMode,
        sidePinned: s.sidePinned,
        mwColumns: s.mwColumns,
        chartStyle: s.chartStyle,
        colorScheme: s.colorScheme,
        showGrid: s.showGrid,
        indicators: s.indicators,
        indicatorSettings: s.indicatorSettings,
        showAskLine: s.showAskLine,
        oneClickVolume: s.oneClickVolume,
        reconnectEveryMs: s.reconnectEveryMs,
        maxDeviationPips: s.maxDeviationPips,
        activeAccountId: s.activeAccountId,
      }),
    },
  ),
);

/** Positions a bulk close targets: all, only the profitable or only the losing ones. */
export function bulkTargets(positions: Position[], profitOf: (p: Position) => number, which: 'all' | 'profit' | 'loss'): Position[] {
  return positions.filter((p) => (which === 'all' ? true : which === 'profit' ? profitOf(p) > 0 : profitOf(p) < 0));
}

// ---- selectors -----------------------------------------------------------
export const selectActiveAccount = (s: TerminalState): Account | undefined =>
  s.accounts.find((a) => a.id === s.activeAccountId);
const EMPTY_P: Position[] = [];
const EMPTY_O: PendingOrder[] = [];
const EMPTY_D: Deal[] = [];
export const selectPositions = (s: TerminalState): Position[] =>
  (s.activeAccountId && s.positions[s.activeAccountId]) || EMPTY_P;
export const selectOrders = (s: TerminalState): PendingOrder[] =>
  (s.activeAccountId && s.orders[s.activeAccountId]) || EMPTY_O;
export const selectHistory = (s: TerminalState): Deal[] =>
  (s.activeAccountId && s.history[s.activeAccountId]) || EMPTY_D;
