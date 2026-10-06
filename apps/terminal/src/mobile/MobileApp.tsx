import { memo, useEffect, useMemo, useState, type ReactNode } from 'react';
import type { Deal, OrderHistoryEntry, PendingOrder, Position, Side, SymbolSpec } from '@fxvps/trading-core';
import {
  TIMEFRAMES,
  big,
  closePrice,
  formatMoney,
  formatPrice,
  formatTimeShort,
  lotsToVolume,
  roundPrice,
  parseDecimal,
  positionProfit,
  spreadPoints,
  volumeToLots,
} from '@fxvps/trading-core';
import { useMetrics, useRates, useT } from '../hooks';
import type { MessageKey } from '../i18n';
import { getApi, trade as tradeApi } from '../store/api';
import { useSession } from '../store/session';
import { bulkTargets, selectActiveAccount, selectHistory, selectOrders, selectPositions, useTerminal } from '../store/terminal';
import { ChartAttribution, ChartPanel } from '../components/ChartPanel';
import { ObjectList } from '../components/ObjectList';
import { IndicatorSettings } from '../components/IndicatorSettings';
import { promptInstall, useInstallPrompt } from '../lib/install';
import { OrderTicket } from '../components/OrderTicket';

/** Phone layout: one view at a time, bottom tab bar, order ticket as a bottom sheet. */

type Tab = 'markets' | 'chart' | 'trade' | 'history' | 'account';
const TABS: { id: Tab; label: MessageKey; icon: string }[] = [
  { id: 'markets', label: 'm.markets', icon: 'M4 6h16M4 12h16M4 18h10' },
  { id: 'chart', label: 'm.chart', icon: 'M4 19V5m0 14h16M8 15l3-4 3 2 4-6' },
  { id: 'trade', label: 'm.trade', icon: 'M7 4v16m0 0-3-3m3 3 3-3M17 20V4m0 0-3 3m3-3 3 3' },
  { id: 'history', label: 'tb.history', icon: 'M12 8v4l3 2M4 12a8 8 0 1 0 2.5-5.8M4 5v4h4' },
  { id: 'account', label: 'm.account', icon: 'M12 12a4 4 0 100-8 4 4 0 000 8zm-7 8a7 7 0 0114 0' },
];

const tone = (v: number) => (v > 0 ? 'text-up' : v < 0 ? 'text-down' : 'text-fg');

/** Price with the pip digits enlarged (FX convention). */
function Price({ value, digits, className = '' }: { value: number; digits: number; className?: string }) {
  const s = formatPrice(value, digits);
  const frac = digits === 5 || digits === 3;
  const head = s.slice(0, frac ? -3 : -2);
  const pips = frac ? s.slice(-3, -1) : s.slice(-2);
  return (
    <span className={`num ${className}`}>
      <span className="opacity-70">{head}</span>
      <span className="text-[1.3em] font-semibold leading-none">{pips}</span>
      {frac && <sup className="text-[0.75em]">{s.slice(-1)}</sup>}
    </span>
  );
}

const QUICK_KEY = 'fxvps.quickOrder';
function readQuick(): boolean {
  try {
    return localStorage.getItem(QUICK_KEY) === '1';
  } catch {
    return false;
  }
}
function writeQuick(on: boolean): void {
  try {
    localStorage.setItem(QUICK_KEY, on ? '1' : '0');
  } catch {
    /* private mode: the choice lasts for this page only */
  }
}

function changePct(bid: number, dayOpen?: number): number | null {
  return dayOpen ? Number(big(bid).minus(dayOpen).div(dayOpen).times(100).toFixed(2)) : null;
}

const MarketRow = memo(function MarketRow({ symbol, onOpen }: { symbol: string; onOpen: (s: string) => void }) {
  const q = useTerminal((s) => s.quotes[symbol]);
  const spec = useTerminal((s) => s.symbols[symbol]);
  const dir = useTerminal((s) => s.tickDir[symbol]);
  const fav = useTerminal((s) => s.favorites.includes(symbol));
  const toggleFavorite = useTerminal((s) => s.toggleFavorite);
  const t = useT();
  if (!spec) return null;
  const chg = q ? changePct(q.bid, q.dayOpen) : null;
  const flash = dir === 1 ? 'text-up' : dir === -1 ? 'text-down' : '';
  return (
    <div
      role="button"
      tabIndex={0}
      data-testid={`m-row-${symbol}`}
      onClick={() => onOpen(symbol)}
      className="flex items-center gap-3 px-4 h-[46px] cursor-pointer active:bg-hover border-b border-line/40"
    >
      <button
        aria-label={t('mw.favorite')}
        className={`text-[18px] w-6 ${fav ? 'text-amber-400' : 'text-muted/40'}`}
        onClick={(e) => {
          e.stopPropagation();
          toggleFavorite(symbol);
        }}
      >
        {fav ? '★' : '☆'}
      </button>
      <div className="flex-1 min-w-0">
        <div className="font-semibold text-[14px] leading-tight tracking-tight">{symbol}</div>
        <div className="num text-[10px] leading-tight text-muted truncate">{q ? `${t('m.spread')} ${spreadPoints(q, spec.digits)}` : spec.description}</div>
      </div>
      {q ? (
        <>
          <Price key={`b${q.time}`} value={q.bid} digits={spec.digits} className={`text-[14px] w-[76px] text-right ${flash}`} />
          <Price key={`a${q.time}`} value={q.ask} digits={spec.digits} className={`text-[14px] w-[76px] text-right ${flash}`} />
          <span
            className={`num text-[11px] w-[54px] text-center rounded-full py-1 ${chg !== null && chg < 0 ? 'bg-down-bg text-down' : 'bg-up-bg text-up'}`}
          >
            {chg === null ? '—' : `${chg > 0 ? '+' : ''}${chg.toFixed(2)}%`}
          </span>
        </>
      ) : (
        <span className="text-muted pr-2">…</span>
      )}
    </div>
  );
});

