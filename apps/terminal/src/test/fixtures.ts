import type { Quote, SymbolSpec } from '@fxvps/trading-core';
import { MOCK_SYMBOLS, toSpec } from '@fxvps/trading-core';

export const specs: Record<string, SymbolSpec> = Object.fromEntries(MOCK_SYMBOLS.map((s) => [s.name, toSpec(s)]));

export function q(symbol: string, bid: number, ask: number): Quote {
  return { symbol, bid, ask, time: 1_700_000_000_000, dayOpen: bid, dayHigh: ask, dayLow: bid };
}

export const quotes: Record<string, Quote> = {
  EURUSD: q('EURUSD', 1.085, 1.08506),
  GBPUSD: q('GBPUSD', 1.27, 1.27),
  USDJPY: q('USDJPY', 150, 150),
  XAUUSD: q('XAUUSD', 2350, 2350.25),
  EURGBP: q('EURGBP', 0.854, 0.8541),
};
