/**
 * Economic calendar (plan item 10): release events from the broker's console
 * (`/api/client/calendar`), shown in the Toolbox "Calendar" tab and as pins
 * on the chart time axis for the symbol's base / quote currency.
 */
export type EconImpact = 'low' | 'medium' | 'high';

export interface EconEvent {
  id: string;
  /** Release time, epoch ms (UTC). */
  time: number;
  currency: string;
  title: string;
  impact: EconImpact;
  actual: string | null;
  forecast: string | null;
  previous: string | null;
}

export const IMPACT_RANK: Record<EconImpact, number> = { low: 0, medium: 1, high: 2 };
export const IMPACT_COLOR: Record<EconImpact, string> = { low: '#9aa4b2', medium: '#f59e0b', high: '#ef4444' };

/** Currencies an event list is filtered by for a symbol (EURUSD -> EUR, USD; XAUUSD -> XAU, USD). */
export function symbolCurrencies(spec: { base: string; quote: string }): string[] {
  return [...new Set([spec.base, spec.quote].map((c) => c.trim().toUpperCase()).filter((c) => /^[A-Z]{3}$/.test(c)))];
}

/** Events concerning one of `currencies` (`ALL` concerns every one) at `minImpact` or above. */
export function eventsFor(events: readonly EconEvent[], currencies: readonly string[], minImpact: EconImpact = 'low'): EconEvent[] {
  return events.filter((e) => (e.currency === 'ALL' || currencies.includes(e.currency)) && IMPACT_RANK[e.impact] >= IMPACT_RANK[minImpact]);
}

/** Local calendar day key `YYYY-MM-DD` of epoch ms. */
export function localDay(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Events grouped by local day, in time order (days and rows). */
export function groupByDay(events: readonly EconEvent[]): [string, EconEvent[]][] {
  const out = new Map<string, EconEvent[]>();
  for (const e of [...events].sort((a, b) => a.time - b.time || IMPACT_RANK[b.impact] - IMPACT_RANK[a.impact])) {
    const k = localDay(e.time);
    const list = out.get(k);
    if (list) list.push(e);
    else out.set(k, [e]);
  }
  return [...out.entries()];
}

/** A chart pin: one per bar that holds at least one release. */
export interface EventPin {
  /** Bar open time, epoch seconds. */
  time: number;
  impact: EconImpact;
  color: string;
  /** Currencies of the bar's releases, e.g. `USD` or `EUR·USD`. */
  text: string;
  events: EconEvent[];
}

/**
 * Pins on bars: each release goes to the bar whose period `[time, time + tf)`
 * holds it; releases in a gap (weekend) or past the last bar are not pinned.
 * `barTimes` are ascending bar open times in seconds.
 */
export function eventPins(events: readonly EconEvent[], barTimes: readonly number[], tfSeconds: number): EventPin[] {
  if (!barTimes.length || tfSeconds <= 0) return [];
  const byBar = new Map<number, EconEvent[]>();
  for (const e of events) {
    const t = Math.floor(e.time / 1000);
    // last bar opening at or before t
    let lo = 0;
    let hi = barTimes.length - 1;
    if (t < barTimes[0]!) continue;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (barTimes[mid]! <= t) lo = mid;
      else hi = mid - 1;
    }
    const open = barTimes[lo]!;
    if (t >= open + tfSeconds) continue;
    const list = byBar.get(open);
    if (list) list.push(e);
    else byBar.set(open, [e]);
  }
  return [...byBar.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([time, list]) => {
      const sorted = [...list].sort((a, b) => IMPACT_RANK[b.impact] - IMPACT_RANK[a.impact] || a.time - b.time);
      const impact = sorted[0]!.impact;
      return { time, impact, color: IMPACT_COLOR[impact], text: [...new Set(sorted.map((e) => e.currency))].sort().join('·'), events: sorted };
    });
}

/** The next release at or after `now` (ms), if any. */
export function nextEvent(events: readonly EconEvent[], now: number): EconEvent | undefined {
  let best: EconEvent | undefined;
  for (const e of events) if (e.time >= now && (!best || e.time < best.time || (e.time === best.time && IMPACT_RANK[e.impact] > IMPACT_RANK[best.impact]))) best = e;
  return best;
}

/** "in 2h 05m" style countdown parts (ms >= 0). */
export function countdown(ms: number): { h: number; m: number } {
  const min = Math.max(0, Math.round(ms / 60_000));
  return { h: Math.floor(min / 60), m: min % 60 };
}

/** Demo calendar for the mock terminal (no gateway): a few releases around `now`. */
export function demoEvents(now: number): EconEvent[] {
  const hour = 3_600_000;
  const base = now - (now % hour);
  const ev = (id: string, h: number, currency: string, title: string, impact: EconImpact, forecast: string | null, previous: string | null, actual: string | null = null): EconEvent => ({
    id,
    time: base + h * hour + 30 * 60_000,
    currency,
    title,
    impact,
    actual,
    forecast,
    previous,
  });
  return [
    ev('d1', -50, 'EUR', 'German Industrial Production m/m', 'medium', '0.4%', '1.3%', '-0.2%'),
    ev('d2', -27, 'USD', 'ISM Services PMI', 'high', '51.7', '52.0', '50.8'),
    ev('d3', -5, 'GBP', 'BOE Gov Speaks', 'medium', null, null),
    ev('d4', -3, 'USD', 'Unemployment Claims', 'medium', '225K', '218K', '231K'),
    ev('d5', -3, 'EUR', 'ECB Monetary Policy Statement', 'high', null, null),
    ev('d6', -1, 'JPY', 'Household Spending y/y', 'low', '1.1%', '2.3%', '0.9%'),
    ev('d7', 2, 'USD', 'CPI m/m', 'high', '0.3%', '0.4%'),
    ev('d8', 6, 'CAD', 'Employment Change', 'high', '15.2K', '-40.8K'),
    ev('d9', 20, 'AUD', 'RBA Rate Statement', 'high', null, null),
    ev('d10', 30, 'USD', 'Crude Oil Inventories', 'low', '-1.2M', '2.1M'),
    ev('d11', 50, 'USD', 'Non-Farm Employment Change', 'high', '140K', '22K'),
  ];
}
