import { memo, useEffect, useMemo, useState } from 'react';
import type { Deal, PendingOrder, Position } from '@fxvps/trading-core';
import { useRates, useT } from '../hooks';
import { getApi } from '../store/api';
import { bulkTargets, selectActiveAccount, selectHistory, selectOrders, selectPositions, useTerminal, type ToolboxTab } from '../store/terminal';
import { closePrice, distanceToPips, formatMoney, formatPrice, lotsToVolume, pipsToDistance, positionProfit, volumeToLots } from '@fxvps/trading-core';
import { formatTimeShort as formatTime, parseDecimal } from '@fxvps/trading-core';
import { VirtualTable } from './VirtualTable';
import { Modal } from './Dialogs';

const TABS: ToolboxTab[] = ['positions', 'orders', 'history', 'journal'];
const POS_COLS = '48px 104px 60px 36px 44px 68px 68px 68px 40px 68px 48px 72px minmax(250px,1fr)';
const ORD_COLS = '48px 104px 60px 104px 44px 140px 68px 68px 68px 104px minmax(110px,1fr)';
const HIST_COLS = '48px 104px 60px 36px 36px 44px 80px 64px 80px 70px';
const JOUR_COLS = '140px 60px 1fr';

function Pnl({ v }: { v: number }) {
  return <span className={`num text-right ${v > 0 ? 'text-up' : v < 0 ? 'text-down' : ''}`}>{formatMoney(v)}</span>;
}

const sideCls = (s: string) => (s === 'buy' ? 'text-up' : 'text-down');

type QuickField = 'sl' | 'tp' | 'trail';

/** One click on an empty "—" (or a value) opens an input right there: Enter saves, Esc cancels. */
function QuickCell({ value, placeholder, onSave, onCancel, testId }: { value: string; placeholder: string; onSave: (v: string) => void; onCancel: () => void; testId: string }) {
  const [v, setV] = useState(value);
  return (
    <input
      autoFocus
      className="num w-full bg-panel-2 border border-accent rounded px-1"
      value={v}
      placeholder={placeholder}
      onChange={(e) => setV(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') onSave(v);
        else if (e.key === 'Escape') onCancel();
      }}
      onBlur={onCancel}
      aria-label={placeholder}
      data-testid={testId}
    />
  );
}

