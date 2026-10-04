import { memo, useMemo, useState } from 'react';
import type { Deal, PendingOrder, Position } from '../api/types';
import { useRates, useT } from '../hooks';
import { getApi } from '../store/api';
import { selectActiveAccount, selectHistory, selectOrders, selectPositions, useTerminal, type ToolboxTab } from '../store/terminal';
import { closePrice, distanceToPips, formatMoney, formatPrice, lotsToVolume, pipsToDistance, positionProfit, volumeToLots } from '../lib/money';
import { formatTimeShort as formatTime, parseDecimal } from '../lib/format';
import { VirtualTable } from './VirtualTable';

const TABS: ToolboxTab[] = ['positions', 'orders', 'history', 'journal'];
const POS_COLS = '48px 104px 60px 36px 44px 68px 68px 68px 40px 68px 48px 72px minmax(250px,1fr)';
const ORD_COLS = '48px 104px 60px 104px 44px 140px 68px 68px 68px 104px minmax(110px,1fr)';
const HIST_COLS = '48px 104px 60px 36px 36px 44px 80px 64px 80px 70px';
const JOUR_COLS = '140px 60px 1fr';

function Pnl({ v }: { v: number }) {
  return <span className={`num text-right ${v > 0 ? 'text-up' : v < 0 ? 'text-down' : ''}`}>{formatMoney(v)}</span>;
}

const sideCls = (s: string) => (s === 'buy' ? 'text-up' : 'text-down');

const PositionRow = memo(function PositionRow({ p }: { p: Position }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[p.symbol]);
  const q = useTerminal((s) => s.quotes[p.symbol]);
  const account = useTerminal(selectActiveAccount);
  const toast = useTerminal((s) => s.toast);
  const rates = useRates();
  const [edit, setEdit] = useState(false);
  const [sl, setSl] = useState(p.sl !== undefined ? String(p.sl) : '');
  const [tp, setTp] = useState(p.tp !== undefined ? String(p.tp) : '');
  const [trail, setTrail] = useState('');
  const [partial, setPartial] = useState(volumeToLots(Math.max(spec?.volumeStep ?? 1, Math.floor(p.volume / 2))));
  if (!spec || !q || !account) return null;
  let profit = 0;
  try {
    profit = positionProfit(p, spec, q, account.currency, rates);
  } catch {
    /* no rate yet */
  }
  const report = (r: { ok: boolean; error?: string; price?: number }) =>
    r.ok ? toast('ok', t('toast.closed', { id: p.id, price: r.price ?? '' })) : toast('error', t('toast.rejected', { error: r.error ?? '' }));
  const close = async (volume?: number) => report(await getApi().closePosition(account.id, p.id, volume));
  const startEdit = () => {
    setSl(p.sl !== undefined ? String(p.sl) : '');
    setTp(p.tp !== undefined ? String(p.tp) : '');
    setTrail(p.trailing !== undefined ? String(distanceToPips(p.trailing, spec)) : '');
    setEdit(true);
  };
  const save = async () => {
    const tr = parseDecimal(trail);
    const r = await getApi().modifyPosition(account.id, p.id, parseDecimal(sl), parseDecimal(tp), tr ? pipsToDistance(tr, spec) : undefined);
    if (r.ok) setEdit(false);
    else toast('error', t('toast.rejected', { error: r.error }));
  };
  const inp = 'num w-full bg-panel-2 border border-line rounded px-1';
  return (
    <>
      <span className="num">{p.id}</span>
      <span className="num text-muted">{formatTime(p.openTime)}</span>
      <span className="font-medium">{p.symbol}</span>
      <span className={sideCls(p.side)}>{p.side}</span>
      <span className="num">{volumeToLots(p.volume)}</span>
      <span className="num">{formatPrice(p.openPrice, spec.digits)}</span>
      {edit ? (
        <>
          <input className={inp} value={sl} onChange={(e) => setSl(e.target.value)} aria-label={t('tb.sl')} data-testid={`sl-input-${p.id}`} />
          <input className={inp} value={tp} onChange={(e) => setTp(e.target.value)} aria-label={t('tb.tp')} data-testid={`tp-input-${p.id}`} />
          <input className={inp} value={trail} onChange={(e) => setTrail(e.target.value)} aria-label={t('tb.trailing')} />
        </>
      ) : (
        <>
          <span className="num" data-testid={`sl-${p.id}`}>{p.sl !== undefined ? formatPrice(p.sl, spec.digits) : '—'}</span>
          <span className="num" data-testid={`tp-${p.id}`}>{p.tp !== undefined ? formatPrice(p.tp, spec.digits) : '—'}</span>
          <span className="num text-muted">{p.trailing !== undefined ? distanceToPips(p.trailing, spec) : '—'}</span>
        </>
      )}
      <span className="num">{formatPrice(closePrice(p.side, q), spec.digits)}</span>
      <span className="num text-muted">{formatMoney(p.commission)}</span>
      <Pnl v={profit} />
      <span className="flex gap-1 justify-end">
        {edit ? (
          <>
            <button className="px-1.5 rounded bg-accent text-white" onClick={() => void save()} data-testid={`save-${p.id}`}>{t('tb.save')}</button>
            <button className="px-1.5 rounded border border-line" onClick={() => setEdit(false)}>{t('tb.cancel')}</button>
          </>
        ) : (
          <button className="px-1.5 rounded border border-line" onClick={startEdit} data-testid={`modify-${p.id}`}>{t('tb.modify')}</button>
        )}
        <input className="num w-12 bg-panel-2 border border-line rounded px-1" value={partial} onChange={(e) => setPartial(e.target.value)} aria-label={t('tb.closePartial')} data-testid={`partial-input-${p.id}`} />
        <button
          data-testid={`partial-${p.id}`}
          className="px-1.5 rounded border border-line"
          onClick={() => {
            const v = lotsToVolume(partial);
            if (v) void close(v);
          }}
        >
          {t('tb.closePartial')}
        </button>
        <button className="px-1.5 rounded bg-down/90 text-white" onClick={() => void close()} data-testid={`close-${p.id}`}>
          {t('tb.close')}
        </button>
      </span>
    </>
  );
});

