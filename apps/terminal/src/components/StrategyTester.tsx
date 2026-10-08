import { useState } from 'react';
import { backtestMaCross, formatPrice, parseDecimal, TIMEFRAMES, type BtResult, type Timeframe } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { getApi } from '../store/api';
import { useTerminal } from '../store/terminal';
import { Modal } from './Dialogs';

/**
 * Field text -> number; comma or dot decimal ("0,1" / "0.1") like the order ticket. Blank -> `blank`;
 * null when the text is not a number passing `ok`, so the run never uses a value other than the one shown.
 */
const readField = (v: string, ok: (n: number) => boolean, blank?: number): number | null => {
  const n = v.trim() ? parseDecimal(v) : blank;
  return n !== undefined && ok(n) ? n : null;
};
const period = (min: number) => (n: number) => Number.isInteger(n) && n >= min;

/** Strategy tester: moving-average crossover on the symbol's history, run in the browser. */
export function StrategyTester({ symbol, timeframe, onClose }: { symbol: string; timeframe: Timeframe; onClose: () => void }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[symbol]);
  const q = useTerminal((s) => s.quotes[symbol]);
  const [tf, setTf] = useState<Timeframe>(timeframe);
  // numeric fields keep the typed text ("0." while typing 0.1) and are parsed on Run
  const [fastText, setFast] = useState('10');
  const [slowText, setSlow] = useState('30');
  const [kind, setKind] = useState<'sma' | 'ema'>('ema');
  const [lotsText, setLots] = useState('1');
  const [stopText, setStopPts] = useState('0');
  const [takeText, setTakePts] = useState('0');
  const [res, setRes] = useState<{ r: BtResult; bars: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  if (!spec) return null;
  const point = 1 / 10 ** spec.digits;
  const run = async () => {
    const fast = readField(fastText, period(1));
    const slow = readField(slowText, period(2));
    const lots = readField(lotsText, (n) => n > 0);
    const stopPts = readField(stopText, (n) => n >= 0, 0); // blank = no stop
    const takePts = readField(takeText, (n) => n >= 0, 0);
    if (fast === null || slow === null || lots === null || stopPts === null || takePts === null) {
      // name the first bad field instead of silently running with a substitute
      const field = fast === null ? t('bt.fast') : slow === null ? t('bt.slow') : lots === null ? t('bt.lots') : stopPts === null ? 'SL' : 'TP';
      setErr(t('bt.badField', { field }));
      return;
    }
    if (fast >= slow) {
      setErr(t('bt.fastSlow'));
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      const bars = await getApi().getBars(symbol, tf, 2000);
      const r = backtestMaCross(bars, {
        fast,
        slow,
        kind,
        units: lots * spec.contractSize,
        costPrice: q ? q.ask - q.bid : 0,
        stopPrice: stopPts * point,
        takePrice: takePts * point,
      });
      setRes({ r, bars: bars.length });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const inp = 'bg-panel-2 border border-line rounded px-2 h-7 text-[12px] w-20 num';
  const money = (v: number) => `${v.toFixed(2)} ${spec.quote}`;
  const curve = (eq: number[]) => {
    if (eq.length < 2) return null;
    const lo = Math.min(0, ...eq);
    const hi = Math.max(0, ...eq);
    const span = hi - lo || 1;
    const pts = eq.map((v, i) => `${(i / (eq.length - 1)) * 400},${80 - ((v - lo) / span) * 80}`).join(' ');
    return (
      <svg viewBox="0 0 400 80" className="w-full h-20 bg-panel-2 rounded" preserveAspectRatio="none" data-testid="bt-curve">
        <polyline points={pts} fill="none" stroke="var(--accent)" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
      </svg>
    );
  };
  return (
    <Modal title={`${t('bt.title')} · ${symbol}`} onClose={onClose} width="w-[560px]">
      <div className="p-3 grid gap-3 text-[12px]" data-testid="strategy-tester">
        <div className="text-muted">{t('bt.hint')}</div>
        <div className="flex flex-wrap items-end gap-3">
          <label className="grid gap-1">
            {t('bt.timeframe')}
            <select className={inp} value={tf} onChange={(e) => setTf(e.target.value as Timeframe)}>
              {TIMEFRAMES.map((x) => (
                <option key={x} value={x}>
                  {x}
                </option>
              ))}
            </select>
          </label>
          <label className="grid gap-1">
            {t('bt.kind')}
            <select className={inp} value={kind} onChange={(e) => setKind(e.target.value as 'sma' | 'ema')}>
              <option value="ema">EMA</option>
              <option value="sma">SMA</option>
            </select>
          </label>
          <label className="grid gap-1">{t('bt.fast')}<input className={inp} inputMode="numeric" value={fastText} onChange={(e) => setFast(e.target.value)} data-testid="bt-fast" /></label>
          <label className="grid gap-1">{t('bt.slow')}<input className={inp} inputMode="numeric" value={slowText} onChange={(e) => setSlow(e.target.value)} /></label>
          <label className="grid gap-1">{t('bt.lots')}<input className={inp} inputMode="decimal" value={lotsText} onChange={(e) => setLots(e.target.value)} data-testid="bt-lots" /></label>
          <label className="grid gap-1">SL ({t('bt.points')})<input className={inp} inputMode="decimal" value={stopText} onChange={(e) => setStopPts(e.target.value)} data-testid="bt-sl" /></label>
          <label className="grid gap-1">TP ({t('bt.points')})<input className={inp} inputMode="decimal" value={takeText} onChange={(e) => setTakePts(e.target.value)} /></label>
          <button className="px-3 h-7 rounded bg-accent text-white disabled:opacity-40" disabled={busy} onClick={() => void run()} data-testid="bt-run">
            {t('bt.run')}
          </button>
        </div>
        {err && <div className="text-down" data-testid="bt-err">{err}</div>}
        {res && (
          <div className="grid gap-2" data-testid="bt-result">
            <div className="flex flex-wrap gap-x-4 gap-y-1 num">
              <span>{t('bt.bars')}: {res.bars}</span>
              <span>{t('bt.trades')}: {res.r.trades.length}</span>
              <span>
                {t('bt.net')}: <b className={res.r.net < 0 ? 'text-down' : 'text-up'}>{money(res.r.net)}</b>
              </span>
              <span>{t('bt.winRate')}: {res.r.trades.length ? Math.round((res.r.wins / res.r.trades.length) * 100) : 0}%</span>
              <span>PF: {res.r.profitFactor == null ? '—' : res.r.profitFactor.toFixed(2)}</span>
              <span>{t('bt.maxDd')}: {money(res.r.maxDrawdown)}</span>
            </div>
            {curve(res.r.equity)}
            <div className="max-h-48 overflow-auto">
              <table className="w-full num">
                <tbody>
                  {res.r.trades
                    .slice(-100)
                    .reverse()
                    .map((x, i) => (
                      <tr key={i} className="border-t border-line/50">
                        <td className="py-0.5">{new Date(x.entryTime * 1000).toISOString().slice(0, 16).replace('T', ' ')}</td>
                        <td className={x.side === 'buy' ? 'text-up' : 'text-down'}>{x.side}</td>
                        <td>
                          {formatPrice(x.entry, spec.digits)} → {formatPrice(x.exit, spec.digits)}
                        </td>
                        <td className="text-muted">{x.reason}</td>
                        <td className={`text-right ${x.pnl < 0 ? 'text-down' : 'text-up'}`}>{money(x.pnl)}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
            <div className="text-muted text-[11px]">{t('bt.disclaimer')}</div>
          </div>
        )}
      </div>
    </Modal>
  );
}
