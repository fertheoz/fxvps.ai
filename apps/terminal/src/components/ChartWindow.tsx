import { useEffect, useState } from 'react';
import type { TradingApi } from '../api/types';
import { useT } from '../hooks';
import type { ChartView } from '../native';
import { bootstrap } from '../store/api';
import { useTerminal } from '../store/terminal';
import { ChartPanel } from './ChartPanel';
import { Toasts } from './Dialogs';

const started = new WeakMap<TradingApi, Promise<() => void>>();

/** `?view=chart&symbol=&tf=`: a single chart, used by detached desktop windows. */
export function ChartWindow({ api, view }: { api: TradingApi; view: ChartView }) {
  const t = useT();
  const theme = useTerminal((s) => s.theme);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  useEffect(() => {
    document.title = `${view.symbol} · ${view.timeframe} — fxvps`;
  }, [view]);
  useEffect(() => {
    let cancelled = false;
    let p = started.get(api);
    if (!p) {
      p = bootstrap(api);
      started.set(api, p);
    }
    p.then(() => {
      if (cancelled) return;
      // bootstrap may remap unknown symbols; restore the requested one if it exists
      const s = useTerminal.getState();
      if (s.symbols[view.symbol]) s.setChartSymbol(view.symbol, 0);
      setReady(true);
    }).catch((e: Error) => !cancelled && setError(e.message));
    return () => {
      cancelled = true;
    };
  }, [api, view]);

  if (error) return <div className="p-6 text-down" data-testid="connect-error">{t('gw.failed', { error })}</div>;
  if (!ready) return <div className="h-full grid place-items-center text-muted">{t('conn.connecting')}…</div>;
  return (
    <div className="h-full" data-testid="chart-window">
      <ChartPanel index={0} detached />
      <Toasts />
    </div>
  );
}
