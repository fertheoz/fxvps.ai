import { useState } from 'react';
import { formatMoney } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { clientApi, type ClientAccount, type Statement } from '../api/clientApi';
import { statementHtml, type StatementLabelKey } from '../lib/statement';

function printStatement(s: Statement, labels: Record<StatementLabelKey, string>) {
  const w = window.open('', '_blank');
  if (!w) return false;
  w.document.write(statementHtml(s, labels));
  w.document.close();
  w.focus();
  w.print();
  return true;
}

/** Statement of one account; render it keyed by the account so a switch starts empty. */
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
  const labels: Record<StatementLabelKey, string> = {
    title: t('stmt.title'), generated: t('stmt.generated'), summary: t('stmt.summary'), balance: t('acct.balance'), equity: t('stmt.equity'),
    closed: t('stmt.closed'), pnl: t('stmt.pnl'), commission: t('stmt.commission'), commissionOpen: t('stmt.commissionOpen'), swap: t('stmt.swap'),
    cash: t('stmt.cash'), date: t('stmt.date'), amount: t('acct.amount'), deposit: t('acct.deposit'), withdraw: t('acct.withdraw'),
    copy_fee: t('stmt.copyFee'), copy_fee_income: t('stmt.copyFeeIncome'), nbp: t('stmt.nbp'), price: t('stmt.price'), open: t('stmt.open'),
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
        <div className="grid gap-1 num" data-testid="stmt-summary">
          <div>{t('stmt.closed')}: {s.totals.trades} · {s.totals.lots.toFixed(2)} lot</div>
          <div>
            {t('stmt.pnl')}: <span className={s.totals.pnl < 0 ? 'text-down' : 'text-up'}>{formatMoney(s.totals.pnl)} {s.currency}</span> · {t('stmt.commission')}: {formatMoney(s.totals.commission)}
            {s.totals.commissionOpen ? ` (${t('stmt.commissionOpen')}: ${formatMoney(s.totals.commissionOpen)})` : ''} · {t('stmt.swap')}: {formatMoney(s.totals.swap)}
          </div>
          <div>{t('stmt.cash')}: {s.cash.length} · {t('stmt.open')}: {s.positions.length}</div>
        </div>
      )}
    </div>
  );
}
