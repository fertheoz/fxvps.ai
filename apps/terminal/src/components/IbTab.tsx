import { useEffect, useState } from 'react';
import { formatMoney } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { clientApi, type ClientAccount, type IbDashboard, type IbPeriod } from '../api/clientApi';

/** IB dashboard: referral link, terms, this / last month, payouts. */
export function IbTab({ acc }: { acc: ClientAccount }) {
  const t = useT();
  const toast = useTerminal((s) => s.toast);
  const [ibs, setIbs] = useState<IbDashboard[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const id = setTimeout(() => {
      clientApi.ib().then(
        (r) => setIbs(r.ibs),
        (e: unknown) => setError((e as Error).message),
      );
    }, 0);
    return () => clearTimeout(id);
  }, []);
  const ib = ibs?.find((x) => x.ib === acc.login) ?? ibs?.[0];
  if (error) return <div className="text-down">{error}</div>;
  if (!ibs) return <div className="text-muted">…</div>;
  if (!ib) return <div className="text-muted" data-testid="ib-none">{t('ib.none')}</div>;
  const link = ib.code ? `${window.location.origin}/?ref=${ib.code}` : null;
  const period = (label: string, p: IbPeriod | null) => (
    <div className="border border-line rounded p-2 grid gap-1">
      <div className="text-[10px] uppercase text-muted">{label}</div>
      <div className="num">{t('ib.lots')}: {p ? p.lots.toFixed(2) : '0.00'}</div>
      <div className="num">{t('ib.earned')}: <span className="text-up">{formatMoney(p?.payout ?? 0)} {acc.currency}</span></div>
    </div>
  );
  return (
    <div className="grid gap-3" data-testid="ib-tab">
      {link && (
        <div className="grid gap-1">
          <div className="text-[10px] uppercase text-muted">{t('ib.link')}</div>
          <div className="flex gap-2 items-center">
            <code className="select-all break-all bg-panel-2 border border-line rounded px-2 py-1 flex-1">{link}</code>
            <button
              className="px-2 h-7 rounded border border-line hover:bg-hover text-[12px]"
              onClick={() => void navigator.clipboard?.writeText(link).then(() => toast('ok', t('ib.copied')))}
            >
              {t('ib.copy')}
            </button>
          </div>
        </div>
      )}
      <div className="text-muted">
        {t('ib.terms')}: {ib.sharePct}% · {formatMoney(ib.perLotCents)} {acc.currency}/lot{ib.overridePct ? ` · override ${ib.overridePct}%` : ''} · {t('ib.clients')}: {ib.clients}
      </div>
      <div className="grid grid-cols-2 gap-2">
        {period(t('ib.thisMonth'), ib.thisMonth)}
        {period(t('ib.lastMonth'), ib.lastMonth)}
      </div>
      <div>
        <div className="text-[10px] uppercase text-muted mb-1">{t('ib.payouts')}</div>
        {ib.payouts.length === 0 && <div className="text-muted">{t('tb.empty')}</div>}
        {ib.payouts.map((p, i) => (
          <div key={i} className="flex justify-between border-t border-line/60 py-1">
            <span>{p.reason}</span>
            <span className="num">{formatMoney(p.amount)} {p.currency} · {p.status}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
