import Big from 'big.js';
import { create } from '@bufbuild/protobuf';
import { DecimalSchema, type Decimal } from './gen/fxvps_client_v1_pb';

/**
 * Exact conversions between the wire `Decimal{value int64, scale uint32}`
 * (real value = value * 10^-scale) and decimal strings / big.js. No floats.
 */

/** Wire Decimal -> canonical decimal string ("1.08501", "-0.5", "100000"). */
export function decimalToString(d: Decimal | undefined): string | undefined {
  if (!d) return undefined;
  const neg = d.value < 0n;
  const digits = (neg ? -d.value : d.value).toString();
  const scale = d.scale;
  let s: string;
  if (scale === 0) s = digits;
  else {
    const padded = digits.padStart(scale + 1, '0');
    const int = padded.slice(0, padded.length - scale);
    const frac = padded.slice(padded.length - scale).replace(/0+$/, '');
    s = frac ? `${int}.${frac}` : int;
  }
  return neg && s !== '0' ? `-${s}` : s;
}

export function decimalToBig(d: Decimal | undefined): Big | undefined {
  const s = decimalToString(d);
  return s === undefined ? undefined : new Big(s);
}

/** Decimal string / Big / number (formatted exactly via big.js) -> wire Decimal. */
export function toDecimal(v: string | number | Big): Decimal {
  const s = new Big(v).toFixed();
  const neg = s.startsWith('-');
  const [int = '0', frac = ''] = (neg ? s.slice(1) : s).split('.');
  const value = BigInt(int + frac) * (neg ? -1n : 1n);
  return create(DecimalSchema, { value, scale: frac.length });
}
