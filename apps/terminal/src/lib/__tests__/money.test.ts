import { describe, expect, it } from 'vitest';
import {
  buildRates,
  commissionMinor,
  computeAccountMetrics,
  formatMoney,
  lotsToVolume,
  marginMinor,
  pipValue,
  pipsBetween,
  priceFromPips,
  profitMinor,
  volumeToLots,
} from '../money';
import { quotes, specs } from '../../test/fixtures';
import type { Account, Position } from '../../api/types';

const rates = buildRates(specs, quotes);
const EURUSD = specs.EURUSD!;

describe('volume conversion', () => {
  it('parses lots into integer centi-lots without float drift', () => {
    expect(lotsToVolume('0.1')).toBe(10);
    expect(lotsToVolume('0.29')).toBe(29); // 0.29*100 = 28.999999999999996 in floats
    expect(lotsToVolume('1,25')).toBe(125);
    expect(lotsToVolume('0.001')).toBeNull();
    expect(lotsToVolume('abc')).toBeNull();
    expect(volumeToLots(7)).toBe('0.07');
  });
});

describe('formatMoney', () => {
  it('formats minor units exactly', () => {
    expect(formatMoney(10_000_000)).toBe('100,000.00');
    expect(formatMoney(-1)).toBe('-0.01');
    expect(formatMoney(123456789)).toBe('1,234,567.89');
    expect(formatMoney(0)).toBe('0.00');
  });
});

describe('profit', () => {
  it('EURUSD 1 lot +10 pips = $100.00', () => {
    expect(profitMinor(EURUSD, 'buy', 100, 1.085, 1.086, 'USD', rates)).toBe(10_000);
    expect(profitMinor(EURUSD, 'sell', 100, 1.085, 1.086, 'USD', rates)).toBe(-10_000);
  });

  it('avoids float error: 0.3 lots * 0.1 + 0.2 style sums are exact', () => {
    // 1.1 - 1.0999 = 0.00009999999999998899 in floats
    expect(profitMinor(EURUSD, 'buy', 30, 1.0999, 1.1, 'USD', rates)).toBe(300);
  });

  it('converts JPY-quoted profit into USD', () => {
    // 1 lot USDJPY +1.00 JPY = 100,000 JPY / 150 = $666.67
    expect(profitMinor(specs.USDJPY!, 'buy', 100, 149, 150, 'USD', rates)).toBe(66_667);
  });

  it('converts GBP-quoted cross profit via GBPUSD', () => {
    // EURGBP 1 lot +0.0010 = 100 GBP * 1.27 = $127.00
    expect(profitMinor(specs.EURGBP!, 'buy', 100, 0.853, 0.854, 'USD', rates)).toBe(12_700);
  });
});

describe('margin', () => {
  it('EURUSD 1 lot @1.085 1:100 = $1,085.00', () => {
    expect(marginMinor(EURUSD, 100, 1.085, 100, 'USD', rates)).toBe(108_500);
  });
  it('USD-based pairs need no price', () => {
    expect(marginMinor(specs.USDJPY!, 100, 150, 100, 'USD', rates)).toBe(100_000);
  });
  it('rounds margin up to the next cent', () => {
    // 0.01 lot EURUSD @1.08503 /100 = 10.8503 -> 10.86
    expect(marginMinor(EURUSD, 1, 1.08503, 100, 'USD', rates)).toBe(1086);
  });
  it('applies symbol margin rate (crypto x5)', () => {
    const btc = specs.BTCUSD!;
    expect(marginMinor(btc, 100, 65000, 100, 'USD', rates)).toBe(325_000);
  });
});

describe('pips', () => {
  it('pip value of 1 lot EURUSD is $10', () => {
    expect(pipValue(EURUSD, 100, 'USD', rates).toFixed(2)).toBe('10.00');
  });
  it('price <-> pips', () => {
    expect(priceFromPips(1.085, 20, -1, EURUSD)).toBe(1.083);
    expect(priceFromPips(150.0, 15.5, 1, specs.USDJPY!)).toBe(150.155);
    expect(pipsBetween(1.0853, 1.085, EURUSD)).toBe(3);
  });
});

describe('account metrics', () => {
  const account: Account = { id: 'a', name: 'A', currency: 'USD', balance: 1_000_000, leverage: 100, isDemo: true };
  it('equity = balance + floating; free margin; margin level', () => {
    const pos: Position = {
      id: '1', accountId: 'a', symbol: 'EURUSD', side: 'buy', volume: 100, openPrice: 1.084,
      openTime: 0, commission: commissionMinor(100), swap: 0,
    };
    const m = computeAccountMetrics(account, [pos], quotes, specs, rates);
    // closes at bid 1.08500: +10 pips = $100 minus $3.50 commission
    expect(m.floating).toBe(10_000 - 350);
    expect(m.equity).toBe(1_009_650);
    expect(m.margin).toBe(108_400);
    expect(m.freeMargin).toBe(1_009_650 - 108_400);
    expect(m.marginLevel).toBe('931.41');
  });
  it('no positions -> no margin level', () => {
    expect(computeAccountMetrics(account, [], quotes, specs, rates).marginLevel).toBeNull();
  });
});
