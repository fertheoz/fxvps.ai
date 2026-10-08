import { formatMoney } from '@fxvps/trading-core';
import { cashSigned, type Statement } from '../api/clientApi';

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const day = (iso: string) => iso.slice(0, 10);

export type StatementLabelKey =
  | 'title' | 'generated' | 'summary' | 'balance' | 'equity' | 'closed' | 'pnl' | 'commission' | 'commissionOpen' | 'swap' | 'cash'
  | 'date' | 'amount' | 'deposit' | 'withdraw' | 'copy_fee' | 'copy_fee_income' | 'nbp' | 'price' | 'open';

/** Printable statement page (HTML); the browser's print dialog saves it as PDF. */
export function statementHtml(s: Statement, labels: Record<StatementLabelKey, string>): string {
  const m = (v: number) => `${formatMoney(v)} ${s.currency}`;
  const row = (cells: (string | number)[]) => `<tr>${cells.map((c) => `<td>${esc(String(c))}</td>`).join('')}</tr>`;
  // commission covers both sides of every deal in the period; the opening side is shown on its own too
  const commission = s.totals.commissionOpen
    ? `${m(s.totals.commission)} (${labels.commissionOpen}: ${m(s.totals.commissionOpen)})`
    : m(s.totals.commission);
  return `<!doctype html><html><head><meta charset="utf-8"><title>${esc(labels.title)} ${s.account}</title>
<style>body{font:12px system-ui,sans-serif;margin:24px;color:#111}h1{font-size:18px;margin:0 0 4px}h2{font-size:13px;margin:18px 0 6px}
table{width:100%;border-collapse:collapse}td,th{border-bottom:1px solid #ddd;padding:3px 6px;text-align:left}th{background:#f4f4f4}.muted{color:#666}</style></head><body>
<h1>${esc(s.broker)} — ${esc(labels.title)}</h1>
<div class="muted">${esc(s.name)} · #${s.account} · ${esc(s.group)} · ${day(s.from)} – ${s.to ? day(s.to) : day(s.generatedAt)} · ${esc(labels.generated)} ${esc(s.generatedAt)}</div>
<h2>${esc(labels.summary)}</h2><table>
${row([labels.balance, m(s.balance)])}${row([labels.equity, m(s.equity)])}
${row([labels.closed, `${s.totals.trades} · ${s.totals.lots.toFixed(2)} lot`])}
${row([labels.pnl, m(s.totals.pnl)])}${row([labels.commission, commission])}${row([labels.swap, m(s.totals.swap)])}</table>
<h2>${esc(labels.cash)}</h2><table><tr><th>${esc(labels.date)}</th><th></th><th>${esc(labels.amount)}</th></tr>
${s.cash.map((c) => row([day(c.at), labels[c.kind] ?? c.kind, m(cashSigned(c))])).join('')}</table>
<h2>${esc(labels.closed)}</h2><table><tr><th>${esc(labels.date)}</th><th>#</th><th></th><th></th><th>Lot</th><th>${esc(labels.price)}</th><th>${esc(labels.pnl)}</th><th>${esc(labels.commission)}</th><th>${esc(labels.swap)}</th></tr>
${s.trades.map((x) => row([x.at.replace('T', ' ').slice(0, 19), x.position, x.symbol, x.side, x.lots, x.price, m(x.pnl), m(x.commission), m(x.swap)])).join('')}</table>
<h2>${esc(labels.open)}</h2><table><tr><th>#</th><th></th><th></th><th>Lot</th><th>${esc(labels.price)}</th><th>${esc(labels.swap)}</th></tr>
${s.positions.map((p) => row([p.id, p.symbol, p.side, p.lots, p.openPrice, m(p.swap)])).join('')}</table>
</body></html>`;
}
