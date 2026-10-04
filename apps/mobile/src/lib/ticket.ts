import {
  buildRates,
  lotsToVolume,
  marginMinor,
  openPrice,
  validateTicket,
  type Account,
  type OrderRequest,
  type Quote,
  type Side,
  type SymbolSpec,
  type ValidationError,
} from '@fxvps/trading-core';

export type MobileOrderType = 'market' | 'limit' | 'stop';

export interface TicketForm {
  side: Side;
  type: MobileOrderType;
  lots: string;
  price: string;
  sl: string;
  tp: string;
}

export function parseOptional(s: string): number | undefined {
  const t = s.trim().replace(',', '.');
  if (t === '') return undefined;
  const n = Number(t);
  return Number.isFinite(n) ? n : undefined;
}

export interface TicketEvaluation {
  errors: ValidationError[];
  requiredMargin?: number;
  request?: OrderRequest;
}

/** Validates the mobile ticket with the shared trading-core rules and builds the request. */
export function evaluateTicket(
  form: TicketForm,
  ctx: {
    account: Account;
    spec: SymbolSpec;
    quote: Quote | undefined;
    freeMargin: number;
    specs: Record<string, SymbolSpec>;
    quotes: Record<string, Quote>;
    now: number;
  },
): TicketEvaluation {
  const volume = lotsToVolume(form.lots);
  const price = form.type === 'market' ? undefined : parseOptional(form.price);
  const sl = parseOptional(form.sl);
  const tp = parseOptional(form.tp);
  let requiredMargin: number | undefined;
  if (volume !== null && ctx.quote) {
    const at = price ?? openPrice(form.side, ctx.quote);
    requiredMargin = marginMinor(
      ctx.spec,
      volume,
      at,
      ctx.account.leverage,
      ctx.account.currency,
      buildRates(ctx.specs, ctx.quotes),
    );
  }
  const errors = validateTicket(
    { side: form.side, type: form.type, volume, price, sl, tp },
    {
      spec: ctx.spec,
      quote: ctx.quote,
      now: ctx.now,
      requiredMargin,
      freeMargin: ctx.freeMargin,
    },
  );
  if (errors.length > 0 || volume === null) return { errors, requiredMargin };
  return {
    errors,
    requiredMargin,
    request: {
      accountId: ctx.account.id,
      symbol: ctx.spec.name,
      side: form.side,
      type: form.type,
      volume,
      price,
      sl,
      tp,
    },
  };
}
