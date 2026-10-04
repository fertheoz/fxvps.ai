import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { decimalToBig, decimalToString, toDecimal } from './decimal';
import { DecimalSchema } from './gen/fxvps_client_v1_pb';

const d = (value: bigint, scale: number) => create(DecimalSchema, { value, scale });

describe('Decimal mapping', () => {
  it('formats wire decimals exactly', () => {
    expect(decimalToString(d(108_501_000n, 8))).toBe('1.08501');
    expect(decimalToString(d(-50n, 2))).toBe('-0.5');
    expect(decimalToString(d(5n, 8))).toBe('0.00000005');
    expect(decimalToString(d(10_000_000_000_000n, 8))).toBe('100000');
    expect(decimalToString(d(0n, 8))).toBe('0');
    expect(decimalToString(d(42n, 0))).toBe('42');
    expect(decimalToString(undefined)).toBeUndefined();
    // Beyond 2^53: still exact.
    expect(decimalToString(d(9_007_199_254_740_993n, 2))).toBe('90071992547409.93');
  });

  it('round-trips strings and numbers without float error', () => {
    expect(toDecimal('1.08501')).toMatchObject({ value: 108501n, scale: 5 });
    expect(toDecimal('0.3')).toMatchObject({ value: 3n, scale: 1 });
    expect(toDecimal(25000)).toMatchObject({ value: 25000n, scale: 0 });
    expect(toDecimal('-0.0001')).toMatchObject({ value: -1n, scale: 4 });
    for (const s of ['1.08501', '149.523', '-12.5', '0.00000001', '100000']) {
      expect(decimalToString(toDecimal(s))).toBe(s);
    }
    expect(decimalToBig(d(108_501_000n, 8))!.eq('1.08501')).toBe(true);
  });
});
