import type { OrderType, Quote, Side, SymbolSpec } from '../api/types';
import { big } from './money';

export type ValidationKey =
  | 'volume.required'
  | 'volume.min'
  | 'volume.max'
  | 'volume.step'
  | 'price.required'
  | 'price.side'
  | 'limitPrice.required'
  | 'limitPrice.side'
  | 'sl.side'
  | 'tp.side'
  | 'expiry.past'
  | 'margin.insufficient'
  | 'quote.missing';

export interface ValidationError {
  field: 'volume' | 'price' | 'limitPrice' | 'sl' | 'tp' | 'expiry' | 'form';
  key: ValidationKey;
}

export interface TicketInput {
  side: Side;
  type: OrderType;
  /** Centi-lots, null when the text field does not parse. */
  volume: number | null;
  price?: number;
  limitPrice?: number;
  sl?: number;
  tp?: number;
  expiry?: number;
}

export interface ValidationContext {
  spec: SymbolSpec;
  quote: Pick<Quote, 'bid' | 'ask'> | undefined;
  now: number;
  /** Required margin (minor) for this order and current free margin (minor). */
  requiredMargin?: number;
  freeMargin?: number;
}

/** The price at which the order would enter (used for SL/TP sanity). */
export function entryPrice(input: TicketInput, quote: Pick<Quote, 'bid' | 'ask'>): number | undefined {
  switch (input.type) {
    case 'market':
      return input.side === 'buy' ? quote.ask : quote.bid;
    case 'limit':
    case 'stop':
      return input.price;
    case 'stop_limit':
      return input.limitPrice;
  }
}

export function validateTicket(input: TicketInput, ctx: ValidationContext): ValidationError[] {
  const errors: ValidationError[] = [];
  const { spec, quote } = ctx;
  const v = input.volume;
  if (v === null || v <= 0) errors.push({ field: 'volume', key: 'volume.required' });
  else {
    if (v < spec.minVolume) errors.push({ field: 'volume', key: 'volume.min' });
    if (v > spec.maxVolume) errors.push({ field: 'volume', key: 'volume.max' });
    if ((v - spec.minVolume) % spec.volumeStep !== 0) errors.push({ field: 'volume', key: 'volume.step' });
  }
  if (!quote) {
    errors.push({ field: 'form', key: 'quote.missing' });
    return errors;
  }
  const buy = input.side === 'buy';
  const ask = big(quote.ask);
  const bid = big(quote.bid);

  if (input.type !== 'market') {
    if (input.price === undefined || !(input.price > 0)) errors.push({ field: 'price', key: 'price.required' });
    else {
      const p = big(input.price);
      // buy limit below ask, buy stop above ask, sell limit above bid, sell stop below bid
      const ok =
        input.type === 'limit' ? (buy ? p.lt(ask) : p.gt(bid)) : buy ? p.gt(ask) : p.lt(bid);
      if (!ok) errors.push({ field: 'price', key: 'price.side' });
    }
    if (input.type === 'stop_limit') {
      if (input.limitPrice === undefined || !(input.limitPrice > 0))
        errors.push({ field: 'limitPrice', key: 'limitPrice.required' });
      else if (input.price !== undefined) {
        // MT5 semantics: buy stop-limit places a buy limit at or below the stop; sell at or above.
        const lp = big(input.limitPrice);
        const sp = big(input.price);
        if (buy ? lp.gt(sp) : lp.lt(sp)) errors.push({ field: 'limitPrice', key: 'limitPrice.side' });
      }
    }
  }

  const entry = entryPrice(input, quote);
  if (entry !== undefined) {
    const e = big(entry);
    if (input.sl !== undefined && (buy ? big(input.sl).gte(e) : big(input.sl).lte(e)))
      errors.push({ field: 'sl', key: 'sl.side' });
    if (input.tp !== undefined && (buy ? big(input.tp).lte(e) : big(input.tp).gte(e)))
      errors.push({ field: 'tp', key: 'tp.side' });
  }

  if (input.expiry !== undefined && input.type !== 'market' && input.expiry <= ctx.now)
    errors.push({ field: 'expiry', key: 'expiry.past' });

  if (
    input.type === 'market' &&
    ctx.requiredMargin !== undefined &&
    ctx.freeMargin !== undefined &&
    ctx.requiredMargin > ctx.freeMargin
  )
    errors.push({ field: 'form', key: 'margin.insufficient' });

  return errors;
}
