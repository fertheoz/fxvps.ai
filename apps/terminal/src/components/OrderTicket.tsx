import { useEffect, useId, useMemo, useState } from 'react';
import type { OrderType, Side } from '@fxvps/trading-core';
import { useMetrics, useRates, useT } from '../hooks';
import { getApi } from '../store/api';
import { selectActiveAccount, useTerminal, type TicketPreset } from '../store/terminal';
import {
  formatMoney,
  formatPrice,
  lotsToVolume,
  marginMinor,
  pipValue,
  priceFromPips,
  pipsToDistance,
  profitMinor,
  toMinor,
  units,
  volumeToLots,
  convert,
} from '@fxvps/trading-core';
import { entryPrice, validateTicket, type TicketInput } from '@fxvps/trading-core';
import { parseDecimal } from '@fxvps/trading-core';
import type { MessageKey } from '../i18n';

const TYPES: OrderType[] = ['market', 'limit', 'stop', 'stop_limit'];

interface Props {
  preset?: TicketPreset | null;
  /** Follow the active chart's symbol (inline ticket). */
  followChart?: boolean;
  onDone?: () => void;
  autoFocus?: boolean;
  /** `sheet`: touch layout for the phone bottom sheet (large controls, card style). */
  variant?: 'panel' | 'sheet';
}

const QUICK_LOTS = ['0.01', '0.10', '0.50', '1.00'];

