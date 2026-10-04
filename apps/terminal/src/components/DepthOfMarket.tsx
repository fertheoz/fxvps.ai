import { useEffect, useMemo, useState } from 'react';
import type { Depth, Side } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { getApi } from '../store/api';
import { useTerminal } from '../store/terminal';
import { RafBatcher } from '@fxvps/trading-core';
import { vwapFill } from '@fxvps/trading-core';
import { formatPrice, lotsToVolume, pipsBetween, volumeToLots } from '@fxvps/trading-core';

export function DepthOfMarket() {
  const t = useT();
  const symbol = useTerminal((s) => s.charts[s.activeChart]?.symbol);
  const spec = useTerminal((s) => (symbol ? s.symbols[symbol] : undefined));
  const [depth, setDepth] = useState<Depth | null>(null);
  const [mode, setMode] = useState<'standard' | 'vwap'>('standard');
  const [lots, setLots] = useState('5.00');

  useEffect(() => {
    if (!symbol) return;
    const batcher = new RafBatcher<Depth>((d) => setDepth(d[d.length - 1] ?? null));
    return getApi().subscribeDepth(symbol, (d) => batcher.push('d', d));
  }, [symbol]);

  const d = depth && depth.symbol === symbol ? depth : null;
  const maxVol = useMemo(() => (d ? Math.max(1, ...d.bids.map((x) => x.volume), ...d.asks.map((x) => x.volume)) : 1), [d]);
  const volume = lotsToVolume(lots);

  if (!spec) return null;

  const fill = (side: Side) => (d && volume ? vwapFill(side, volume, d.bids, d.asks, spec.digits) : null);

  return (
    <section className="flex flex-col h-full bg-panel" aria-label={t('dom.title')} data-testid="dom">
      <div className="flex items-center justify-between px-2 h-8 border-b border-line shrink-0">
        <h2 className="font-semibold">
          {t('dom.title')} <span className="text-muted font-normal">{symbol}</span>
        </h2>
        <div className="flex text-[11px] rounded border border-line overflow-hidden">
          {(['standard', 'vwap'] as const).map((m) => (
            <button key={m} className={`px-2 ${mode === m ? 'bg-accent text-white' : 'text-muted'}`} onClick={() => setMode(m)}>
              {t(`dom.${m}`)}
            </button>
          ))}
        </div>
      </div>
      {mode === 'vwap' && (
        <div className="p-2 border-b border-line">
          <label className="flex items-center gap-2">
            <span className="text-muted">{t('dom.fillFor')}</span>
            <input
              value={lots}
              onChange={(e) => setLots(e.target.value)}
              className="num w-20 bg-panel-2 border border-line rounded px-2 py-0.5"
              aria-label={t('dom.fillFor')}
              inputMode="decimal"
            />
            <span className="text-muted">{t('chart.lots')}</span>
          </label>
          <div className="grid grid-cols-2 gap-2 mt-2">
            {(['sell', 'buy'] as Side[]).map((side) => {
              const f = fill(side);
              const top = d ? (side === 'buy' ? d.asks[0]?.price : d.bids[0]?.price) : undefined;
              return (
                <div key={side} className={`rounded border p-1.5 ${side === 'buy' ? 'border-up/40' : 'border-down/40'}`}>
                  <div className={`text-[10px] uppercase ${side === 'buy' ? 'text-up' : 'text-down'}`}>{side === 'buy' ? t('ticket.buy') : t('ticket.sell')}</div>
                  <div className="flex justify-between"><span className="text-muted">{t('dom.avg')}</span><span className="num" data-testid={`vwap-${side}`}>{f?.avgPrice != null ? formatPrice(f.avgPrice, spec.digits) : '—'}</span></div>
                  <div className="flex justify-between"><span className="text-muted">{t('dom.worst')}</span><span className="num">{f?.worstPrice != null ? formatPrice(f.worstPrice, spec.digits) : '—'}</span></div>
                  <div className="flex justify-between"><span className="text-muted">{t('dom.slippage')}</span><span className="num">{f?.avgPrice != null && top !== undefined ? `${pipsBetween(f.avgPrice, top, spec)} pip` : '—'}</span></div>
                  {f && volume && f.filled < volume && <div className="text-down text-[10px]">{t('dom.partial')} ({volumeToLots(f.filled)})</div>}
                </div>
              );
            })}
          </div>
        </div>
      )}
      <div className="grid grid-cols-[1fr_1fr] px-2 h-6 items-center text-[10px] uppercase text-muted border-b border-line">
        <span>{t('dom.price')}</span>
        <span className="text-right">{t('dom.size')}</span>
      </div>
      <div className="flex-1 overflow-auto num">
        {d &&
          [...d.asks].reverse().map((l) => (
            <div key={`a${l.price}`} className="relative grid grid-cols-2 px-2 h-5 items-center">
              <div className="absolute inset-y-0.5 right-0 bg-down-bg" style={{ width: `${(l.volume / maxVol) * 100}%` }} />
              <span className="relative text-down">{formatPrice(l.price, spec.digits)}</span>
              <span className="relative text-right">{volumeToLots(l.volume)}</span>
            </div>
          ))}
        {d && d.asks[0] && d.bids[0] && (
          <div className="px-2 h-5 flex items-center justify-center text-[10px] text-muted border-y border-line">
            spread {pipsBetween(d.asks[0].price, d.bids[0].price, spec)} pip
          </div>
        )}
        {d &&
          d.bids.map((l) => (
            <div key={`b${l.price}`} className="relative grid grid-cols-2 px-2 h-5 items-center">
              <div className="absolute inset-y-0.5 right-0 bg-up-bg" style={{ width: `${(l.volume / maxVol) * 100}%` }} />
              <span className="relative text-up">{formatPrice(l.price, spec.digits)}</span>
              <span className="relative text-right">{volumeToLots(l.volume)}</span>
            </div>
          ))}
      </div>
    </section>
  );
}
