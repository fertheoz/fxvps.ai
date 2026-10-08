import { useEffect, useState } from 'react';
import { formatMoney } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { clientApi, type ClientAccount, type CopyOverview } from '../api/clientApi';

/** Strategy showcase: follow a provider at a chosen ratio with an equity stop. */
export function CopyTab({ acc }: { acc: ClientAccount }) {
  const t = useT();
  const toast = useTerminal((s) => s.toast);
  const [data, setData] = useState<CopyOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ratio, setRatio] = useState('100');
  const [stop, setStop] = useState('30');
  const [busy, setBusy] = useState(false);
  const load = () =>
    clientApi.copy().then(
      (d) => {
        setData(d);
        setError(null);
      },
      (e: unknown) => setError((e as Error).message),
    );
  useEffect(() => {
    const id = setTimeout(() => void load(), 0);
    return () => clearTimeout(id);
  }, []);
  const run = async (f: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    try {
      await f();
      toast('ok', ok);
      await load();
    } catch (e) {
      toast('error', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const mine = (data?.subscriptions ?? []).filter((s) => s.follower === acc.login && s.active);
  const inp = 'bg-panel-2 border border-line rounded px-2 h-7 text-[12px] w-16 num';
  const btn = 'px-2 h-7 rounded border border-line hover:bg-hover disabled:opacity-40 text-[12px]';
  return (
    <div className="grid gap-3" data-testid="copy-tab">
      <div className="text-muted">{t('copy.hint')}</div>
      {error && <div className="text-down">{error}</div>}
      <div className="flex flex-wrap items-center gap-3">
        <label className="flex items-center gap-1">{t('copy.ratio')} <input className={inp} value={ratio} onChange={(e) => setRatio(e.target.value)} inputMode="decimal" />%</label>
        <label className="flex items-center gap-1">{t('copy.equityStop')} <input className={inp} value={stop} onChange={(e) => setStop(e.target.value)} inputMode="numeric" />%</label>
      </div>
      {data && data.strategies.length === 0 && <div className="text-muted">{t('copy.none')}</div>}
      {data?.strategies.map((s) => {
        const sub = mine.find((m) => m.provider === s.account);
        return (
          <div key={s.account} className="border border-line rounded p-2 grid gap-1" data-testid={`copy-strategy-${s.account}`}>
            <div className="flex items-center gap-2">
              <span className="font-semibold">{s.name}</span>
              <span className="text-muted text-[11px]">{t('copy.fee')} {(s.perfFeeBps / 100).toFixed(0)}%</span>
              <span className="ml-auto">
                {sub ? (
                  <button className={btn} disabled={busy} onClick={() => void run(() => clientApi.copyUnsubscribe({ account: acc.externalId, provider: s.account, close: true }), t('copy.stopped'))}>
                    {t('copy.stop')}
                  </button>
                ) : (
                  <button
                    className="px-2 h-7 rounded bg-accent text-white disabled:opacity-40 text-[12px]"
                    disabled={busy}
                    data-testid={`copy-follow-${s.account}`}
                    onClick={() =>
                      void run(
                        () =>
                          clientApi.copySubscribe({
                            account: acc.externalId,
                            provider: s.account,
                            ratioBps: Math.round(Number(ratio.replace(',', '.')) * 100),
                            equityStopPct: Math.round(Number(stop)),
                          }),
                        t('copy.following'),
                      )
                    }
                  >
                    {t('copy.follow')}
                  </button>
                )}
              </span>
            </div>
            {s.description && <div className="text-muted text-[11px]">{s.description}</div>}
            <div className="flex flex-wrap gap-x-4 text-[11px] text-muted num">
              <span>30d: <span className={s.pnl30d < 0 ? 'text-down' : 'text-up'}>{s.return30dPct == null ? '—' : `${s.return30dPct.toFixed(1)}%`}</span></span>
              <span>{t('copy.deals')}: {s.deals30d}</span>
              <span>{t('copy.winRate')}: {s.winRate == null ? '—' : `${Math.round(s.winRate * 100)}%`}</span>
              <span>{t('copy.followers')}: {s.followers}</span>
              {sub && <span className="text-fg">{t('copy.result')}: {formatMoney(sub.realized)} {acc.currency}</span>}
            </div>
          </div>
        );
      })}
      <div className="text-muted text-[11px]">{t('copy.risk')}</div>
    </div>
  );
}
