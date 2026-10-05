import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import type {
  Account,
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
import type { Lang } from '../i18n';

export type Theme = 'dark' | 'light';
export type ChartLayout = 1 | 2 | 4;
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
  journal: JournalEntry[];
  // UI (persisted)
  theme: Theme;
  lang: Lang;
  favorites: string[];
  layout: ChartLayout;
  charts: ChartSlot[];
  activeChart: number;
  indicators: Indicators;
  oneClickVolume: number;
  // UI (transient)
  toolboxTab: ToolboxTab;
  ticket: TicketPreset | null;
  paletteOpen: boolean;
  shortcutsOpen: boolean;
  toasts: Toast[];

  // actions
  applyQuotes(quotes: Quote[]): void;
  applyEvent(e: TradingEvent): void;
  setReference(symbols: SymbolSpec[], accounts: Account[]): void;
  setHistory(accountId: string, deals: Deal[]): void;
  setOrderHistory(accountId: string, orders: OrderHistoryEntry[]): void;
  setActiveAccount(id: string): void;
  setTheme(t: Theme): void;
  toggleTheme(): void;
  setLang(l: Lang): void;
  toggleFavorite(symbol: string): void;
  setLayout(l: ChartLayout): void;
  setChartSymbol(symbol: string, index?: number): void;
  setChartTimeframe(tf: Timeframe, index?: number): void;
  setActiveChart(i: number): void;
  toggleIndicator(k: keyof Indicators): void;
  setOneClickVolume(v: number): void;
  setToolboxTab(t: ToolboxTab): void;
  openTicket(preset?: Partial<TicketPreset>): void;
  closeTicket(): void;
  setPaletteOpen(o: boolean): void;
  setShortcutsOpen(o: boolean): void;
  toast(kind: Toast['kind'], text: string): void;
  dismissToast(id: number): void;
}

let toastSeq = 0;

const defaultCharts: ChartSlot[] = [
  { symbol: 'EURUSD', timeframe: 'M5' },
  { symbol: 'XAUUSD', timeframe: 'M15' },
  { symbol: 'GBPUSD', timeframe: 'H1' },
  { symbol: 'BTCUSD', timeframe: 'M5' },
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
      journal: [],
      theme: 'dark',
      lang: 'en',
      favorites: ['EURUSD', 'XAUUSD', 'BTCUSD'],
      layout: 1,
      charts: defaultCharts,
      activeChart: 0,
      indicators: { sma: false, ema: true, bollinger: false, rsi: false, volume: true },
      oneClickVolume: 10,
      toolboxTab: 'positions',
      ticket: null,
      paletteOpen: false,
      shortcutsOpen: false,
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
        set({ layout, activeChart: Math.min(get().activeChart, layout - 1) });
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
      toggleIndicator(k) {
        set({ indicators: { ...get().indicators, [k]: !get().indicators[k] } });
      },
      setOneClickVolume(oneClickVolume) {
        set({ oneClickVolume });
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
        indicators: s.indicators,
        oneClickVolume: s.oneClickVolume,
        activeAccountId: s.activeAccountId,
      }),
    },
  ),
);

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