const PositionRow = memo(function PositionRow({ p }: { p: Position }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[p.symbol]);
  const q = useTerminal((s) => s.quotes[p.symbol]);
  const account = useTerminal(selectActiveAccount);
  const toast = useTerminal((s) => s.toast);
  const rates = useRates();
  const [edit, setEdit] = useState(false);
  const [quick, setQuick] = useState<QuickField | null>(null);
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
  // Quick edit of one field; the other two keep their current values.
  const quickSave = async (field: QuickField, raw: string) => {
    const v = parseDecimal(raw);
    const nsl = field === 'sl' ? v : p.sl;
    const ntp = field === 'tp' ? v : p.tp;
    const ntr = field === 'trail' ? (v ? pipsToDistance(v, spec) : undefined) : p.trailing;
    const r = await getApi().modifyPosition(account.id, p.id, nsl, ntp, ntr);
    if (r.ok) toast('ok', t('tb.quickSaved', { id: p.id }));
    else toast('error', t('toast.rejected', { error: r.error }));
    setQuick(null);
  };
  const inp = 'num w-full bg-panel-2 border border-line rounded px-1';
  const cell = (field: QuickField, shown: string, current: string, testId: string, muted = false) =>
    quick === field ? (
      <QuickCell value={current} placeholder={t(field === 'sl' ? 'tb.sl' : field === 'tp' ? 'tb.tp' : 'tb.trailing')} onSave={(v) => void quickSave(field, v)} onCancel={() => setQuick(null)} testId={`${testId}-quick-${p.id}`} />
    ) : (
      <button
        type="button"
        className={`num text-left rounded px-0.5 hover:bg-panel-2 hover:ring-1 hover:ring-line ${muted ? 'text-muted' : ''}`}
        title={t('tb.quickHint')}
        onClick={() => setQuick(field)}
        data-testid={`${testId}-${p.id}`}
      >
        {shown}
      </button>
    );
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
          {cell('sl', p.sl !== undefined ? formatPrice(p.sl, spec.digits) : '—', p.sl !== undefined ? String(p.sl) : '', 'sl')}
          {cell('tp', p.tp !== undefined ? formatPrice(p.tp, spec.digits) : '—', p.tp !== undefined ? String(p.tp) : '', 'tp')}
          {cell('trail', p.trailing !== undefined ? String(distanceToPips(p.trailing, spec)) : '—', p.trailing !== undefined ? String(distanceToPips(p.trailing, spec)) : '', 'trail', true)}
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

/** Detail card of one open position (row click): live P&L, protection, sizing, exits. */
function PositionCard({ id, onClose }: { id: string; onClose: () => void }) {
  const t = useT();
  const p = useTerminal((s) => selectPositions(s).find((x) => x.id === id));
  const spec = useTerminal((s) => (p ? s.symbols[p.symbol] : undefined));
  const q = useTerminal((s) => (p ? s.quotes[p.symbol] : undefined));
  const account = useTerminal(selectActiveAccount);
  const toast = useTerminal((s) => s.toast);
  const openChart = useTerminal((s) => s.openChart);
  const openTicket = useTerminal((s) => s.openTicket);
  const rates = useRates();
  const [sl, setSl] = useState(p?.sl !== undefined ? String(p.sl) : '');
  const [tp, setTp] = useState(p?.tp !== undefined ? String(p.tp) : '');
  const [trail, setTrail] = useState(p?.trailing !== undefined && spec ? String(distanceToPips(p.trailing, spec)) : '');
  const [lots, setLots] = useState(p ? volumeToLots(Math.max(spec?.volumeStep ?? 1, Math.floor(p.volume / 2))) : '');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!p) onClose(); // closed meanwhile
  }, [p, onClose]);
  if (!p || !spec || !q || !account) return null;
  let profit = 0;
  try {
    profit = positionProfit(p, spec, q, account.currency, rates);
  } catch {
    /* no rate yet */
  }
  const cur = closePrice(p.side, q);
  const pips = distanceToPips(Math.abs(cur - p.openPrice), spec) * (cur >= p.openPrice ? (p.side === 'buy' ? 1 : -1) : p.side === 'buy' ? -1 : 1);
  const run = async (f: () => Promise<{ ok: boolean; error?: string; price?: number }>, okMsg?: string) => {
    setBusy(true);
    try {
      const r = await f();
      if (r.ok) toast('ok', okMsg ?? t('toast.closed', { id: p.id, price: r.price ?? '' }));
      else toast('error', t('toast.rejected', { error: r.error ?? '' }));
      return r.ok;
    } finally {
      setBusy(false);
    }
  };
  const api = getApi();
  const modify = (nsl?: number, ntp?: number, ntr?: number) => run(() => api.modifyPosition(account.id, p.id, nsl, ntp, ntr), t('tb.quickSaved', { id: p.id }));
  const saveProtection = () => {
    const tr = parseDecimal(trail);
    void modify(parseDecimal(sl), parseDecimal(tp), tr ? pipsToDistance(tr, spec) : undefined);
  };
  const breakEven = () => {
    setSl(String(p.openPrice));
    void modify(p.openPrice, p.tp, p.trailing);
  };
  const closePart = (fraction?: number) => {
    const v = fraction ? Math.max(spec.volumeStep, Math.round((p.volume * fraction) / spec.volumeStep) * spec.volumeStep) : lotsToVolume(lots);
    if (v) void run(() => api.closePosition(account.id, p.id, Math.min(v, p.volume)));
  };
  const reverse = async () => {
    const ok = await run(() => api.closePosition(account.id, p.id));
    if (ok) await run(() => api.placeOrder({ accountId: account.id, symbol: p.symbol, side: p.side === 'buy' ? 'sell' : 'buy', type: 'market', volume: p.volume }), t('tb.reversed', { id: p.id }));
    onClose();
  };
  const row = 'flex items-center justify-between gap-3';
  const inp = 'num w-28 bg-panel-2 border border-line rounded px-1.5 h-7';
  const btn = 'px-2 h-7 rounded border border-line hover:bg-hover disabled:opacity-40';
  return (
    <Modal title={t('tb.detail', { id: p.id })} onClose={onClose} width="w-[520px]">
      <div className="p-3 grid gap-3 text-[12px]" data-testid={`position-card-${p.id}`}>
        <div className="grid grid-cols-3 gap-2">
          <div><div className="text-muted text-[10px] uppercase">{t('tb.symbol')}</div><div className="font-medium">{p.symbol} <span className={sideCls(p.side)}>{p.side}</span> {volumeToLots(p.volume)}</div></div>
          <div><div className="text-muted text-[10px] uppercase">{t('tb.openPrice')} → {t('tb.current')}</div><div className="num">{formatPrice(p.openPrice, spec.digits)} → {formatPrice(cur, spec.digits)} <span className={pips >= 0 ? 'text-up' : 'text-down'}>({pips >= 0 ? '+' : ''}{pips.toFixed(1)} pip)</span></div></div>
          <div><div className="text-muted text-[10px] uppercase">{t('tb.profit')}</div><div className="num text-[14px]"><Pnl v={profit} /> <span className="text-muted text-[11px]">{t('tb.commission')} {formatMoney(p.commission)} · swap {formatMoney(p.swap)}</span></div></div>
        </div>
        <div className="text-muted text-[11px]">{t('tb.time')}: {formatTime(p.openTime)} · {t('tb.id')} {p.id}</div>

        <div className="border-t border-line pt-2 grid gap-2">
          <div className="text-[10px] uppercase text-muted">{t('tb.protection')}</div>
          <div className={row}>
            <label className="flex items-center gap-2">{t('tb.sl')} <input className={inp} value={sl} onChange={(e) => setSl(e.target.value)} data-testid="card-sl" /></label>
            <label className="flex items-center gap-2">{t('tb.tp')} <input className={inp} value={tp} onChange={(e) => setTp(e.target.value)} data-testid="card-tp" /></label>
            <label className="flex items-center gap-2">{t('tb.trailing')} <input className={`${inp} w-16`} value={trail} onChange={(e) => setTrail(e.target.value)} placeholder="pip" /></label>
          </div>
          <div className="flex flex-wrap gap-1.5">
            <button className="px-2 h-7 rounded bg-accent text-white disabled:opacity-40" disabled={busy} onClick={saveProtection} data-testid="card-save">{t('tb.save')}</button>
            <button className={btn} disabled={busy} onClick={breakEven} data-testid="card-breakeven">{t('tb.breakEven')}</button>
            <button className={btn} disabled={busy} onClick={() => { setSl(''); setTp(''); setTrail(''); void modify(undefined, undefined, undefined); }}>{t('tb.clearProtection')}</button>
          </div>
        </div>

        <div className="border-t border-line pt-2 grid gap-2">
          <div className="text-[10px] uppercase text-muted">{t('tb.exits')}</div>
          <div className="flex flex-wrap items-center gap-1.5">
            <input className={`${inp} w-20`} value={lots} onChange={(e) => setLots(e.target.value)} aria-label={t('tb.closePartial')} data-testid="card-lots" />
            <button className={btn} disabled={busy} onClick={() => closePart()} data-testid="card-partial">{t('tb.closePartial')}</button>
            {[0.25, 0.5, 0.75].map((f) => (
              <button key={f} className={btn} disabled={busy} onClick={() => closePart(f)}>{Math.round(f * 100)}%</button>
            ))}
            <button className="px-2 h-7 rounded bg-down/90 text-white disabled:opacity-40 ml-auto" disabled={busy} onClick={() => void run(() => api.closePosition(account.id, p.id)).then(onClose)} data-testid="card-close">{t('tb.close')}</button>
            <button className={btn} disabled={busy} onClick={() => void reverse()} title={t('tb.reverseHint')} data-testid="card-reverse">{t('tb.reverse')}</button>
          </div>
        </div>

        <div className="border-t border-line pt-2 flex flex-wrap gap-1.5">
          <button className={btn} onClick={() => { openChart(p.symbol); onClose(); }}>{t('tb.openChart')}</button>
          <button className={btn} onClick={() => { openTicket({ symbol: p.symbol, side: p.side }); onClose(); }}>{t('tb.newOrder')}</button>
          <button className={btn} onClick={() => { openTicket({ symbol: p.symbol, side: p.side === 'buy' ? 'sell' : 'buy' }); onClose(); }}>{t('tb.hedgeOrder')}</button>
        </div>
      </div>
    </Modal>
  );
}