function Positions() {
  const t = useT();
  const positions = useTerminal(selectPositions);
  return (
    <VirtualTable
      testId="positions-table"
      columns={POS_COLS}
      rows={positions}
      rowKey={(p) => p.id}
      renderRow={(p) => <PositionRow p={p} />}
      empty={t('tb.empty')}
      header={
        <>
          <span>{t('tb.id')}</span><span>{t('tb.time')}</span><span>{t('tb.symbol')}</span><span>{t('tb.type')}</span>
          <span>{t('tb.volume')}</span><span>{t('tb.openPrice')}</span><span>{t('tb.sl')}</span><span>{t('tb.tp')}</span>
          <span>{t('tb.trailing')}</span><span>{t('tb.current')}</span><span>{t('tb.commission')}</span><span className="text-right">{t('tb.profit')}</span><span />
        </>
      }
    />
  );
}

function OrderRow({ o }: { o: PendingOrder }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[o.symbol]);
  const q = useTerminal((s) => s.quotes[o.symbol]);
  const accountId = useTerminal((s) => s.activeAccountId);
  const toast = useTerminal((s) => s.toast);
  const [edit, setEdit] = useState(false);
  const [price, setPrice] = useState('');
  const [limit, setLimit] = useState('');
  const [lots, setLots] = useState('');
  const [sl, setSl] = useState('');
  const [tp, setTp] = useState('');
  if (!spec || !accountId) return null;
  const startEdit = () => {
    setPrice(String(o.price));
    setLimit(o.limitPrice !== undefined ? String(o.limitPrice) : '');
    setLots(volumeToLots(o.volume));
    setSl(o.sl !== undefined ? String(o.sl) : '');
    setTp(o.tp !== undefined ? String(o.tp) : '');
    setEdit(true);
  };
  const save = async () => {
    const volume = lotsToVolume(lots);
    const r = await getApi().modifyOrder(accountId, o.id, {
      price: parseDecimal(price),
      ...(o.type === 'stop_limit' ? { limitPrice: parseDecimal(limit) } : {}),
      ...(volume !== null ? { volume } : {}),
      sl: parseDecimal(sl),
      tp: parseDecimal(tp),
    });
    if (r.ok) setEdit(false);
    else toast('error', t('toast.rejected', { error: r.error }));
  };
  const inp = 'num w-full bg-panel-2 border border-line rounded px-1';
  return (
    <>
      <span className="num">{o.id}</span>
      <span className="num text-muted">{formatTime(o.createdAt)}</span>
      <span className="font-medium">{o.symbol}</span>
      <span className={sideCls(o.side)}>{o.side} {o.type.replace('_', ' ')}{o.triggered ? ' ✓' : ''}</span>
      {edit ? (
        <>
          <input className={inp} value={lots} onChange={(e) => setLots(e.target.value)} aria-label={t('tb.volume')} data-testid={`order-volume-${o.id}`} />
          <span className="flex gap-0.5">
            <input className={inp} value={price} onChange={(e) => setPrice(e.target.value)} aria-label={t('tb.price')} data-testid={`order-price-${o.id}`} />
            {o.type === 'stop_limit' && <input className={inp} value={limit} onChange={(e) => setLimit(e.target.value)} aria-label={t('ticket.limitPrice')} />}
          </span>
          <input className={inp} value={sl} onChange={(e) => setSl(e.target.value)} aria-label={t('tb.sl')} />
          <input className={inp} value={tp} onChange={(e) => setTp(e.target.value)} aria-label={t('tb.tp')} />
        </>
      ) : (
        <>
          <span className="num">{volumeToLots(o.volume)}</span>
          <span className="num" data-testid={`order-px-${o.id}`}>{formatPrice(o.price, spec.digits)}{o.limitPrice !== undefined ? ` / ${formatPrice(o.limitPrice, spec.digits)}` : ''}</span>
          <span className="num">{o.sl !== undefined ? formatPrice(o.sl, spec.digits) : '—'}</span>
          <span className="num">{o.tp !== undefined ? formatPrice(o.tp, spec.digits) : '—'}</span>
        </>
      )}
      <span className="num">{q ? formatPrice(o.side === 'buy' ? q.ask : q.bid, spec.digits) : '—'}</span>
      <span className="num text-muted">{o.expiry ? formatTime(o.expiry) : 'GTC'}</span>
      <span className="flex gap-1 justify-end">
        {edit ? (
          <>
            <button className="px-1.5 rounded bg-accent text-white" onClick={() => void save()} data-testid={`order-save-${o.id}`}>{t('tb.save')}</button>
            <button className="px-1.5 rounded border border-line" onClick={() => setEdit(false)}>{t('tb.cancel')}</button>
          </>
        ) : (
          <>
            <button className="px-1.5 rounded border border-line" onClick={startEdit} data-testid={`order-edit-${o.id}`}>{t('tb.edit')}</button>
            <button className="px-1.5 rounded border border-line" onClick={() => void getApi().cancelOrder(accountId, o.id)} data-testid={`order-cancel-${o.id}`}>{t('tb.cancel')}</button>
          </>
        )}
      </span>
    </>
  );
}

