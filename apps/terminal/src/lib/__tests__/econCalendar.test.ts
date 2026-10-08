import { describe, expect, it } from 'vitest';
import { countdown, demoEvents, eventPins, eventsFor, groupByDay, IMPACT_COLOR, localDay, nextEvent, symbolCurrencies, type EconEvent } from '../econCalendar';

const ev = (id: string, iso: string, currency: string, impact: EconEvent['impact'] = 'high'): EconEvent => ({
  id,
  time: Date.parse(iso),
  currency,
  title: `${currency} release ${id}`,
  impact,
  actual: null,
  forecast: null,
  previous: null,
});

describe('economic calendar helpers', () => {
  it('maps a symbol to its currencies', () => {
    expect(symbolCurrencies({ base: 'EUR', quote: 'USD' })).toEqual(['EUR', 'USD']);
    expect(symbolCurrencies({ base: 'xau', quote: 'USD' })).toEqual(['XAU', 'USD']);
    expect(symbolCurrencies({ base: 'US30', quote: 'USD' })).toEqual(['USD']);
  });

  it('filters by currency and impact; ALL concerns every symbol', () => {
    const list = [ev('a', '2026-10-09T12:30:00Z', 'USD'), ev('b', '2026-10-09T08:00:00Z', 'GBP'), ev('c', '2026-10-09T09:00:00Z', 'ALL', 'medium'), ev('d', '2026-10-09T10:00:00Z', 'EUR', 'low')];
    expect(eventsFor(list, ['EUR', 'USD']).map((e) => e.id)).toEqual(['a', 'c', 'd']);
    expect(eventsFor(list, ['EUR', 'USD'], 'medium').map((e) => e.id)).toEqual(['a', 'c']);
    expect(eventsFor(list, ['EUR', 'USD'], 'high').map((e) => e.id)).toEqual(['a']);
  });

  it('pins each release on the bar whose period holds it', () => {
    // M15 bars 12:00, 12:15, 12:30 then a gap (no 12:45 bar), then 13:30
    const t0 = Date.parse('2026-10-09T12:00:00Z') / 1000;
    const bars = [t0, t0 + 900, t0 + 1800, t0 + 5400];
    const list = [
      ev('nfp', '2026-10-09T12:30:00Z', 'USD', 'high'),
      ev('ur', '2026-10-09T12:30:00Z', 'USD', 'medium'),
      ev('ip', '2026-10-09T12:20:00Z', 'EUR', 'medium'),
      ev('gap', '2026-10-09T13:00:00Z', 'USD'),
      ev('early', '2026-10-09T11:00:00Z', 'USD'),
      ev('late', '2026-10-09T13:45:00Z', 'USD'),
      ev('last', '2026-10-09T13:44:59Z', 'EUR', 'low'),
    ];
    const pins = eventPins(list, bars, 900);
    expect(pins.map((p) => [p.time - t0, p.impact, p.text])).toEqual([
      [900, 'medium', 'EUR'],
      [1800, 'high', 'USD'],
      [5400, 'low', 'EUR'],
    ]);
    // the pin takes the strongest impact's colour and lists that release first
    expect(pins[1]!.color).toBe(IMPACT_COLOR.high);
    expect(pins[1]!.events.map((e) => e.id)).toEqual(['nfp', 'ur']);
    expect(eventPins(list, [], 900)).toEqual([]);
  });

  it('groups by local day in time order and finds the next release', () => {
    const list = [ev('b', '2026-10-09T15:00:00', 'USD', 'low'), ev('a', '2026-10-09T09:00:00', 'EUR'), ev('c', '2026-10-10T09:00:00', 'JPY')];
    const days = groupByDay(list);
    expect(days.map(([d, l]) => [d, l.map((e) => e.id)])).toEqual([
      ['2026-10-09', ['a', 'b']],
      ['2026-10-10', ['c']],
    ]);
    expect(localDay(Date.parse('2026-10-10T09:00:00'))).toBe('2026-10-10');
    expect(nextEvent(list, Date.parse('2026-10-09T10:00:00'))?.id).toBe('b');
    expect(nextEvent(list, Date.parse('2026-10-11T00:00:00'))).toBeUndefined();
    expect(countdown(2 * 3_600_000 + 5 * 60_000)).toEqual({ h: 2, m: 5 });
  });

  it('has a demo calendar around now for the mock terminal', () => {
    const now = Date.parse('2026-10-08T10:10:00Z');
    const demo = demoEvents(now);
    expect(demo.some((e) => e.time < now)).toBe(true);
    expect(demo.some((e) => e.time > now && e.impact === 'high')).toBe(true);
    expect(new Set(demo.map((e) => e.id)).size).toBe(demo.length);
  });
});
