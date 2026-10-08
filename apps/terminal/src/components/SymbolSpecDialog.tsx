import { useEffect, useState } from 'react';
import { Modal } from './Dialogs';
import { clientApi, clientApiBase, type Sentiment } from '../api/clientApi';
import { useT, useRates } from '../hooks';
import { useTerminal, selectActiveAccount } from '../store/terminal';
import { formatMoney, formatPrice, marginMinor, volumeToLots, CENTILOTS_PER_LOT } from '@fxvps/trading-core';

/** Contract specification of a symbol, like MT5's "Specification" window. */
export function SymbolSpecDialog({ symbol, onClose }: { symbol: string; onClose: () => void }) {
  const t = useT();
  const spec = useTerminal((s) => s.symbols[symbol]);
  const q = useTerminal((s) => s.quotes[symbol]);
  const account = useTerminal(selectActiveAccount);
  const rates = useRates();
  const [sent, setSent] = useState<Sentiment | null>(null);
  useEffect(() => {
    if (!clientApiBase()) return;
    let live = true;
    clientApi.sentiment().then(
      (rows) => {
        if (live) setSent(rows.find((r) => r.symbol === symbol) ?? null);
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [symbol]);
  if (!spec) return null;
  const point = 1 / 10 ** spec.digits;
  const spread = q ? Math.round((q.ask - q.bid) / point) : null;
  const marginPerLot = (() => {
    try {
      return account && q ? marginMinor(spec, CENTILOTS_PER_LOT, q.ask, account.leverage, account.currency, rates) : null;
    } catch {
      return null;
    }
  })();
  const rows: [string, string][] = [
    [t('spec.description'), spec.description],
    [t('spec.category'), spec.category],
    [t('spec.base'), spec.base],
    [t('spec.quote'), spec.quote],
    [t('spec.digits'), String(spec.digits)],
    [t('spec.point'), formatPrice(point, spec.digits)],
    [t('spec.pip'), spec.pipSize],
    [t('spec.contract'), `${spec.contractSize.toLocaleString()} ${spec.base}`],
    [t('spec.minVolume'), volumeToLots(spec.minVolume)],
    [t('spec.maxVolume'), volumeToLots(spec.maxVolume)],
    [t('spec.volumeStep'), volumeToLots(spec.volumeStep)],
    [t('spec.marginRate'), `${spec.marginRate}${account ? ` · 1:${account.leverage}` : ''}`],
    [t('spec.marginPerLot'), marginPerLot !== null && account ? `${formatMoney(marginPerLot)} ${account.currency}` : '—'],
    [t('spec.spread'), spread !== null ? `${spread} ${t('spec.points')}` : '—'],
    [t('spec.lastTick'), q ? new Date(q.time).toLocaleTimeString() : '—'],
    [t('spec.swap'), t('spec.fromServer')],
    [t('spec.commission'), t('spec.fromServer')],
  ];
  return (
    <Modal title={`${t('spec.title')} · ${symbol}`} onClose={onClose} width="w-[460px]">
      <div className="px-3 py-2 max-h-[70vh] overflow-auto" data-testid="symbol-spec">
        <table className="w-full text-[12px]">
          <tbody>
            {rows.map(([k, v]) => (
              <tr key={k} className="border-b border-line/50">
                <td className="py-1.5 pr-3 text-muted whitespace-nowrap">{k}</td>
                <td className="py-1.5 num text-right">{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {sent && (
          <div className="mt-2 grid gap-1 text-[12px]" data-testid="sentiment">
            <div className="flex justify-between text-muted">
              <span>{t('spec.sentiment')}</span>
              <span>{sent.traders} {t('spec.traders')}</span>
            </div>
            <div className="flex h-2 rounded overflow-hidden bg-down/60">
              <div className="bg-up" style={{ width: `${sent.longPct}%` }} />
            </div>
            <div className="flex justify-between num">
              <span className="text-up">{t('spec.buy')} {sent.longPct}%</span>
              <span className="text-down">{t('spec.sell')} {100 - sent.longPct}%</span>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
