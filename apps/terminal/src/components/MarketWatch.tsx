import { memo, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { big, formatPrice, spreadPoints } from '../lib/money';

const Row = memo(function Row({ symbol }: { symbol: string }) {
  const q = useTerminal((s) => s.quotes[symbol]);
  const spec = useTerminal((s) => s.symbols[symbol]);
  const dir = useTerminal((s) => s.tickDir[symbol]);
  const fav = useTerminal((s) => s.favorites.includes(symbol));
  const active = useTerminal((s) => s.charts[s.activeChart]?.symbol === symbol);
  const toggleFavorite = useTerminal((s) => s.toggleFavorite);
  const setChartSymbol = useTerminal((s) => s.setChartSymbol);
  const openTicket = useTerminal((s) => s.openTicket);
  const t = useT();
  if (!spec) return null;
  const change = q && q.dayOpen ? big(q.bid).minus(q.dayOpen).div(q.dayOpen).times(100).toFixed(2) : null;
  const flash = dir === 1 ? 'flash-up' : dir === -1 ? 'flash-down' : '';
  return (
    <div
      role="row"
      data-testid={`mw-row-${symbol}`}
      className={`grid grid-cols-[18px_1fr_74px_74px_34px_48px] items-center h-7 px-2 cursor-pointer hover:bg-hover border-b border-line/50 ${active ? 'bg-hover' : ''}`}
      onClick={() => setChartSymbol(symbol)}
      onDoubleClick={() => openTicket({ symbol })}
      title={spec.description}
    >
      <button
        aria-label={t('mw.favorite')}
        className={`text-[12px] ${fav ? 'text-amber-400' : 'text-muted/50 hover:text-muted'}`}
        onClick={(e) => {
          e.stopPropagation();
          toggleFavorite(symbol);
        }}
      >
        {fav ? '★' : '☆'}
      </button>
      <span className="font-medium truncate">{symbol}</span>
      {q ? (
        <>
          <span key={`b${q.time}`} data-testid={`bid-${symbol}`} className={`num text-right rounded px-1 ${flash}`}>
            {formatPrice(q.bid, spec.digits)}
          </span>
          <span key={`a${q.time}`} className={`num text-right rounded px-1 ${flash}`}>
            {formatPrice(q.ask, spec.digits)}
          </span>
          <span className="num text-right text-muted">{spreadPoints(q, spec.digits)}</span>
          <span className={`num text-right ${change && change.startsWith('-') ? 'text-down' : 'text-up'}`}>
            {change}
          </span>
        </>
      ) : (
        <span className="col-span-4 text-muted">…</span>
      )}
    </div>
  );
});

export function MarketWatch() {
  const t = useT();
  const order = useTerminal((s) => s.symbolOrder);
  const symbols = useTerminal((s) => s.symbols);
  const favorites = useTerminal((s) => s.favorites);
  const [query, setQuery] = useState('');
  const [onlyFav, setOnlyFav] = useState(false);
  const parentRef = useRef<HTMLDivElement>(null);

  const rows = useMemo(() => {
    const q = query.trim().toUpperCase();
    const list = order.filter(
      (s) =>
        (!onlyFav || favorites.includes(s)) &&
        (!q || s.includes(q) || symbols[s]?.description.toUpperCase().includes(q)),
    );
    // favourites first, then the server order
    return [...list].sort((a, b) => Number(favorites.includes(b)) - Number(favorites.includes(a)));
  }, [order, query, onlyFav, favorites, symbols]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 8,
    initialRect: { width: 320, height: 600 },
  });

  return (
    <section className="flex flex-col h-full bg-panel" aria-label={t('mw.title')}>
      <div className="flex items-center justify-between px-2 h-8 border-b border-line">
        <h2 className="font-semibold">{t('mw.title')}</h2>
        <div className="flex text-[11px] rounded border border-line overflow-hidden">
          <button className={`px-2 ${!onlyFav ? 'bg-accent text-white' : ''}`} onClick={() => setOnlyFav(false)}>
            {t('mw.all')}
          </button>
          <button className={`px-2 ${onlyFav ? 'bg-accent text-white' : ''}`} onClick={() => setOnlyFav(true)}>
            ★ {t('mw.favorites')}
          </button>
        </div>
      </div>
      <div className="p-2 border-b border-line">
        <input
          id="mw-search"
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('mw.search')}
          aria-label={t('mw.search')}
          className="w-full bg-panel-2 border border-line rounded px-2 py-1 outline-none focus:border-accent"
        />
      </div>
      <div className="grid grid-cols-[18px_1fr_74px_74px_34px_48px] px-2 h-6 items-center text-[10px] uppercase text-muted border-b border-line">
        <span />
        <span>{t('mw.symbol')}</span>
        <span className="text-right">{t('mw.bid')}</span>
        <span className="text-right">{t('mw.ask')}</span>
        <span className="text-right">{t('mw.spread')}</span>
        <span className="text-right">{t('mw.change')}</span>
      </div>
      <div ref={parentRef} className="flex-1 overflow-auto" role="table" aria-label={t('mw.title')}>
        {rows.length === 0 && <div className="p-3 text-muted">{t('mw.empty')}</div>}
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((v) => (
            <div key={rows[v.index]} style={{ position: 'absolute', top: 0, left: 0, right: 0, transform: `translateY(${v.start}px)` }}>
              <Row symbol={rows[v.index]!} />
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