export function OrderTicket({ preset, followChart, onDone, autoFocus, variant = 'panel' }: Props) {
  const sheet = variant === 'sheet';
  const t = useT();
  const uid = useId();
  const chartSymbol = useTerminal((s) => s.charts[s.activeChart]?.symbol);
  const symbolOrder = useTerminal((s) => s.symbolOrder);
  const [symbol, setSymbol] = useState(preset?.symbol ?? chartSymbol ?? 'EURUSD');
  const [side, setSide] = useState<Side>(preset?.side ?? 'buy');
  const [type, setType] = useState<OrderType>(preset?.type ?? 'market');
  const [lots, setLots] = useState('0.10');
  const [priceText, setPriceText] = useState(preset?.price !== undefined ? String(preset.price) : '');
  const [limitText, setLimitText] = useState('');
  const [protMode, setProtMode] = useState<'price' | 'pips'>('pips');
  const [slText, setSlText] = useState('');
  const [tpText, setTpText] = useState('');
  const [expiryText, setExpiryText] = useState('');
  const [trailText, setTrailText] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [touched, setTouched] = useState(false);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    if (followChart && chartSymbol) setSymbol(chartSymbol);
  }, [followChart, chartSymbol]);

  const spec = useTerminal((s) => s.symbols[symbol]);
  const quote = useTerminal((s) => s.quotes[symbol]);
  const account = useTerminal(selectActiveAccount);
  const metrics = useMetrics();
  const rates = useRates();
  const toast = useTerminal((s) => s.toast);

  const volume = lotsToVolume(lots);
  const price = parseDecimal(priceText);
  const limitPrice = parseDecimal(limitText);
  const ccy = account?.currency ?? 'USD';

  const derived = useMemo(() => {
    if (!spec || !quote) return null;
    const base: TicketInput = { side, type, volume, price, limitPrice };
    const entry = entryPrice(base, quote);
    const toAbs = (txt: string, kind: 'sl' | 'tp'): number | undefined => {
      const v = parseDecimal(txt);
      if (v === undefined) return undefined;
      if (protMode === 'price') return v;
      if (entry === undefined) return undefined;
      const sign: 1 | -1 = (side === 'buy') === (kind === 'tp') ? 1 : -1;
      return priceFromPips(entry, v, sign, spec);
    };
    const sl = toAbs(slText, 'sl');
    const tp = toAbs(tpText, 'tp');
    const expiry = expiryText ? new Date(expiryText).getTime() : undefined;
    const input: TicketInput = { ...base, sl, tp, expiry: type === 'market' ? undefined : expiry };
    let reqMargin: number | undefined;
    let pv: string | undefined;
    let notional: string | undefined;
    let risk: number | undefined;
    let reward: number | undefined;
    try {
      if (volume && entry !== undefined && account) {
        reqMargin = marginMinor(spec, volume, entry, account.leverage, ccy, rates);
        pv = formatMoney(toMinor(pipValue(spec, volume, ccy, rates)));
        notional = formatMoney(toMinor(convert(units(spec, volume).times(entry), spec.quote, ccy, rates)));
        if (sl !== undefined) risk = profitMinor(spec, side, volume, entry, sl, ccy, rates);
        if (tp !== undefined) reward = profitMinor(spec, side, volume, entry, tp, ccy, rates);
      }
    } catch {
      /* missing conversion rate: preview unavailable */
    }
    const errors = validateTicket(input, {
      spec,
      quote,
      now: quote.time,
      requiredMargin: reqMargin,
      freeMargin: metrics?.freeMargin,
    });
    return { input, entry, errors, reqMargin, pv, notional, risk, reward };
  }, [spec, quote, side, type, volume, price, limitPrice, protMode, slText, tpText, expiryText, account, ccy, rates, metrics?.freeMargin]);

  if (!spec) return null;

  const trailPips = parseDecimal(trailText);
  const errFor = (field: string) => (touched ? derived?.errors.find((e) => e.field === field) : undefined);
  const stepLots = (dir: 1 | -1) => {
    const v = volume ?? spec.minVolume;
    const next = Math.min(spec.maxVolume, Math.max(spec.minVolume, v + dir * spec.volumeStep));
    setLots(volumeToLots(next));
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setTouched(true);
    if (!derived || derived.errors.length || !account || volume === null) return;
    setSubmitting(true);
    try {
      const { input } = derived;
      const r = await getApi().placeOrder({
        accountId: account.id,
        symbol,
        side,
        type,
        volume,
        price: type === 'market' ? undefined : input.price,
        limitPrice: type === 'stop_limit' ? input.limitPrice : undefined,
        sl: input.sl,
        tp: input.tp,
        trailing: trailPips !== undefined && trailPips > 0 ? pipsToDistance(trailPips, spec) : undefined,
        expiry: input.expiry,
      });
      if (r.ok) {
        toast(
          'ok',
          r.positionId
            ? t('toast.filled', { side: side.toUpperCase(), lots: volumeToLots(volume), symbol, price: r.price ?? '' })
            : t('toast.placed', { id: r.orderId ?? '' }),
        );
        setTouched(false);
        onDone?.();
      } else toast('error', t('toast.rejected', { error: r.error }));
    } finally {
      setSubmitting(false);
    }
  };

  const field = sheet
    ? 'w-full h-12 bg-panel-2 border border-line/60 rounded-2xl px-4 num outline-none focus:border-accent'
    : 'w-full bg-panel-2 border border-line rounded px-2 py-1 num outline-none focus:border-accent';
  const label = sheet ? 'block mb-1.5 text-[11px] uppercase tracking-wider text-muted' : 'text-[10px] uppercase tracking-wide text-muted';
  const stepBtn = sheet ? 'w-12 h-12 shrink-0 rounded-2xl bg-panel-2 border border-line/60 text-[20px]' : 'px-2 rounded border border-line';
  const chip = (on: boolean) => `flex-1 h-10 rounded-full text-[13px] font-medium ${on ? 'bg-accent text-white' : 'bg-panel-2 text-muted'}`;
  const err = (f: string) => {
    const e = errFor(f);
    return e ? <p className="text-down text-[11px] mt-0.5" role="alert">{t(`err.${e.key}` as MessageKey)}</p> : null;
  };
  const priceLabel = type === 'stop_limit' ? t('ticket.stopPrice') : t('ticket.price');
  const sideName = side === 'buy' ? t('ticket.buy') : t('ticket.sell');

  return (
    <form onSubmit={(e) => void submit(e)} className={sheet ? 'flex flex-col gap-4 px-5 pt-3 pb-5 text-[14px]' : 'flex flex-col gap-2 p-2 text-[12px]'} aria-label={t('ticket.title')} data-testid="order-ticket" noValidate>
      {sheet && (
        <div className="flex gap-2" role="radiogroup" aria-label={t('ticket.type')}>
          {TYPES.map((x) => (
            <button type="button" key={x} role="radio" aria-checked={type === x} className={chip(type === x)} onClick={() => setType(x)}>
              {t(`ticket.${x}`)}
            </button>
          ))}
        </div>
      )}
      <div className={sheet ? 'hidden' : 'flex gap-2'}>
        <label className="flex-1">
          <span className={label}>{t('ticket.symbol')}</span>
          <select className={field} value={symbol} onChange={(e) => setSymbol(e.target.value)} aria-label={t('ticket.symbol')} disabled={followChart}>
            {symbolOrder.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        </label>
        <label className="flex-1">
          <span className={label}>{t('ticket.type')}</span>
          <select className={field} value={type} onChange={(e) => setType(e.target.value as OrderType)} aria-label={t('ticket.type')} data-testid="ticket-type">
            {TYPES.map((x) => (
              <option key={x} value={x}>
                {t(`ticket.${x}`)}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className={`grid grid-cols-2 ${sheet ? 'gap-3' : 'gap-1'}`} role="radiogroup">
        {(['sell', 'buy'] as Side[]).map((s) => (
          <button
            type="button"
            key={s}
            role="radio"
            aria-checked={side === s}
            onClick={() => setSide(s)}
            className={`${sheet ? 'rounded-2xl py-3' : 'rounded py-1.5'} flex flex-col items-center border ${
              side === s ? (s === 'buy' ? 'bg-up text-white border-up' : 'bg-down text-white border-down') : 'border-line text-muted'
            }`}
          >
            <span className="text-[10px] uppercase">{s === 'buy' ? t('ticket.buy') : t('ticket.sell')}</span>
            <span className={`num font-semibold ${sheet ? 'text-[19px]' : 'text-[14px]'}`}>{quote ? formatPrice(s === 'buy' ? quote.ask : quote.bid, spec.digits) : '—'}</span>
          </button>
        ))}
      </div>

      <label>
        <span className={label}>{t('ticket.volume')}</span>
        <div className={`flex ${sheet ? 'gap-2' : 'gap-1'}`}>
          <button type="button" className={stepBtn} onClick={() => stepLots(-1)} aria-label="-">−</button>
          <input
            id={`${uid}-vol`}
            className={`${field} ${sheet ? 'text-center text-[18px] font-semibold' : ''}`}
            value={lots}
            onChange={(e) => setLots(e.target.value)}
            inputMode="decimal"
            aria-label={t('ticket.volume')}
            data-testid="ticket-volume"
            autoFocus={autoFocus}
          />
          <button type="button" className={stepBtn} onClick={() => stepLots(1)} aria-label="+">+</button>
        </div>
        {sheet && (
          <div className="flex gap-2 mt-2">
            {QUICK_LOTS.map((x) => (
              <button type="button" key={x} className={`num ${chip(lots === x)}`} onClick={() => setLots(x)}>
                {x}
              </button>
            ))}
          </div>
        )}
        {err('volume')}
      </label>

      {type !== 'market' && (
        <div className="flex gap-2">
          <label className="flex-1">
            <span className={label}>{priceLabel}</span>
            <input className={field} value={priceText} onChange={(e) => setPriceText(e.target.value)} inputMode="decimal" aria-label={priceLabel} placeholder={quote ? formatPrice(side === 'buy' ? quote.ask : quote.bid, spec.digits) : ''} />
            {err('price')}
          </label>
          {type === 'stop_limit' && (
            <label className="flex-1">
              <span className={label}>{t('ticket.limitPrice')}</span>
              <input className={field} value={limitText} onChange={(e) => setLimitText(e.target.value)} inputMode="decimal" aria-label={t('ticket.limitPrice')} />
              {err('limitPrice')}
            </label>
          )}
        </div>
      )}

      <div>
        <div className="flex items-center justify-between">
          <span className={label}>SL / TP</span>
          <div className={sheet ? 'flex text-[12px] rounded-full bg-panel-2 p-0.5 mb-1.5 [&>button]:px-3 [&>button]:h-7 [&>button]:rounded-full' : 'flex text-[10px] rounded border border-line overflow-hidden'}>
            {(['pips', 'price'] as const).map((m) => (
              <button type="button" key={m} className={`px-1.5 ${protMode === m ? 'bg-accent text-white' : 'text-muted'}`} onClick={() => setProtMode(m)}>
                {m === 'pips' ? t('ticket.inPips') : t('ticket.inPrice')}
              </button>
            ))}
          </div>
        </div>
        <div className={`flex ${sheet ? 'gap-3' : 'gap-2 mt-0.5'}`}>
          <label className="flex-1">
            <span className="sr-only">{t('ticket.sl')}</span>
            <input className={field} placeholder={t('ticket.sl')} value={slText} onChange={(e) => setSlText(e.target.value)} inputMode="decimal" aria-label={t('ticket.sl')} />
            {protMode === 'pips' && derived?.input.sl !== undefined && <span className="num text-muted text-[10px]">= {formatPrice(derived.input.sl, spec.digits)}</span>}
            {err('sl')}
          </label>
          <label className="flex-1">
            <span className="sr-only">{t('ticket.tp')}</span>
            <input className={field} placeholder={t('ticket.tp')} value={tpText} onChange={(e) => setTpText(e.target.value)} inputMode="decimal" aria-label={t('ticket.tp')} />
            {protMode === 'pips' && derived?.input.tp !== undefined && <span className="num text-muted text-[10px]">= {formatPrice(derived.input.tp, spec.digits)}</span>}
            {err('tp')}
          </label>
        </div>
      </div>

      <label>
        <span className={label}>{t('ticket.trailing')}</span>
        <input
          className={field}
          value={trailText}
          onChange={(e) => setTrailText(e.target.value)}
          inputMode="decimal"
          placeholder={t('ticket.trailingOff')}
          aria-label={t('ticket.trailing')}
          data-testid="ticket-trailing"
        />
      </label>

      {type !== 'market' && (
        <label>
          <span className={label}>{t('ticket.expiry')}</span>
          <input type="datetime-local" className={field} value={expiryText} onChange={(e) => setExpiryText(e.target.value)} aria-label={t('ticket.expiry')} />
          <span className="text-muted text-[10px]">{expiryText ? '' : t('ticket.gtc')}</span>
          {err('expiry')}
        </label>
      )}

      <dl className={sheet ? 'grid grid-cols-2 gap-x-2 gap-y-2 p-4 rounded-2xl bg-panel-2 border border-line/60' : 'grid grid-cols-2 gap-x-2 gap-y-0.5 p-2 rounded bg-panel-2 border border-line'} data-testid="ticket-preview">
        <dt className="text-muted">{t('ticket.margin')}</dt>
        <dd className="num text-right" data-testid="ticket-margin">{derived?.reqMargin !== undefined ? formatMoney(derived.reqMargin) : '—'}</dd>
        <dt className="text-muted">{t('ticket.pipValue')}</dt>
        <dd className="num text-right">{derived?.pv ?? '—'}</dd>
        <dt className="text-muted">{t('ticket.notional')}</dt>
        <dd className="num text-right">{derived?.notional ?? '—'}</dd>
        {derived?.risk !== undefined && (
          <>
            <dt className="text-muted">{t('ticket.slRisk')}</dt>
            <dd className="num text-right text-down">{formatMoney(derived.risk)}</dd>
          </>
        )}
        {derived?.reward !== undefined && (
          <>
            <dt className="text-muted">{t('ticket.tpReward')}</dt>
            <dd className="num text-right text-up">{formatMoney(derived.reward)}</dd>
          </>
        )}
      </dl>
      {errFor('form') && <p className="text-down text-[11px]" role="alert">{t(`err.${errFor('form')!.key}` as MessageKey)}</p>}

      <button
        type="submit"
        disabled={submitting}
        data-testid="ticket-submit"
        className={`${sheet ? 'h-14 rounded-2xl text-[16px] active:scale-[0.98] transition-transform' : 'rounded py-2'} font-semibold text-white disabled:opacity-60 ${side === 'buy' ? 'bg-up' : 'bg-down'}`}
      >
        {t('ticket.place', { side: sideName, type: t(`ticket.${type}`) })} {volume ? volumeToLots(volume) : ''} {symbol}
      </button>
    </form>
  );
}
