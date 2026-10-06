import { memo, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useT } from '../hooks';
import { PinButton } from './SideDock';
import { useTerminal } from '../store/terminal';
import { big, formatPrice, spreadPoints } from '@fxvps/trading-core';
import { ContextMenu, type MenuItem } from './ContextMenu';
import { SymbolSpecDialog } from './SymbolSpecDialog';
import { MW_COLUMNS, mwGridTemplate, type MwColumnId } from './mwColumns';

type Menu = { symbol: string; x: number; y: number };

const Row = memo(function Row({ symbol, onMenu, columns, grid }: { symbol: string; onMenu: (m: Menu) => void; columns: MwColumnId[]; grid: string }) {
  const q = useTerminal((s) => s.quotes[symbol]);
  // Net lots of the active account on this symbol (centi-lots, signed).
  const myLots = useTerminal((s) => {
    const ps = s.activeAccountId ? s.positions[s.activeAccountId] : undefined;
    return ps ? ps.filter((p) => p.symbol === symbol).reduce((a, p) => a + (p.side === 'buy' ? p.volume : -p.volume), 0) : 0;
  });
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
  const px = (v: number) => formatPrice(v, spec.digits);
  const signed = (v: string) => (v.startsWith('-') ? 'text-down' : 'text-up');
  const cell = (id: MwColumnId) => {
    if (!q) return <span key={id} className="text-muted text-right">…</span>;
    switch (id) {
      case 'bid':
        return <span key={`b${q.time}`} data-testid={`bid-${symbol}`} className={`num text-right rounded px-1 ${flash}`}>{px(q.bid)}</span>;
      case 'ask':
        return <span key={`a${q.time}`} className={`num text-right rounded px-1 ${flash}`}>{px(q.ask)}</span>;
      case 'spread':
        return <span key={id} className="num text-right text-muted">{spreadPoints(q, spec.digits)}</span>;
      case 'change':
        return <span key={id} className={`num text-right ${change ? signed(change) : ''}`}>{change}</span>;
      case 'changePts': {
        const pts = q.dayOpen ? Math.round((q.bid - q.dayOpen) * 10 ** spec.digits) : null;
        return <span key={id} className={`num text-right ${pts === null ? '' : pts < 0 ? 'text-down' : 'text-up'}`}>{pts ?? ''}</span>;
      }
      case 'time':
        return <span key={id} className="num text-right text-muted">{new Date(q.time).toLocaleTimeString([], { hour12: false })}</span>;
      case 'open':
        return <span key={id} className="num text-right text-muted">{q.dayOpen ? px(q.dayOpen) : ''}</span>;
      case 'high':
        return <span key={id} className="num text-right text-muted">{q.dayHigh ? px(q.dayHigh) : ''}</span>;
      case 'low':
        return <span key={id} className="num text-right text-muted">{q.dayLow ? px(q.dayLow) : ''}</span>;
      case 'myLots':
        return <span key={id} className={`num text-right ${myLots === 0 ? 'text-muted' : myLots > 0 ? 'text-up' : 'text-down'}`}>{myLots === 0 ? '' : (myLots / 100).toFixed(2)}</span>;
    }
  };
  return (
    <div
      role="row"
      data-testid={`mw-row-${symbol}`}
      style={{ gridTemplateColumns: grid }}
      className={`grid items-center h-7 px-2 cursor-pointer hover:bg-hover border-b border-line/50 ${active ? 'bg-hover' : ''}`}
      onClick={() => setChartSymbol(symbol)}
      onDoubleClick={() => openTicket({ symbol })}
      onContextMenu={(e) => {
        e.preventDefault();
        onMenu({ symbol, x: e.clientX, y: e.clientY });
      }}
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
      {MW_COLUMNS.filter((c) => columns.includes(c.id)).map((c) => cell(c.id))}
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
  const [menu, setMenu] = useState<Menu | null>(null);
  const [specOf, setSpecOf] = useState<string | null>(null);
  const [colsOpen, setColsOpen] = useState(false);
  const columns = useTerminal((s) => s.mwColumns);
  const toggleColumn = useTerminal((s) => s.toggleMwColumn);
  const grid = mwGridTemplate(columns);
  const toggleFavorite = useTerminal((s) => s.toggleFavorite);
  const setChartSymbol = useTerminal((s) => s.setChartSymbol);
  const openChart = useTerminal((s) => s.openChart);
  const openTicket = useTerminal((s) => s.openTicket);
  const isFav = (s: string) => favorites.includes(s);
  const menuItems = (s: string): MenuItem[] => [
    { label: t('mw.menu.chart'), onClick: () => setChartSymbol(s) },
    { label: t('mw.menu.newChart'), onClick: () => openChart(s) },
    { label: t('mw.menu.buy'), onClick: () => openTicket({ symbol: s, side: 'buy' }), separator: true },
    { label: t('mw.menu.sell'), onClick: () => openTicket({ symbol: s, side: 'sell' }) },
    { label: t('mw.menu.pending'), onClick: () => openTicket({ symbol: s, type: 'limit' }) },
    { label: t('mw.menu.depth'), onClick: () => setChartSymbol(s), separator: true },
    { label: t('mw.menu.spec'), onClick: () => setSpecOf(s) },
    { label: isFav(s) ? t('mw.menu.unfavorite') : t('mw.menu.favorite'), onClick: () => toggleFavorite(s), separator: true },
  ];
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
        <PinButton side="left" />
        <div className="relative">
          <button
            className={`w-5 h-5 grid place-items-center rounded text-[11px] ${colsOpen ? 'text-fg bg-panel-2' : 'text-muted hover:text-fg hover:bg-panel-2'}`}
            title={t('mw.columns')}
            aria-label={t('mw.columns')}
            aria-expanded={colsOpen}
            onClick={() => setColsOpen((o) => !o)}
            data-testid="mw-columns"
          >
            ⚙
          </button>
          {colsOpen && (
            <div className="absolute right-0 top-6 z-30 w-48 rounded-md border border-line bg-panel shadow-xl py-1 text-[12px]" onMouseLeave={() => setColsOpen(false)}>
              {MW_COLUMNS.map((c) => (
                <label key={c.id} className="flex items-center gap-2 px-3 h-7 hover:bg-hover cursor-pointer">
                  <input type="checkbox" checked={columns.includes(c.id)} onChange={() => toggleColumn(c.id)} />
                  {t(c.label)}
                </label>
              ))}
            </div>
          )}
        </div>
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
      <div className="grid px-2 h-6 items-center text-[10px] uppercase text-muted border-b border-line" style={{ gridTemplateColumns: grid }}>
        <span />
        <span>{t('mw.symbol')}</span>
        {MW_COLUMNS.filter((c) => columns.includes(c.id)).map((c) => (
          <span key={c.id} className="text-right truncate">{t(c.label)}</span>
        ))}
      </div>
      <div ref={parentRef} className="flex-1 overflow-auto" role="table" aria-label={t('mw.title')}>
        {rows.length === 0 && <div className="p-3 text-muted">{t('mw.empty')}</div>}
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((v) => (
            <div key={rows[v.index]} style={{ position: 'absolute', top: 0, left: 0, right: 0, transform: `translateY(${v.start}px)` }}>
              <Row symbol={rows[v.index]!} onMenu={setMenu} columns={columns} grid={grid} />
            </div>
          ))}
        </div>
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.symbol)} onClose={() => setMenu(null)} testId="mw-menu" />}
      {specOf && <SymbolSpecDialog symbol={specOf} onClose={() => setSpecOf(null)} />}
    </section>
  );
}