function Markets({ onOpen }: { onOpen: (s: string) => void }) {
  const t = useT();
  const order = useTerminal((s) => s.symbolOrder);
  const symbols = useTerminal((s) => s.symbols);
  const favorites = useTerminal((s) => s.favorites);
  const [query, setQuery] = useState('');
  const [onlyFav, setOnlyFav] = useState(false);
  const rows = useMemo(() => {
    const q = query.trim().toUpperCase();
    const list = order.filter(
      (s) => (!onlyFav || favorites.includes(s)) && (!q || s.includes(q) || symbols[s]?.description.toUpperCase().includes(q)),
    );
    return [...list].sort((a, b) => Number(favorites.includes(b)) - Number(favorites.includes(a)));
  }, [order, query, onlyFav, favorites, symbols]);
  const chip = (on: boolean) => `px-4 h-9 rounded-full text-[13px] font-medium ${on ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`;
  return (
    <div className="fx-view flex flex-col h-full">
      <Hero />
      <div className="flex gap-2 px-4 pb-2">
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('mw.search')}
          aria-label={t('mw.search')}
          className="flex-1 min-w-0 h-9 rounded-full bg-panel-2 border border-line/60 px-4 outline-none focus:border-accent"
        />
        <button className={chip(!onlyFav)} onClick={() => setOnlyFav(false)}>
          {t('mw.all')}
        </button>
        <button className={chip(onlyFav)} onClick={() => setOnlyFav(true)} aria-label={t('mw.favorites')}>
          ★
        </button>
      </div>
      <div className="flex-1 overflow-y-auto" data-testid="m-markets">
        {rows.length === 0 && <div className="p-6 text-center text-muted">{t('mw.empty')}</div>}
        {rows.map((s) => (
          <MarketRow key={s} symbol={s} onOpen={onOpen} />
        ))}
      </div>
    </div>
  );
}

/** Equity card at the top of the markets and trade views. */
function Hero() {
  const t = useT();
  const m = useMetrics();
  const account = useTerminal(selectActiveAccount);
  if (!m || !account) return null;
  return (
    <div className="fx-card mx-4 my-2 px-4 py-2 flex items-end justify-between" data-testid="m-hero">
      <div>
        <div className="text-[11px] uppercase tracking-wider text-muted">
          {t('top.equity')} · {account.currency}
        </div>
        <div className="num text-[24px] font-semibold leading-tight tracking-tight">{formatMoney(m.equity)}</div>
      </div>
      <div className="text-right">
        <div className={`num text-[15px] font-semibold ${tone(m.floating)}`}>
          {m.floating > 0 ? '+' : ''}
          {formatMoney(m.floating)}
        </div>
        <div className="text-[11px] text-muted">
          {t('top.freeMargin')} <span className="num text-fg">{formatMoney(m.freeMargin)}</span>
        </div>
      </div>
    </div>
  );
}

/** Lot size: arrows step, a tap on the number opens a keypad for a direct entry. */
function LotStepper({ volume, onStep, onSet, spec }: { volume: number; onStep: (dir: 1 | -1) => void; onSet: (v: number) => void; spec?: SymbolSpec }) {
  const t = useT();
  const [text, setText] = useState<string | null>(null);
  const commit = () => {
    if (text !== null) {
      const v = lotsToVolume(text);
      if (v && spec) onSet(Math.min(spec.maxVolume, Math.max(spec.minVolume, v)));
    }
    setText(null);
  };
  return (
    <div className="flex items-center rounded-2xl bg-panel-2 border border-line/60 px-1" data-testid="m-quick-volume">
      <button className="w-9 h-14 text-[18px] text-muted" onClick={() => onStep(-1)} aria-label="-">
        ▾
      </button>
      <div className="w-14 text-center">
        <div className="text-[9px] uppercase tracking-wider text-muted">{t('chart.lots')}</div>
        {text === null ? (
          <button className="num text-[15px] font-semibold w-full" onClick={() => setText(volumeToLots(volume))} data-testid="m-lots-edit" aria-label={t('ticket.volume')}>
            {volumeToLots(volume)}
          </button>
        ) : (
          <input
            autoFocus
            inputMode="decimal"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === 'Enter') commit();
              if (e.key === 'Escape') setText(null);
            }}
            onFocus={(e) => e.currentTarget.select()}
            className="num w-full text-center bg-transparent outline-none text-[15px] font-semibold"
            aria-label={t('ticket.volume')}
            data-testid="m-lots-input"
          />
        )}
      </div>
      <button className="w-9 h-14 text-[18px] text-muted" onClick={() => onStep(1)} aria-label="+">
        ▴
      </button>
    </div>
  );
}