function Orders() {
  const t = useT();
  const orders = useTerminal(selectOrders);
  return (
    <VirtualTable
      testId="orders-table"
      columns={ORD_COLS}
      rows={orders}
      rowKey={(o) => o.id}
      renderRow={(o) => <OrderRow o={o} />}
      empty={t('tb.empty')}
      header={
        <>
          <span>{t('tb.id')}</span><span>{t('tb.time')}</span><span>{t('tb.symbol')}</span><span>{t('tb.type')}</span>
          <span>{t('tb.volume')}</span><span>{t('tb.price')}</span><span>{t('tb.sl')}</span><span>{t('tb.tp')}</span>
          <span>{t('tb.current')}</span><span>{t('tb.expiry')}</span><span />
        </>
      }
    />
  );
}

function History() {
  const t = useT();
  const deals = useTerminal(selectHistory);
  const symbols = useTerminal((s) => s.symbols);
  const rows = useMemo(() => [...deals].reverse(), [deals]);
  const total = useMemo(() => deals.reduce((a, d) => a + d.profit + d.commission, 0), [deals]);
  const accountId = useTerminal((s) => s.activeAccountId);
  const setHistory = useTerminal((s) => s.setHistory);
  const refresh = async () => {
    if (accountId) setHistory(accountId, await getApi().getHistory(accountId));
  };
  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 min-h-0">
        <VirtualTable<Deal>
          testId="history-table"
          columns={HIST_COLS}
          rows={rows}
          rowKey={(d) => d.id}
          empty={t('tb.empty')}
          renderRow={(d) => (
            <>
              <span className="num">{d.positionId}</span>
              <span className="num text-muted">{formatTime(d.time)}</span>
              <span className="font-medium">{d.symbol}</span>
              <span className={sideCls(d.side)}>{d.side}</span>
              <span>{d.entry}</span>
              <span className="num">{volumeToLots(d.volume)}</span>
              <span className="num">{formatPrice(d.price, symbols[d.symbol]?.digits ?? 5)}</span>
              <span className="num text-muted">{formatMoney(d.commission)}</span>
              <Pnl v={d.profit} />
              <span className="text-muted">{d.reason}</span>
            </>
          )}
          header={
            <>
              <span>{t('tb.id')}</span><span>{t('tb.time')}</span><span>{t('tb.symbol')}</span><span>{t('tb.type')}</span>
              <span>{t('tb.entry')}</span><span>{t('tb.volume')}</span><span>{t('tb.price')}</span><span>{t('tb.commission')}</span>
              <span className="text-right">{t('tb.profit')}</span><span>{t('tb.reason')}</span>
            </>
          }
        />
      </div>
      <div className="flex justify-end gap-2 px-3 h-6 items-center border-t border-line">
        <button className="mr-auto text-muted hover:text-fg" onClick={() => void refresh()} data-testid="history-refresh">
          {t('tb.historyRefresh')}
        </button>
        <span className="text-muted">{t('tb.total')}</span>
        <Pnl v={total} />
      </div>
    </div>
  );
}

