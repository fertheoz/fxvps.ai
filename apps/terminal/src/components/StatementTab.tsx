import { useState } from 'react';
import { formatMoney } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { clientApi, type ClientAccount, type Statement } from '../api/clientApi';

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const day = (iso: string) => iso.slice(0, 10);

/** Printable statement page; the browser's print dialog saves it as PDF. */
type LabelKey = 'title' | 'generated' | 'summary' | 'balance' | 'equity' | 'closed' | 'pnl' | 'commission' | 'swap' | 'cash' | 'date' | 'amount' | 'deposit' | 'withdraw' | 'price' | 'open';

function printStatement(s: Statement, labels: Record<LabelKey, string>) {
  const m = (v: number) => `${formatMoney(v)} ${s.currency}`;
  const row = (cells: (string | number)[]) => `<tr>${cells.map((c) => `<td>${esc(String(c))}</td>`).join('')}</tr>`;
  const html = `<!doctype html><html><head><meta charset="utf-8"><title>${esc(labels.title)} ${s.account}</title>
<style>body{font:12px system-ui,sans-serif;margin:24px;color:#111}h1{font-size:18px;margin:0 0 4px}h2{font-size:13px;margin:18px 0 6px}
table{width:100%;border-collapse:collapse}td,th{border-bottom:1px solid #ddd;padding:3px 6px;text-align:left}th{background:#f4f4f4}.muted{color:#666}</style></head><body>
<h1>${esc(s.broker)} — ${esc(labels.title)}</h1>
<div class="muted">${esc(s.name)} · #${s.account} · ${esc(s.group)} · ${day(s.from)} – ${s.to ? day(s.to) : day(s.generatedAt)} · ${esc(labels.generated)} ${esc(s.generatedAt)}</div>
<h2>${esc(labels.summary)}</h2><table>
${row([labels.balance, m(s.balance)])}${row([labels.equity, m(s.equity)])}
${row([labels.closed, `${s.totals.trades} · ${s.totals.lots.toFixed(2)} lot`])}
${row([labels.pnl, m(s.totals.pnl)])}${row([labels.commission, m(s.totals.commission)])}${row([labels.swap, m(s.totals.swap)])}</table>
<h2>${esc(labels.cash)}</h2><table><tr><th>${esc(labels.date)}</th><th></th><th>${esc(labels.amount)}</th></tr>
${s.cash.map((c) => row([day(c.at), c.kind === 'deposit' ? labels.deposit : labels.withdraw, m(c.kind === 'deposit' ? c.amount : -c.amount)])).join('')}</table>
<h2>${esc(labels.closed)}</h2><table><tr><th>${esc(labels.date)}</th><th>#</th><th></th><th></th><th>Lot</th><th>${esc(labels.price)}</th><th>${esc(labels.pnl)}</th><th>${esc(labels.commission)}</th><th>${esc(labels.swap)}</th></tr>
${s.trades.map((x) => row([x.at.replace('T', ' ').slice(0, 19), x.position, x.symbol, x.side, x.lots, x.price, m(x.pnl), m(x.commission), m(x.swap)])).join('')}</table>
<h2>${esc(labels.open)}</h2><table><tr><th>#</th><th></th><th></th><th>Lot</th><th>${esc(labels.price)}</th><th>${esc(labels.swap)}</th></tr>
${s.positions.map((p) => row([p.id, p.symbol, p.side, p.lots, p.openPrice, m(p.swap)])).join('')}</table>
</body></html>`;
  const w = window.open('', '_blank');
  if (!w) return false;
  w.document.write(html);
  w.document.close();
  w.focus();
  w.print();
  return true;
}

export function StatementTab({ acc }: { acc: ClientAccount }) {
  const t = useT();
  const toast = useTerminal((s) => s.toast);
  const [today] = useState(() => new Date().toISOString().slice(0, 10));
  const [from, setFrom] = useState(() => new Date(Date.now() - 30 * 864e5).toISOString().slice(0, 10));
  const [to, setTo] = useState(today);
  const [s, setS] = useState<Statement | null>(null);
  const [busy, setBusy] = useState(false);
  const load = async () => {
    setBusy(true);
    try {
      setS(await clientApi.statement(acc.externalId, from, to));
    } catch (e) {
      toast('error', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const labels: Record<LabelKey, string> = {
    title: t('stmt.title'), generated: t('stmt.generated'), summary: t('stmt.summary'), balance: t('acct.balance'), equity: t('stmt.equity'),
    closed: t('stmt.closed'), pnl: t('stmt.pnl'), commission: t('stmt.commission'), swap: t('stmt.swap'), cash: t('stmt.cash'),
    date: t('stmt.date'), amount: t('acct.amount'), deposit: t('acct.deposit'), withdraw: t('acct.withdraw'), price: t('stmt.price'), open: t('stmt.open'),
  };
  const inp = 'bg-panel-2 border border-line rounded px-2 h-8 text-[12px]';
  const btn = 'px-3 h-8 rounded border border-line hover:bg-hover disabled:opacity-40 text-[12px]';
  return (
    <div className="grid gap-3" data-testid="statement-tab">
      <div className="flex flex-wrap items-end gap-2">
        <label className="grid gap-1">{t('stmt.from')}<input type="date" className={inp} value={from} max={to} onChange={(e) => setFrom(e.target.value)} /></label>
        <label className="grid gap-1">{t('stmt.to')}<input type="date" className={inp} value={to} max={today} onChange={(e) => setTo(e.target.value)} /></label>
        <button className={btn} disabled={busy} onClick={() => void load()} data-testid="stmt-load">{t('stmt.show')}</button>
        <button className="px-3 h-8 rounded bg-accent text-white disabled:opacity-40 text-[12px]" disabled={!s} onClick={() => { if (s && !printStatement(s, labels)) toast('error', t('stmt.popup')); }} data-testid="stmt-print">{t('stmt.print')}</button>
      </div>
      {s && (
        <div className="grid gap-1 num">
          <div>{t('stmt.closed')}: {s.totals.trades} · {s.totals.lots.toFixed(2)} lot</div>
          <div>{t('stmt.pnl')}: <span className={s.totals.pnl < 0 ? 'text-down' : 'text-up'}>{formatMoney(s.totals.pnl)} {s.currency}</span> · {t('stmt.commission')}: {formatMoney(s.totals.commission)} · {t('stmt.swap')}: {formatMoney(s.totals.swap)}</div>
          <div>{t('stmt.cash')}: {s.cash.length} · {t('stmt.open')}: {s.positions.length}</div>
        </div>
      )}
    </div>
  );
}