function ChartView({ onSymbols }: { onSymbols: () => void }) {
  const t = useT();
  const slot = useTerminal((s) => s.charts[0]);
  const spec = useTerminal((s) => (slot ? s.symbols[slot.symbol] : undefined));
  const q = useTerminal((s) => (slot ? s.quotes[slot.symbol] : undefined));
  const setTf = useTerminal((s) => s.setChartTimeframe);
  const openTicket = useTerminal((s) => s.openTicket);
  const volume = useTerminal((s) => s.oneClickVolume);
  const setVolume = useTerminal((s) => s.setOneClickVolume);
  // Quick order: Sell / Buy send a market order of the shown size at once (no ticket).
  const [quick, setQuick] = useState(() => readQuick());
  const [busy, setBusy] = useState(false);
  // Chart tools sheet (lines, alerts) and the armed tool.
  const [tools, setTools] = useState(false);
  const setChartTool = useTerminal((s) => s.setChartTool);
  const chartTool = useTerminal((s) => s.chartTool);
  // Full screen: the Fullscreen API where it exists (Android, desktop); iPhone Safari
  // has none for pages, so the button explains "Add to Home Screen" instead.
  const [fsHelp, setFsHelp] = useState(false);
  const [fullscreen, setFullscreen] = useState(() => typeof document !== 'undefined' && !!document.fullscreenElement);
  useEffect(() => {
    const on = () => setFullscreen(!!document.fullscreenElement);
    document.addEventListener('fullscreenchange', on);
    return () => document.removeEventListener('fullscreenchange', on);
  }, []);
  const standalone = typeof matchMedia === 'function' && (matchMedia('(display-mode: standalone)').matches || (navigator as { standalone?: boolean }).standalone === true);
  // Android Chrome hands us an install prompt we can show from our own button;
  // iPhone browsers only offer the system share sheet (which has "Add to Home Screen").
  const installPrompt = useInstallPrompt();
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  const iosChrome = /CriOS/.test(ua);
  // iPhone Chrome's web-share sheet has no "Add to Home Screen" (only Chrome's own
  // share menu next to the address bar does), so there we show the steps instead.
  const canShare = !iosChrome && typeof navigator !== 'undefined' && typeof navigator.share === 'function';
  const [shared, setShared] = useState(false);
  const addToHome = async () => {
    if (installPrompt) {
      await promptInstall();
      setFsHelp(false);
      return;
    }
    if (canShare) {
      try {
        await navigator.share({ title: 'fxvps.ai', url: location.origin + '/' });
      } catch {
        /* sheet dismissed */
      }
      setShared(true);
    }
  };
  const toggleFullscreen = () => {
    const el = document.documentElement;
    if (document.fullscreenElement) return void document.exitFullscreen?.();
    if (document.fullscreenEnabled && el.requestFullscreen) return void el.requestFullscreen().catch(() => setFsHelp(true));
    setFsHelp(true);
  };
  // Pending order from the chart: drag the line to a price, then place.
  const [limit, setLimit] = useState<number | null>(null);
  const [lineType, setLineType] = useState<'limit' | 'stop'>('limit');
  const accountId = useTerminal((s) => s.activeAccountId);
  const toast = useTerminal((s) => s.toast);
  if (!slot) return null;
  const chg = q ? changePct(q.bid, q.dayOpen) : null;
  const step = (dir: 1 | -1) => {
    if (!spec) return;
    setVolume(Math.min(spec.maxVolume, Math.max(spec.minVolume, volume + dir * spec.volumeStep)));
  };
  // Below the market a limit buys and a stop sells; above it the other way round.
  const above = limit !== null && !!q && limit >= q.bid;
  const limitSide: Side = above === (lineType === 'limit') ? 'sell' : 'buy';
  const limitName = `${t(limitSide === 'buy' ? 'ticket.buy' : 'ticket.sell')} ${t(lineType === 'limit' ? 'ticket.limit' : 'ticket.stop')}`;
  const toggleLimit = () => {
    if (limit !== null || !q || !spec) return setLimit(null);
    // Start a little below the market so the line is easy to grab.
    setLimit(roundPrice(q.bid * 0.9995, spec.digits));
  };
  const placeLimit = async () => {
    if (limit === null || !accountId || busy) return;
    setBusy(true);
    try {
      const r = await getApi().placeOrder({ accountId, symbol: slot.symbol, side: limitSide, type: lineType, volume, price: limit });
      if (r.ok) {
        toast('ok', t('toast.placed', { id: r.orderId ?? '' }));
        setLimit(null);
      } else toast('error', t('toast.rejected', { error: r.error ?? '' }));
    } finally {
      setBusy(false);
    }
  };
  const lotStepper = <LotStepper volume={volume} onStep={step} onSet={setVolume} spec={spec} />;
  const toggleQuick = () => {
    writeQuick(!quick);
    setQuick(!quick);
  };
  const send = async (side: Side) => {
    if (!quick) return openTicket({ symbol: slot.symbol, side, type: 'market' });
    if (busy) return;
    setBusy(true);
    try {
      await tradeApi.market(slot.symbol, side, volume);
    } finally {
      setBusy(false);
    }
  };
  const trade = (side: Side, cls: string, label: MessageKey, px?: number) => (
    <button
      onClick={() => void send(side)}
      disabled={busy}
      className={`flex-1 h-14 rounded-2xl text-white flex flex-col items-center justify-center active:scale-[0.97] transition-transform disabled:opacity-60 ${cls}`}
      data-testid={`m-${side}`}
    >
      <span className="text-[11px] uppercase tracking-wider opacity-85">{t(label)}</span>
      {spec && px !== undefined ? <Price value={px} digits={spec.digits} className="text-[15px]" /> : <span>—</span>}
    </button>
  );
  return (
    <div className="fx-view flex flex-col h-full">
      <div className="flex items-center justify-between px-4 pt-1">
        <button className="text-left" onClick={onSymbols} data-testid="m-symbol">
          <div className="text-[20px] font-semibold tracking-tight">
            {slot.symbol} <span className="text-muted text-[14px]">▾</span>
          </div>
          <div className="text-[11px] text-muted">{spec?.description}</div>
        </button>
        {!standalone && (
          <button
            onClick={toggleFullscreen}
            aria-pressed={fullscreen}
            aria-label={t('m.fullscreen')}
            title={t('m.fullscreen')}
            data-testid="m-fullscreen"
            className={`ml-auto mr-2 h-10 w-10 rounded-full grid place-items-center ${fullscreen ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`}
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
            </svg>
          </button>
        )}
        <button
          onClick={toggleQuick}
          aria-pressed={quick}
          aria-label={t('m.quick')}
          title={t('m.quick')}
          data-testid="m-quick-toggle"
          className={`ml-auto h-10 w-10 rounded-full grid place-items-center ${quick ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
            <path d="M13 2 4 14h6l-1 8 9-12h-6z" />
          </svg>
        </button>
        <button
          onClick={toggleLimit}
          aria-pressed={limit !== null}
          aria-label={t('m.limitLine')}
          title={t('m.limitLine')}
          data-testid="m-limit-toggle"
          className={`mx-2 h-10 w-10 rounded-full grid place-items-center ${limit !== null ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
            <path d="M3 12h18M12 4v4m0 8v4M9 6l3-3 3 3M9 18l3 3 3-3" />
          </svg>
        </button>
        <button
          onClick={() => setTools(true)}
          aria-label={t('obj.tools')}
          title={t('obj.tools')}
          data-testid="m-tools"
          className={`ml-2 mr-1 h-10 w-10 rounded-full grid place-items-center ${chartTool ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden>
            <path d="M4 20l4-1 11-11-3-3L5 16l-1 4zM14 7l3 3" />
          </svg>
        </button>
        <div className="text-right">
          {spec && q && <Price value={q.bid} digits={spec.digits} className="text-[20px]" />}
          {chg !== null && (
            <div className={`num text-[12px] ${chg < 0 ? 'text-down' : 'text-up'}`}>
              {chg > 0 ? '+' : ''}
              {chg.toFixed(2)}%
            </div>
          )}
        </div>
      </div>
      <div className="flex gap-1 px-3 py-1 overflow-x-auto">
        {TIMEFRAMES.map((tf) => (
          <button
            key={tf}
            onClick={() => setTf(tf, 0)}
            className={`px-3 h-8 rounded-full text-[12px] font-medium shrink-0 ${tf === slot.timeframe ? 'bg-accent text-white' : 'text-muted'}`}
          >
            {tf}
          </button>
        ))}
      </div>
      <div className="flex-1 min-h-0">
        <ChartPanel
          index={0}
          bare
          draft={limit !== null && spec ? { price: limit, label: `${limitName} ${formatPrice(limit, spec.digits)}`, tone: limitSide === 'buy' ? 'up' : 'down', onMove: setLimit } : undefined}
        />
      </div>
      {fsHelp && (
        <div className="fixed inset-0 z-40 flex flex-col justify-end bg-black/55" onClick={() => setFsHelp(false)} data-testid="m-fs-help">
          <div className="fx-sheet bg-panel rounded-t-[28px] px-5 pt-3 pb-[calc(env(safe-area-inset-bottom)+20px)]" onClick={(e) => e.stopPropagation()}>
            <div className="mx-auto mb-3 h-1 w-10 rounded-full bg-line" />
            <h2 className="text-[17px] font-semibold">{t('m.fsTitle')}</h2>
            <p className="mt-2 text-muted">{t('m.fsBody')}</p>
            {(installPrompt || canShare) && (
              <button className="mt-3 w-full h-12 rounded-2xl bg-accent text-white font-semibold" onClick={() => void addToHome()} data-testid="m-add-home">
                {installPrompt ? t('m.fsInstall') : t('m.fsShare')}
              </button>
            )}
            {shared && <p className="mt-2 text-[13px] text-muted">{t('m.fsAfterShare')}</p>}
            {!installPrompt && (
              <ol className="mt-3 flex flex-col gap-2">
                {([iosChrome ? 'm.fsStep1Chrome' : 'm.fsStep1Safari', 'm.fsStep2', 'm.fsStep3'] as const).map((k, i) => (
                  <li key={k} className="flex items-center gap-3">
                    <span className="num w-7 h-7 rounded-full bg-panel-2 text-muted grid place-items-center text-[13px] font-semibold">{i + 1}</span>
                    {t(k)}
                  </li>
                ))}
              </ol>
            )}
            <button className="mt-4 w-full h-12 rounded-2xl bg-panel-2 font-medium" onClick={() => setFsHelp(false)}>
              {t('sec.done')}
            </button>
          </div>
        </div>
      )}
      {tools && (
        <div className="fixed inset-0 z-40 flex flex-col justify-end bg-black/55" onClick={() => setTools(false)} data-testid="m-tools-sheet">
          <div className="fx-sheet bg-panel rounded-t-[28px] max-h-[80dvh] overflow-y-auto pb-[env(safe-area-inset-bottom)]" onClick={(e) => e.stopPropagation()}>
            <div className="mx-auto mt-2 h-1 w-10 rounded-full bg-line" />
            <h2 className="px-5 pt-2 pb-3 text-[17px] font-semibold">
              {t('obj.tools')} · {slot.symbol}
            </h2>
            <div className="grid grid-cols-2 gap-2 px-5 pb-4">
              {(['hline', 'alert', 'trend', 'rect', 'fib'] as const).map((k) => (
                <button
                  key={k}
                  className="flex-1 h-12 rounded-2xl bg-panel-2 border border-line/60 font-medium"
                  onClick={() => {
                    setChartTool(k);
                    setTools(false);
                  }}
                  data-testid={`m-tool-${k}`}
                >
                  {k === 'hline' ? '—' : k === 'alert' ? '🔔' : k === 'trend' ? '╱' : k === 'rect' ? '▭' : '𝔽'} {t(`obj.${k}`)}
                </button>
              ))}
            </div>
            <div className="px-2 pb-4 text-[14px]">
              <div className="px-3 pb-1 text-[11px] uppercase tracking-wider text-muted">{t('obj.objects')}</div>
              <ObjectList symbol={slot.symbol} />
            </div>
            <div className="px-5 pb-4">
              <div className="pb-2 text-[11px] uppercase tracking-wider text-muted">{t('ind.title')}</div>
              <IndicatorSettings compact />
            </div>
          </div>
        </div>
      )}
      {limit !== null && spec && (
        <div className="mx-4 mt-2 p-1 rounded-full bg-panel-2 flex" role="radiogroup" aria-label={t('ticket.type')}>
          {(['limit', 'stop'] as const).map((x) => (
            <button
              key={x}
              role="radio"
              aria-checked={lineType === x}
              onClick={() => setLineType(x)}
              data-testid={`m-line-${x}`}
              className={`flex-1 h-8 rounded-full text-[13px] font-medium ${lineType === x ? 'bg-panel text-fg shadow' : 'text-muted'}`}
            >
              {t(`ticket.${x}`)}
            </button>
          ))}
        </div>
      )}
      {limit !== null && spec ? (
        <div className="flex gap-2 px-4 py-3" data-testid="m-limit-bar">
          <button className="w-12 h-14 rounded-2xl bg-panel-2 border border-line/60 text-muted" onClick={() => setLimit(null)} aria-label={t('tb.cancel')}>
            ✕
          </button>
          {lotStepper}
          <button
            onClick={() => void placeLimit()}
            disabled={busy}
            data-testid="m-limit-place"
            className={`flex-1 h-14 rounded-2xl text-white flex flex-col items-center justify-center active:scale-[0.97] transition-transform disabled:opacity-60 ${limitSide === 'buy' ? 'bg-up' : 'bg-down'}`}
          >
            <span className="text-[11px] uppercase tracking-wider opacity-85">{limitName}</span>
            <Price value={limit} digits={spec.digits} className="text-[15px]" />
          </button>
        </div>
      ) : (
        <div className={`flex px-4 py-3 ${quick ? 'gap-2' : 'gap-3'}`}>
          {trade('sell', 'bg-down', 'chart.sell', q?.bid)}
          {quick && lotStepper}
          {trade('buy', 'bg-up', 'chart.buy', q?.ask)}
        </div>
      )}
    </div>
  );
}

const PositionCard = memo(function PositionCard({ p }: { p: Position }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[p.symbol]);
  const q = useTerminal((s) => s.quotes[p.symbol]);
  const account = useTerminal(selectActiveAccount);
  const toast = useTerminal((s) => s.toast);
  const rates = useRates();
  const [armed, setArmed] = useState(false);
  const [edit, setEdit] = useState(false);
  const [sl, setSl] = useState('');
  const [tp, setTp] = useState('');
  useEffect(() => {
    if (!armed) return;
    const id = setTimeout(() => setArmed(false), 3000);
    return () => clearTimeout(id);
  }, [armed]);
  if (!spec || !account) return null;
  let profit = 0;
  try {
    if (q) profit = positionProfit(p, spec, q, account.currency, rates);
  } catch {
    /* no conversion rate yet */
  }
  const close = async () => {
    if (!armed) return setArmed(true);
    setArmed(false);
    const r = await getApi().closePosition(account.id, p.id);
    if (r.ok) toast('ok', t('toast.closed', { id: p.id, price: r.price ?? '' }));
    else toast('error', t('toast.rejected', { error: r.error ?? '' }));
  };
  const save = async () => {
    const r = await getApi().modifyPosition(account.id, p.id, parseDecimal(sl), parseDecimal(tp));
    if (r.ok) setEdit(false);
    else toast('error', t('toast.rejected', { error: r.error ?? '' }));
  };
  const inp = 'num flex-1 min-w-0 h-10 rounded-xl bg-panel-2 border border-line/60 px-3 outline-none focus:border-accent';
  return (
    <div className="fx-card px-3 py-2.5" data-testid={`m-pos-${p.id}`}>
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-2">
            <span className="font-semibold text-[16px]">{p.symbol}</span>
            <span className={`text-[11px] uppercase font-semibold px-2 py-0.5 rounded-full ${p.side === 'buy' ? 'bg-up-bg text-up' : 'bg-down-bg text-down'}`}>
              {p.side} {volumeToLots(p.volume)}
            </span>
          </div>
          <div className="num text-[12px] text-muted mt-1">
            {formatPrice(p.openPrice, spec.digits)} → {q ? formatPrice(closePrice(p.side, q), spec.digits) : '…'}
          </div>
        </div>
        <div className={`num text-[20px] font-semibold ${tone(profit)}`}>
          {profit > 0 ? '+' : ''}
          {formatMoney(profit)}
        </div>
      </div>
      {edit && (
        <div className="flex gap-2 mt-2">
          <input className={inp} inputMode="decimal" value={sl} onChange={(e) => setSl(e.target.value)} placeholder={t('ticket.sl')} aria-label={t('ticket.sl')} />
          <input className={inp} inputMode="decimal" value={tp} onChange={(e) => setTp(e.target.value)} placeholder={t('ticket.tp')} aria-label={t('ticket.tp')} />
        </div>
      )}
      <div className="flex gap-2 mt-2">
        {edit ? (
          <>
            <button className="flex-1 h-10 rounded-xl border border-line text-muted" onClick={() => setEdit(false)}>
              {t('tb.cancel')}
            </button>
            <button className="flex-1 h-10 rounded-xl bg-accent text-white font-medium" onClick={() => void save()}>
              {t('tb.save')}
            </button>
          </>
        ) : (
          <>
            <button
              className="flex-1 h-10 rounded-xl border border-line text-muted num text-[12px]"
              onClick={() => {
                setSl(p.sl !== undefined ? String(p.sl) : '');
                setTp(p.tp !== undefined ? String(p.tp) : '');
                setEdit(true);
              }}
            >
              {p.sl !== undefined || p.tp !== undefined
                ? `${p.sl !== undefined ? formatPrice(p.sl, spec.digits) : '—'} · ${p.tp !== undefined ? formatPrice(p.tp, spec.digits) : '—'}`
                : t('m.protect')}
            </button>
            <button
              className={`flex-1 h-10 rounded-xl font-medium transition-colors ${armed ? 'bg-down text-white' : 'bg-down-bg text-down'}`}
              onClick={() => void close()}
              data-testid={`m-close-${p.id}`}
            >
              {armed ? t('m.confirmClose') : t('tb.close')}
            </button>
          </>
        )}
      </div>
    </div>
  );
});

function OrderCard({ o }: { o: PendingOrder }) {
  const t = useT();
  const digits = useTerminal((s) => s.symbols[o.symbol]?.digits ?? 5);
  const toast = useTerminal((s) => s.toast);
  const cancel = async () => {
    const r = await getApi().cancelOrder(o.accountId, o.id);
    if (!r.ok) toast('error', t('toast.rejected', { error: r.error ?? '' }));
  };
  return (
    <div className="fx-card px-3 py-2 flex items-center justify-between">
      <div>
        <div className="font-semibold text-[15px]">
          {o.symbol} <span className={`text-[11px] uppercase ${o.side === 'buy' ? 'text-up' : 'text-down'}`}>{o.side} {o.type.replace('_', ' ')}</span>
        </div>
        <div className="num text-[12px] text-muted mt-1">
          {volumeToLots(o.volume)} @ {formatPrice(o.price, digits)}
        </div>
      </div>
      <button className="h-10 px-4 rounded-xl border border-line text-muted" onClick={() => void cancel()}>
        {t('tb.cancel')}
      </button>
    </div>
  );
}

function DealRow({ d }: { d: Deal }) {
  const digits = useTerminal((s) => s.symbols[d.symbol]?.digits ?? 5);
  return (
    <div className="flex items-center justify-between px-1 py-1.5 border-b border-line/40">
      <div>
        <div className="font-medium">
          {d.symbol} <span className={`text-[11px] uppercase ${d.side === 'buy' ? 'text-up' : 'text-down'}`}>{d.side} {d.entry}</span>
        </div>
        <div className="num text-[11px] text-muted">
          {formatTimeShort(d.time)} · {volumeToLots(d.volume)} @ {formatPrice(d.price, digits)}
        </div>
      </div>
      <span className={`num font-semibold ${tone(d.profit)}`}>{d.entry === 'out' ? formatMoney(d.profit) : '—'}</span>
    </div>
  );
}

/** Open positions and working orders (MT5 "Trade" tab). */
/** Close all / profitable / losing positions, each armed by a first tap. */
function BulkClose({ positions }: { positions: Position[] }) {
  const t = useT();
  const account = useTerminal(selectActiveAccount);
  const quotes = useTerminal((s) => s.quotes);
  const symbols = useTerminal((s) => s.symbols);
  const toast = useTerminal((s) => s.toast);
  const rates = useRates();
  const [armed, setArmed] = useState<'all' | 'profit' | 'loss' | null>(null);
  useEffect(() => {
    if (!armed) return;
    const id = setTimeout(() => setArmed(null), 3000);
    return () => clearTimeout(id);
  }, [armed]);
  if (!account || positions.length === 0) return null;
  const profitOf = (p: Position) => {
    const spec = symbols[p.symbol];
    const q = quotes[p.symbol];
    try {
      return spec && q ? positionProfit(p, spec, q, account.currency, rates) : 0;
    } catch {
      return 0;
    }
  };
  const run = async (which: 'all' | 'profit' | 'loss') => {
    const targets = bulkTargets(positions, profitOf, which);
    if (!targets.length) return;
    if (armed !== which) return setArmed(which);
    setArmed(null);
    const results = await Promise.all(targets.map((p) => getApi().closePosition(account.id, p.id)));
    const failed = results.filter((r) => !r.ok);
    if (failed.length) toast('error', t('toast.rejected', { error: failed[0]!.error ?? '' }));
  };
  const btn = (which: 'all' | 'profit' | 'loss', label: MessageKey, cls: string) => {
    const n = bulkTargets(positions, profitOf, which).length;
    return (
      <button
        key={which}
        disabled={n === 0}
        onClick={() => void run(which)}
        data-testid={`m-bulk-${which}`}
        className={`flex-1 h-10 rounded-xl text-[12px] font-medium transition-colors disabled:opacity-40 ${armed === which ? 'bg-down text-white' : cls}`}
      >
        {armed === which ? t('tb.confirmBulk', { n }) : `${t(label)} · ${n}`}
      </button>
    );
  };
  return (
    <div className="flex gap-2" data-testid="m-bulk-close">
      {btn('all', 'tb.closeAll', 'bg-panel-2 text-fg')}
      {btn('profit', 'tb.closeProfit', 'bg-up-bg text-up')}
      {btn('loss', 'tb.closeLoss', 'bg-down-bg text-down')}
    </div>
  );
}

function TradeView() {
  const t = useT();
  const positions = useTerminal(selectPositions);
  const orders = useTerminal(selectOrders);
  const title = (label: string, n: number) => (
    <div className="px-1 pt-1 text-[11px] uppercase tracking-wider text-muted">
      {label} <span className="num text-accent">{n}</span>
    </div>
  );
  return (
    <div className="fx-view flex flex-col h-full" data-testid="m-trade">
      <Hero />
      <div className="flex-1 overflow-y-auto px-4 pb-4 flex flex-col gap-2">
        {positions.length === 0 && orders.length === 0 && <div className="p-10 text-center text-muted">{t('tb.empty')}</div>}
        {positions.length > 0 && title(t('tb.positions'), positions.length)}
        <BulkClose positions={positions} />
        {positions.map((p) => (
          <PositionCard key={p.id} p={p} />
        ))}
        {orders.length > 0 && title(t('tb.orders'), orders.length)}
        {orders.map((o) => (
          <OrderCard key={o.id} o={o} />
        ))}
      </div>
    </div>
  );
}

/** A closed position reconstructed from its deals. */
interface ClosedPosition {
  id: string;
  symbol: string;
  side: Side;
  volume: number;
  openPrice: number;
  closePrice: number;
  openTime: number;
  closeTime: number;
  profit: number;
}

function closedPositions(deals: Deal[]): ClosedPosition[] {
  const byPos = new Map<string, Deal[]>();
  for (const d of deals) byPos.set(d.positionId, [...(byPos.get(d.positionId) ?? []), d]);
  const out: ClosedPosition[] = [];
  for (const [id, ds] of byPos) {
    const ins = ds.filter((d) => d.entry === 'in');
    const outs = ds.filter((d) => d.entry === 'out');
    if (!ins.length || !outs.length) continue;
    const vwap = (xs: Deal[]) => xs.reduce((a, d) => a + d.price * d.volume, 0) / xs.reduce((a, d) => a + d.volume, 0);
    out.push({
      id,
      symbol: ds[0]!.symbol,
      side: ins[0]!.side,
      volume: outs.reduce((a, d) => a + d.volume, 0),
      openPrice: vwap(ins),
      closePrice: vwap(outs),
      openTime: Math.min(...ins.map((d) => d.time)),
      closeTime: Math.max(...outs.map((d) => d.time)),
      profit: ds.reduce((a, d) => a + d.profit + d.commission, 0),
    });
  }
  return out.sort((x, y) => y.closeTime - x.closeTime);
}

function ClosedPositionRow({ p }: { p: ClosedPosition }) {
  const digits = useTerminal((s) => s.symbols[p.symbol]?.digits ?? 5);
  return (
    <div className="flex items-center justify-between px-1 py-1.5 border-b border-line/40">
      <div>
        <div className="font-medium">
          {p.symbol} <span className={`text-[11px] uppercase ${p.side === 'buy' ? 'text-up' : 'text-down'}`}>{p.side} {volumeToLots(p.volume)}</span>
        </div>
        <div className="num text-[11px] text-muted">
          {formatPrice(p.openPrice, digits)} → {formatPrice(p.closePrice, digits)} · {formatTimeShort(p.closeTime)}
        </div>
      </div>
      <span className={`num font-semibold ${tone(p.profit)}`}>
        {p.profit > 0 ? '+' : ''}
        {formatMoney(p.profit)}
      </span>
    </div>
  );
}

function OrderHistoryRow({ o }: { o: OrderHistoryEntry }) {
  const digits = useTerminal((s) => s.symbols[o.symbol]?.digits ?? 5);
  const cls = o.status === 'filled' ? 'text-up' : o.status === 'working' ? 'text-accent' : 'text-muted';
  return (
    <div className="flex items-center justify-between px-1 py-1.5 border-b border-line/40" title={o.text}>
      <div>
        <div className="font-medium">
          {o.symbol} <span className={`text-[11px] uppercase ${o.side === 'buy' ? 'text-up' : 'text-down'}`}>{o.side} {o.type.replace('_', ' ')}</span>
        </div>
        <div className="num text-[11px] text-muted">
          {volumeToLots(o.filled)} / {volumeToLots(o.volume)}
          {o.price !== undefined ? ` @ ${formatPrice(o.price, digits)}` : ''}
          {o.avgPrice !== undefined ? ` → ${formatPrice(o.avgPrice, digits)}` : ''} · {formatTimeShort(o.time)}
        </div>
      </div>
      <span className={`text-[11px] uppercase font-semibold ${cls}`}>{o.status}</span>
    </div>
  );
}

/** History in three views like MT5: closed positions, orders, deals. */
function HistoryView() {
  const t = useT();
  const history = useTerminal(selectHistory);
  const accountId = useTerminal((s) => s.activeAccountId);
  const orderHistory = useTerminal((s) => (s.activeAccountId ? s.orderHistory[s.activeAccountId] : undefined));
  const setOrderHistory = useTerminal((s) => s.setOrderHistory);
  const [seg, setSeg] = useState<'positions' | 'orders' | 'deals'>('positions');
  const total = useMemo(() => history.reduce((a, d) => a + d.profit + d.commission, 0), [history]);
  const positions = useMemo(() => closedPositions(history), [history]);
  // Orders are fetched when the view is opened (and whenever a deal arrives).
  useEffect(() => {
    if (seg !== 'orders' || !accountId) return;
    let live = true;
    void getApi()
      .getOrderHistory(accountId)
      .then((o) => live && setOrderHistory(accountId, o));
    return () => {
      live = false;
    };
  }, [seg, accountId, history.length, setOrderHistory]);
  const empty = <div className="p-10 text-center text-muted">{t('tb.empty')}</div>;
  return (
    <div className="fx-view flex flex-col h-full" data-testid="m-history">
      <div className="fx-card mx-4 my-2 px-4 py-2 flex items-end justify-between">
        <div>
          <div className="text-[11px] uppercase tracking-wider text-muted">{t('tb.total')}</div>
          <div className={`num text-[24px] font-semibold leading-tight ${tone(total)}`}>
            {total > 0 ? '+' : ''}
            {formatMoney(total)}
          </div>
        </div>
        <div className="num text-[12px] text-muted">{positions.length}</div>
      </div>
      <div className="mx-4 mb-2 p-1 rounded-full bg-panel-2 flex">
        {(['positions', 'orders', 'deals'] as const).map((x) => (
          <button
            key={x}
            onClick={() => setSeg(x)}
            className={`flex-1 h-9 rounded-full text-[13px] font-medium ${seg === x ? 'bg-panel text-fg shadow' : 'text-muted'}`}
            data-testid={`m-hist-${x}`}
          >
            {t(x === 'deals' ? 'm.deals' : `tb.${x}`)}
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {seg === 'positions' && (positions.length ? positions.slice(0, 300).map((p) => <ClosedPositionRow key={p.id} p={p} />) : empty)}
        {seg === 'orders' && (orderHistory === undefined ? <div className="p-10 text-center text-muted">…</div> : orderHistory.length ? orderHistory.map((o) => <OrderHistoryRow key={o.id} o={o} />) : empty)}
        {seg === 'deals' &&
          (history.length
            ? [...history]
                .reverse()
                .slice(0, 300)
                .map((d) => <DealRow key={d.id} d={d} />)
            : empty)}
      </div>
    </div>
  );
}

function AccountView() {
  const t = useT();
  const m = useMetrics();
  const accounts = useTerminal((s) => s.accounts);
  const activeId = useTerminal((s) => s.activeAccountId);
  const setActive = useTerminal((s) => s.setActiveAccount);
  const theme = useTerminal((s) => s.theme);
  const toggleTheme = useTerminal((s) => s.toggleTheme);
  const lang = useTerminal((s) => s.lang);
  const setLang = useTerminal((s) => s.setLang);
  const askLine = useTerminal((s) => s.showAskLine);
  const toggleAsk = useTerminal((s) => s.toggleAskLine);
  const conn = useTerminal((s) => s.connection);
  const latency = useTerminal((s) => s.latencyMs);
  const claims = useSession((s) => s.claims);
  const logout = useSession((s) => s.logout);
  const stat = (label: string, value: string, cls = '') => (
    <div className="fx-card px-4 py-3">
      <div className="text-[11px] uppercase tracking-wider text-muted">{label}</div>
      <div className={`num text-[17px] font-semibold mt-0.5 ${cls}`}>{value}</div>
    </div>
  );
  const row = (label: string, control: ReactNode) => (
    <div className="flex items-center justify-between px-4 h-14 border-b border-line/40 last:border-0">
      <span>{label}</span>
      {control}
    </div>
  );
  const pill = 'h-9 px-4 rounded-full bg-panel-2 text-[13px] font-medium';
  return (
    <div className="fx-view h-full overflow-y-auto px-4 py-3 flex flex-col gap-3">
      {m && (
        <div className="grid grid-cols-2 gap-3">
          {stat(t('top.balance'), formatMoney(m.balance))}
          {stat(t('top.equity'), formatMoney(m.equity))}
          {stat(t('top.floating'), formatMoney(m.floating), tone(m.floating))}
          {stat(t('top.margin'), formatMoney(m.margin))}
          {stat(t('top.freeMargin'), formatMoney(m.freeMargin))}
          {stat(t('top.marginLevel'), m.marginLevel ? `${m.marginLevel}%` : '—')}
        </div>
      )}
      <div className="fx-card">
        {row(
          t('top.account'),
          <select aria-label={t('top.account')} className="bg-transparent text-right max-w-[60%]" value={activeId ?? ''} onChange={(e) => setActive(e.target.value)}>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.id} · 1:{a.leverage}
                {a.marginMode ? ` · ${t(a.marginMode === 'hedging' ? 'top.hedging' : 'top.netting')}` : ''}
              </option>
            ))}
          </select>,
        )}
        {row(
          t('top.lang'),
          <button className={`${pill} num`} onClick={() => setLang(lang === 'en' ? 'tr' : 'en')}>
            {lang.toUpperCase()}
          </button>,
        )}
        {row(
          t('chart.askLine'),
          <button className={`${pill} ${askLine ? 'bg-accent text-white' : ''}`} onClick={toggleAsk} aria-pressed={askLine} data-testid="m-ask-line">
            {askLine ? 'ON' : 'OFF'}
          </button>,
        )}
        {row(
          t('top.theme'),
          <button className={pill} onClick={toggleTheme} data-testid="m-theme">
            {theme === 'dark' ? '☾' : '☀'}
          </button>,
        )}
        {row(
          t(`conn.${conn}`),
          <span className="num text-muted">{conn === 'connected' && latency !== undefined ? `${latency} ms` : ''}</span>,
        )}
      </div>
      <ChartAttribution className="px-2 text-center" />
      {claims && (
        <div className="fx-card">
          {row(
            claims.email,
            <button className={`${pill} text-down`} data-testid="m-sign-out" onClick={() => void logout({ keepState: true }).then(() => location.reload())}>
              {t('auth.signOut')}
            </button>,
          )}
        </div>
      )}
    </div>
  );
}

function TicketSheet() {
  const t = useT();
  const ticket = useTerminal((s) => s.ticket);
  const close = useTerminal((s) => s.closeTicket);
  if (!ticket) return null;
  return (
    <div className="fixed inset-0 z-40 flex flex-col justify-end bg-black/55" onClick={close} data-testid="m-ticket">
      <div
        className="fx-sheet bg-panel rounded-t-[28px] max-h-[90dvh] overflow-y-auto pb-[env(safe-area-inset-bottom)]"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={t('ticket.title')}
      >
        <div className="mx-auto mt-2 h-1 w-10 rounded-full bg-line" />
        <div className="flex items-center justify-between px-5 pt-2">
          <h2 className="text-[17px] font-semibold">
            {t('ticket.title')} · {ticket.symbol}
          </h2>
          <button className="h-9 w-9 rounded-full bg-panel-2 text-muted" onClick={close} aria-label={t('ticket.close')}>
            ✕
          </button>
        </div>
        <OrderTicket key={`${ticket.symbol}-${ticket.side}-${ticket.type}`} preset={ticket} onDone={close} variant="sheet" />
      </div>
    </div>
  );
}

function MobileToasts() {
  const toasts = useTerminal((s) => s.toasts);
  const dismiss = useTerminal((s) => s.dismissToast);
  return (
    <div className="fixed top-[calc(env(safe-area-inset-top)+8px)] inset-x-3 z-50 flex flex-col gap-2" aria-live="polite">
      {toasts.map((x) => (
        <button
          key={x.id}
          onClick={() => dismiss(x.id)}
          className={`fx-glass fx-view text-left px-4 py-3 rounded-2xl shadow-lg border ${x.kind === 'ok' ? 'border-up/50' : 'border-down/50'}`}
          data-testid="toast"
        >
          <span className={x.kind === 'ok' ? 'text-up' : 'text-down'}>●</span> {x.text}
        </button>
      ))}
    </div>
  );
}

export function MobileApp() {
  const t = useT();
  const [tab, setTab] = useState<Tab>('markets');
  const setChartSymbol = useTerminal((s) => s.setChartSymbol);
  const conn = useTerminal((s) => s.connection);
  const positions = useTerminal(selectPositions).length;
  const account = useTerminal(selectActiveAccount);
  const open = (symbol: string) => {
    setChartSymbol(symbol, 0);
    setTab('chart');
  };
  const dot = conn === 'connected' ? 'bg-up' : conn === 'disconnected' ? 'bg-down' : 'bg-amber-400';
  return (
    <div className="fx-mobile flex flex-col h-[100dvh]" data-testid="mobile-app">
      <header className="flex items-center justify-between px-3 h-6 shrink-0 pt-[env(safe-area-inset-top)] box-content text-[11px] text-muted">
        <span className="font-semibold tracking-tight">
          <span className="text-accent">fx</span>vps.ai
        </span>
        <span className="flex items-center gap-1.5" data-testid="connection" data-state={conn}>
          {account && <span className="num">{account.id}</span>}
          {account?.isDemo && <span className="text-[9px] font-semibold text-amber-500">{t('top.demo')}</span>}
          <span className={`w-1.5 h-1.5 rounded-full ${dot}`} title={t(`conn.${conn}`)} />
        </span>
      </header>
      <main className="flex-1 min-h-0">
        {tab === 'markets' && <Markets onOpen={open} />}
        {tab === 'chart' && <ChartView onSymbols={() => setTab('markets')} />}
        {tab === 'trade' && <TradeView />}
        {tab === 'history' && <HistoryView />}
        {tab === 'account' && <AccountView />}
      </main>
      <nav className="fx-glass shrink-0 flex border-t border-line/60 pb-[env(safe-area-inset-bottom)]" aria-label="Tabs">
        {TABS.map((x) => (
          <button
            key={x.id}
            onClick={() => setTab(x.id)}
            aria-current={tab === x.id}
            data-testid={`m-tab-${x.id}`}
            className={`relative flex-1 h-[58px] flex flex-col items-center justify-center gap-1 text-[10.5px] font-medium ${tab === x.id ? 'text-accent' : 'text-muted'}`}
          >
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <path d={x.icon} />
            </svg>
            {t(x.label)}
            {x.id === 'trade' && positions > 0 && (
              <span className="num absolute top-1.5 left-1/2 ml-2 min-w-4 h-4 px-1 rounded-full bg-accent text-white text-[10px] grid place-items-center">{positions}</span>
            )}
          </button>
        ))}
      </nav>
      <TicketSheet />
      <MobileToasts />
    </div>
  );
}