function Journal() {
  const t = useT();
  const journal = useTerminal((s) => s.journal);
  const rows = useMemo(() => [...journal].reverse(), [journal]);
  return (
    <VirtualTable
      testId="journal-table"
      columns={JOUR_COLS}
      rows={rows}
      rowKey={(j) => j.id}
      empty={t('tb.empty')}
      renderRow={(j) => (
        <>
          <span className="num text-muted">{formatTime(j.time)}</span>
          <span className={j.level === 'error' ? 'text-down' : j.level === 'warn' ? 'text-amber-500' : 'text-muted'}>{j.level}</span>
          <span className="truncate">{j.message}</span>
        </>
      )}
      header={
        <>
          <span>{t('tb.time')}</span><span>{t('tb.type')}</span><span>{t('tb.message')}</span>
        </>
      }
    />
  );
}

export function Toolbox() {
  const t = useT();
  const tab = useTerminal((s) => s.toolboxTab);
  const setTab = useTerminal((s) => s.setToolboxTab);
  const nPos = useTerminal((s) => selectPositions(s).length);
  const nOrd = useTerminal((s) => selectOrders(s).length);
  const account = useTerminal(selectActiveAccount);
  const positions = useTerminal(selectPositions);
  const counts: Partial<Record<ToolboxTab, number>> = { positions: nPos, orders: nOrd };
  return (
    <section className="flex flex-col h-full bg-panel">
      <div className="flex items-center h-8 border-b border-line shrink-0" role="tablist">
        {TABS.map((k) => (
          <button
            key={k}
            role="tab"
            aria-selected={tab === k}
            onClick={() => setTab(k)}
            className={`px-3 h-full border-b-2 ${tab === k ? 'border-accent text-fg' : 'border-transparent text-muted hover:text-fg'}`}
            data-testid={`tab-${k}`}
          >
            {t(`tb.${k}`)}
            {counts[k] ? <span className="ml-1 num text-[10px] px-1 rounded bg-panel-2">{counts[k]}</span> : null}
          </button>
        ))}
        {tab === 'positions' && positions.length > 0 && account && (
          <button
            className="ml-auto mr-2 px-2 py-0.5 rounded border border-line text-muted hover:text-fg"
            onClick={() => positions.forEach((p) => void getApi().closePosition(account.id, p.id))}
          >
            {t('tb.closeAll')}
          </button>
        )}
      </div>
      <div className="flex-1 min-h-0 overflow-x-auto">
        {tab === 'positions' && <Positions />}
        {tab === 'orders' && <Orders />}
        {tab === 'history' && <History />}
        {tab === 'journal' && <Journal />}
      </div>
    </section>
  );
}