function Positions() {
  const t = useT();
  const positions = useTerminal(selectPositions);
  const [detail, setDetail] = useState<string | null>(null);
  return (
    <>
    {detail && <PositionCard id={detail} onClose={() => setDetail(null)} />}
    <VirtualTable
      testId="positions-table"
      columns={POS_COLS}
      rows={positions}
      rowKey={(p) => p.id}
      renderRow={(p) => <PositionRow p={p} />}
      onRowClick={(p) => setDetail(p.id)}
      empty={t('tb.empty')}
      header={
        <>
          <span>{t('tb.id')}</span><span>{t('tb.time')}</span><span>{t('tb.symbol')}</span><span>{t('tb.type')}</span>
          <span>{t('tb.volume')}</span><span>{t('tb.openPrice')}</span><span>{t('tb.sl')}</span><span>{t('tb.tp')}</span>
          <span>{t('tb.trailing')}</span><span>{t('tb.current')}</span><span>{t('tb.commission')}</span><span className="text-right">{t('tb.profit')}</span><span />
        </>
      }
    />
    </>
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
              <span className="num" data-testid={`deal-pos-${d.positionId}`}>{d.positionId}</span>
              <span className="num text-muted">{formatTime(d.time)}</span>
              <span className="font-medium">{d.symbol}</span>
              <span className={sideCls(d.side)}>{d.side}</span>
              <span data-testid={`deal-entry-${d.entry}`}>{d.entry}</span>
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
  const quotes = useTerminal((s) => s.quotes);
  const symbols = useTerminal((s) => s.symbols);
  const toast = useTerminal((s) => s.toast);
  const rates = useRates();
  // Bulk close is armed by a first click and confirmed by a second within 3 s.
  const [armed, setArmed] = useState<'all' | 'profit' | 'loss' | null>(null);
  useEffect(() => {
    if (!armed) return;
    const id = setTimeout(() => setArmed(null), 3000);
    return () => clearTimeout(id);
  }, [armed]);
  const counts: Partial<Record<ToolboxTab, number>> = { positions: nPos, orders: nOrd };
  const profitOf = (p: Position) => {
    const spec = symbols[p.symbol];
    const q = quotes[p.symbol];
    try {
      return spec && q && account ? positionProfit(p, spec, q, account.currency, rates) : 0;
    } catch {
      return 0;
    }
  };
  const bulk = async (which: 'all' | 'profit' | 'loss') => {
    if (!account) return;
    const targets = bulkTargets(positions, profitOf, which);
    if (!targets.length) return;
    if (armed !== which) return setArmed(which);
    setArmed(null);
    const results = await Promise.all(targets.map((p) => getApi().closePosition(account.id, p.id)));
    const failed = results.filter((r) => !r.ok);
    if (failed.length) toast('error', t('toast.rejected', { error: failed[0]!.error ?? '' }));
  };
  const mode = useTerminal((s) => s.toolboxMode);
  const setMode = useTerminal((s) => s.setToolboxMode);
  const collapsed = mode === 'min';
  return (
    <section className={`flex flex-col ${collapsed ? '' : 'h-full'} bg-panel`} data-toolbox-mode={mode}>
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
        <span className="ml-auto mr-1 flex items-center gap-0.5 order-last">
          <button
            className="w-5 h-5 grid place-items-center rounded text-[11px] text-muted hover:text-fg hover:bg-panel-2"
            title={mode === 'max' ? t('tb.restore') : t('tb.maximize')}
            aria-label={mode === 'max' ? t('tb.restore') : t('tb.maximize')}
            data-testid="tb-max"
            onClick={() => setMode(mode === 'max' ? 'normal' : 'max')}
          >
            {mode === 'max' ? '⤓' : '⤢'}
          </button>
          <button
            className="w-5 h-5 grid place-items-center rounded text-[11px] text-muted hover:text-fg hover:bg-panel-2"
            title={collapsed ? t('tb.expand') : t('tb.collapse')}
            aria-label={collapsed ? t('tb.expand') : t('tb.collapse')}
            data-testid="tb-collapse"
            onClick={() => setMode(collapsed ? 'normal' : 'min')}
          >
            {collapsed ? '▴' : '▾'}
          </button>
        </span>
        {!collapsed && tab === 'positions' && positions.length > 0 && account && (
          <span className="ml-auto mr-2 flex gap-1">
            {(['profit', 'loss', 'all'] as const).map((w) => {
              const n = bulkTargets(positions, profitOf, w).length;
              return (
                <button
                  key={w}
                  disabled={n === 0}
                  data-testid={`close-${w}`}
                  className={`px-2 py-0.5 rounded border disabled:opacity-40 ${armed === w ? 'bg-down text-white border-down' : `border-line ${w === 'profit' ? 'text-up' : w === 'loss' ? 'text-down' : 'text-muted hover:text-fg'}`}`}
                  onClick={() => void bulk(w)}
                >
                  {armed === w ? t('tb.confirmBulk', { n }) : `${t(w === 'all' ? 'tb.closeAll' : w === 'profit' ? 'tb.closeProfit' : 'tb.closeLoss')} ${n}`}
                </button>
              );
            })}
          </span>
        )}
      </div>
      {!collapsed && (
        <>
      <div className="flex-1 min-h-0 overflow-x-auto">
        {tab === 'positions' && <Positions />}
        {tab === 'orders' && <Orders />}
        {tab === 'history' && <History />}
        {tab === 'journal' && <Journal />}
      </div>
        </>
      )}
    </section>
  );
}
