import Big from 'big.js';
import type { Account, Position, Quote, Side, SymbolSpec } from '../api/types';

/**
 * Decimal-safe money & trading math.
 * All arithmetic is done in big.js; results that represent money are
 * returned as integer minor units (cents).
 */

Big.DP = 20;
Big.RM = Big.roundHalfUp;

export type Rates = Record<string, Big>;

export const MINOR_PER_MAJOR = 100;
export const CENTILOTS_PER_LOT = 100;

export const big = (n: number | string | Big): Big => (n instanceof Big ? n : new Big(n));

/** 0.10 lot -> 10 centi-lots. Returns null when not a valid non-negative number with <=2 decimals. */
export function lotsToVolume(lots: string | number): number | null {
  const s = String(lots).trim().replace(',', '.');
  if (!/^\d+(\.\d{0,2})?$/.test(s)) return null;
  return Number(big(s).times(CENTILOTS_PER_LOT).toFixed(0));
}

export function volumeToLots(volume: number): string {
  return big(volume).div(CENTILOTS_PER_LOT).toFixed(2);
}

export function minorToMajorString(minor: number): string {
  return big(minor).div(MINOR_PER_MAJOR).toFixed(2);
}

const groupFmt = new Intl.NumberFormat('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
/** Formats minor units with thousands separators. Uses the exact decimal string, never float division. */
export function formatMoney(minor: number): string {
  const neg = minor < 0;
  const abs = Math.abs(minor);
  const major = Math.trunc(abs / MINOR_PER_MAJOR); // integer division is exact for integers
  const cents = abs % MINOR_PER_MAJOR;
  const s = `${groupFmt.format(major).slice(0, -3)}.${String(cents).padStart(2, '0')}`;
  return neg ? `-${s}` : s;
}

export function formatPrice(price: number, digits: number): string {
  return big(price).toFixed(digits);
}

export function roundPrice(price: Big | number, digits: number): number {
  return Number(big(price).round(digits, Big.roundHalfUp).toFixed(digits));
}

export function mid(q: Pick<Quote, 'bid' | 'ask'>): Big {
  return big(q.bid).plus(q.ask).div(2);
}

export function spreadPoints(q: Pick<Quote, 'bid' | 'ask'>, digits: number): number {
  return Number(big(q.ask).minus(q.bid).times(big(10).pow(digits)).round(0).toFixed(0));
}

/**
 * Build a currency -> USD rate table from quotes. Any symbol quoted in USD
 * gives its base rate; any symbol with USD base gives its quote rate.
 */
export function buildRates(specs: Record<string, SymbolSpec>, quotes: Record<string, Quote>): Rates {
  const rates: Rates = { USD: big(1) };
  for (const q of Object.values(quotes)) {
    const s = specs[q.symbol];
    if (!s) continue;
    const m = mid(q);
    if (m.eq(0)) continue;
    if (s.quote === 'USD' && !rates[s.base]) rates[s.base] = m;
    else if (s.base === 'USD' && !rates[s.quote]) rates[s.quote] = big(1).div(m);
  }
  return rates;
}

export function convert(amount: Big, from: string, to: string, rates: Rates): Big {
  if (from === to) return amount;
  const rf = rates[from];
  const rt = rates[to];
  if (!rf || !rt) throw new Error(`No conversion rate ${from}->${to}`);
  return amount.times(rf).div(rt);
}

/**
 * Cross rates like 1/150 are non-terminating; big.js keeps 20 dp, so
 * 1000 * (1/150) * 150 = 999.99999999999999999...  Settle to 10 dp of a
 * minor unit before the final rounding to drop such artifacts.
 */
const SETTLE_DP = 10;

/** Major -> integer minor units, rounded half-up. */
export function toMinor(major: Big): number {
  return Number(major.times(MINOR_PER_MAJOR).round(SETTLE_DP, Big.roundHalfUp).round(0, Big.roundHalfUp).toFixed(0));
}

/** Major -> integer minor units, rounded up (away from zero) — conservative for margin. */
export function toMinorCeil(major: Big): number {
  return Number(major.times(MINOR_PER_MAJOR).round(SETTLE_DP, Big.roundHalfUp).round(0, Big.roundUp).toFixed(0));
}

export function units(spec: SymbolSpec, volume: number): Big {
  return big(volume).div(CENTILOTS_PER_LOT).times(spec.contractSize);
}

/** Closing price for a position: buys close on bid, sells close on ask. */
export function closePrice(side: Side, q: Pick<Quote, 'bid' | 'ask'>): number {
  return side === 'buy' ? q.bid : q.ask;
}
export function openPrice(side: Side, q: Pick<Quote, 'bid' | 'ask'>): number {
  return side === 'buy' ? q.ask : q.bid;
}

/** Profit in account currency minor units for `volume` closed at `exitPrice`. */
export function profitMinor(
  spec: SymbolSpec,
  side: Side,
  volume: number,
  entry: number,
  exit: number,
  accountCcy: string,
  rates: Rates,
): number {
  const diff = side === 'buy' ? big(exit).minus(entry) : big(entry).minus(exit);
  const inQuote = diff.times(units(spec, volume));
  return toMinor(convert(inQuote, spec.quote, accountCcy, rates));
}

/** Floating P/L of an open position at the current quote (excl. commission/swap). */
export function positionProfit(
  pos: Position,
  spec: SymbolSpec,
  q: Pick<Quote, 'bid' | 'ask'>,
  accountCcy: string,
  rates: Rates,
): number {
  return profitMinor(spec, pos.side, pos.volume, pos.openPrice, closePrice(pos.side, q), accountCcy, rates);
}

/** Required margin in minor units: units * price / leverage / marginRate, converted to account currency. */
export function marginMinor(
  spec: SymbolSpec,
  volume: number,
  price: number,
  leverage: number,
  accountCcy: string,
  rates: Rates,
): number {
  const notionalQuote = units(spec, volume).times(price);
  const notional = convert(notionalQuote, spec.quote, accountCcy, rates);
  return toMinorCeil(notional.times(spec.marginRate).div(leverage));
}

/** Value of one pip for `volume`, in account currency major units (Big, unrounded). */
export function pipValue(spec: SymbolSpec, volume: number, accountCcy: string, rates: Rates): Big {
  return convert(units(spec, volume).times(spec.pipSize), spec.quote, accountCcy, rates);
}

export function priceFromPips(base: number, pips: number | string, direction: 1 | -1, spec: SymbolSpec): number {
  return roundPrice(big(base).plus(big(pips).times(spec.pipSize).times(direction)), spec.digits);
}

export function pipsBetween(a: number, b: number, spec: SymbolSpec): number {
  return Number(big(a).minus(b).abs().div(spec.pipSize).round(1).toFixed(1));
}

export interface AccountMetrics {
  balance: number;
  equity: number;
  floating: number;
  margin: number;
  freeMargin: number;
  /** Percent with 2 decimals as string, null when no margin used. */
  marginLevel: string | null;
}

export function computeAccountMetrics(
  account: Account,
  positions: Position[],
  quotes: Record<string, Quote>,
  specs: Record<string, SymbolSpec>,
  rates: Rates = buildRates(specs, quotes),
): AccountMetrics {
  let floating = 0;
  let margin = 0;
  for (const p of positions) {
    const spec = specs[p.symbol];
    const q = quotes[p.symbol];
    if (!spec || !q) continue;
    floating += positionProfit(p, spec, q, account.currency, rates) + p.commission + p.swap;
    margin += marginMinor(spec, p.volume, p.openPrice, account.leverage, account.currency, rates);
  }
  const equity = account.balance + floating;
  return {
    balance: account.balance,
    equity,
    floating,
    margin,
    freeMargin: equity - margin,
    marginLevel: margin > 0 ? big(equity).div(margin).times(100).toFixed(2) : null,
  };
}

/** Commission per side: $3.50 per lot per side by default -> minor units, rounded up. */
export function commissionMinor(volume: number, perLotMinor = 350): number {
  return -Math.ceil((volume * perLotMinor) / CENTILOTS_PER_LOT);
}

/** Trailing distance: pips -> price units (exact, rounded to the symbol digits). */
export function pipsToDistance(pips: number | string, spec: SymbolSpec): number {
  return Number(big(pips).times(spec.pipSize).toFixed(spec.digits));
}

/** Trailing distance: price units -> pips (for display). */
export function distanceToPips(distance: number, spec: SymbolSpec): number {
  return Number(big(distance).div(spec.pipSize).round(1).toFixed());
}
