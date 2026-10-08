import type { Bar } from '../api/types';
import { ema, sma } from './indicators';

export interface MaCrossParams {
  fast: number;
  slow: number;
  kind: 'sma' | 'ema';
  /** Position size in base units (lots × contract size). */
  units: number;
  /** Round-trip cost in price units (spread + commission equivalent). */
  costPrice: number;
  /** Stop loss / take profit distance in price units (0 = none). */
  stopPrice?: number;
  takePrice?: number;
}

export interface BtTrade {
  side: 'buy' | 'sell';
  entryTime: number;
  entry: number;
  exitTime: number;
  exit: number;
  /** Quote-currency result. */
  pnl: number;
  reason: 'signal' | 'stop' | 'take' | 'end';
}

export interface BtResult {
  trades: BtTrade[];
  /** Cumulative closed result after each trade (quote currency). */
  equity: number[];
  net: number;
  wins: number;
  maxDrawdown: number;
  profitFactor: number | null;
}

/**
 * Moving-average crossover, always in the market after the first cross
 * (fast above slow = long, below = short). Signals act on the next bar's
 * open (no look-ahead). Each bar first executes its signal at the open,
 * then checks the stop / target of whatever position is open against the
 * bar's high / low, including a position opened at that open. Pessimistic
 * fills: the stop first when both are inside one bar, a stop the bar gaps
 * through fills at the open, a target always fills at its own price.
 */
export function backtestMaCross(bars: Bar[], p: MaCrossParams): BtResult {
  const closes = bars.map((b) => b.close);
  const f = p.kind === 'ema' ? ema(closes, p.fast) : sma(closes, p.fast);
  const s = p.kind === 'ema' ? ema(closes, p.slow) : sma(closes, p.slow);
  const trades: BtTrade[] = [];
  // assigned inside the close() closure: keep the declared type (no narrowing to null)
  let pos = null as { side: 'buy' | 'sell'; entry: number; time: number } | null;
  const close = (exit: number, time: number, reason: BtTrade['reason']) => {
    if (!pos) return;
    const dir = pos.side === 'buy' ? 1 : -1;
    const pnl = ((exit - pos.entry) * dir - p.costPrice) * p.units;
    trades.push({ side: pos.side, entryTime: pos.time, entry: pos.entry, exitTime: time, exit, pnl, reason });
    pos = null;
  };
  for (let i = 1; i < bars.length; i++) {
    const b = bars[i];
    if (!b) continue;
    // 1. signal from the previous two closed bars, known at this bar's open and
    //    executed there: the open comes before anything else in the bar
    const [f1, s1, f0, s0] = [f[i - 1], s[i - 1], f[i - 2], s[i - 2]];
    if (f1 != null && s1 != null && f0 != null && s0 != null) {
      const up = f0 <= s0 && f1 > s1;
      const down = f0 >= s0 && f1 < s1;
      const side = up ? 'buy' : down ? 'sell' : null;
      if (side && pos?.side !== side) {
        close(b.open, b.time, 'signal');
        pos = { side, entry: b.open, time: b.time };
      }
    }
    // 2. protective exits inside this bar for the open position, including one
    //    opened at this bar's open (its stop / target are live for the whole bar)
    if (pos && (p.stopPrice || p.takePrice)) {
      const long = pos.side === 'buy';
      const stop = p.stopPrice ? pos.entry + (long ? -p.stopPrice : p.stopPrice) : null;
      const take = p.takePrice ? pos.entry + (long ? p.takePrice : -p.takePrice) : null;
      if (stop !== null && (long ? b.low <= stop : b.high >= stop)) {
        // a bar that opens beyond the stop (gap) fills at the open, not at the stop
        close(long ? Math.min(stop, b.open) : Math.max(stop, b.open), b.time, 'stop');
      } else if (take !== null && (long ? b.high >= take : b.low <= take)) close(take, b.time, 'take');
    }
  }
  const last = bars[bars.length - 1];
  if (pos && last) close(last.close, last.time, 'end');
  const equity: number[] = [];
  let run = 0;
  let peak = 0;
  let dd = 0;
  for (const t of trades) {
    run += t.pnl;
    equity.push(run);
    peak = Math.max(peak, run);
    dd = Math.max(dd, peak - run);
  }
  const gains = trades.filter((t) => t.pnl > 0).reduce((a, t) => a + t.pnl, 0);
  const losses = -trades.filter((t) => t.pnl < 0).reduce((a, t) => a + t.pnl, 0);
  return {
    trades,
    equity,
    net: run,
    wins: trades.filter((t) => t.pnl > 0).length,
    maxDrawdown: dd,
    profitFactor: losses > 0 ? gains / losses : null,
  };
}
