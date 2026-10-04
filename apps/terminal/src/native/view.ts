import { TIMEFRAMES, type Timeframe } from '@fxvps/trading-core';

export interface ChartView {
  symbol: string;
  timeframe: Timeframe;
}

const SYMBOL_RE = /^[A-Z0-9._-]{1,32}$/;

/** Parses `?view=chart&symbol=EURUSD&tf=M5` (used by detached desktop chart windows). */
export function parseChartView(search: string): ChartView | null {
  const p = new URLSearchParams(search);
  if (p.get('view') !== 'chart') return null;
  const symbol = (p.get('symbol') ?? '').trim().toUpperCase();
  if (!SYMBOL_RE.test(symbol)) return null;
  const tf = (p.get('tf') ?? '').toUpperCase() as Timeframe;
  return { symbol, timeframe: (TIMEFRAMES as readonly string[]).includes(tf) ? tf : 'M5' };
}
