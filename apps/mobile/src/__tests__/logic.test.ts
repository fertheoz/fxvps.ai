import { MOCK_SYMBOLS, toSpec, type Account, type Quote, type SymbolSpec } from '@fxvps/trading-core';
import { en } from '../i18n/en';
import { pickLang, translate } from '../i18n';
import { tr } from '../i18n/tr';
import { layoutCandles } from '../lib/candles';
import { evaluateTicket, parseOptional, type TicketForm } from '../lib/ticket';
import { dark, light, resolvePalette } from '../theme';

const specs: Record<string, SymbolSpec> = Object.fromEntries(MOCK_SYMBOLS.map((s) => [s.name, toSpec(s)]));
const q = (symbol: string, bid: number, ask: number): Quote => ({
  symbol,
  bid,
  ask,
  time: 1_700_000_000_000,
  dayOpen: bid,
  dayHigh: ask,
  dayLow: bid,
});
const quotes: Record<string, Quote> = { EURUSD: q('EURUSD', 1.085, 1.08506), GBPUSD: q('GBPUSD', 1.27, 1.27) };
const account: Account = { id: 'A1', name: 'Demo', currency: 'USD', balance: 1_000_000, leverage: 100, isDemo: true };
const base: TicketForm = { side: 'buy', type: 'market', lots: '0.10', price: '', sl: '', tp: '' };
const ctx = (freeMargin = 1_000_000) => ({
  account,
  spec: specs.EURUSD!,
  quote: quotes.EURUSD,
  freeMargin,
  specs,
  quotes,
  now: 1_700_000_000_000,
});

describe('i18n', () => {
  it('TR has every EN key, non-empty', () => {
    for (const k of Object.keys(en) as (keyof typeof en)[]) expect(tr[k]).toBeTruthy();
    expect(Object.keys(tr).sort()).toEqual(Object.keys(en).sort());
  });
  it('picks language from locale tags', () => {
    expect(pickLang(['tr-TR'])).toBe('tr');
    expect(pickLang(['de-DE', 'en-US'])).toBe('en');
    expect(pickLang([null])).toBe('en');
    expect(translate('tr', 'trade.buy')).toBe('Al');
  });
});

describe('theme', () => {
  it('resolves system/explicit preferences', () => {
    expect(resolvePalette('system', 'dark')).toBe(dark);
    expect(resolvePalette('system', null)).toBe(light);
    expect(resolvePalette('dark', 'light')).toBe(dark);
  });
});

describe('candles', () => {
  it('lays out bars within bounds, up/down colored', () => {
    const bars = [
      { time: 0, open: 1, high: 2, low: 0.5, close: 1.5, volume: 1 },
      { time: 60, open: 1.5, high: 1.6, low: 1, close: 1.1, volume: 1 },
    ];
    const l = layoutCandles(bars, 100, 104, 2);
    expect(l.candles).toHaveLength(2);
    expect(l.candles[0]!.up).toBe(true);
    expect(l.candles[1]!.up).toBe(false);
    expect(l.y(2)).toBe(2);
    expect(l.y(0.5)).toBe(102);
    for (const c of l.candles) expect(c.x + c.width).toBeLessThanOrEqual(100);
  });
  it('handles empty input', () => {
    expect(layoutCandles([], 100, 100).candles).toEqual([]);
  });
});

describe('ticket', () => {
  it('parses optional decimals with comma', () => {
    expect(parseOptional('1,25')).toBe(1.25);
    expect(parseOptional(' ')).toBeUndefined();
    expect(parseOptional('x')).toBeUndefined();
  });
  it('builds a market request with margin', () => {
    const ev = evaluateTicket(base, ctx());
    expect(ev.errors).toEqual([]);
    expect(ev.request).toMatchObject({ accountId: 'A1', symbol: 'EURUSD', side: 'buy', type: 'market', volume: 10 });
    expect(ev.requiredMargin).toBeGreaterThan(0);
  });
  it('rejects bad volume, wrong-side SL and insufficient margin', () => {
    expect(evaluateTicket({ ...base, lots: 'abc' }, ctx()).errors.map((e) => e.key)).toContain('volume.required');
    expect(evaluateTicket({ ...base, sl: '1.2' }, ctx()).errors.map((e) => e.key)).toContain('sl.side');
    const ev = evaluateTicket(base, ctx(1));
    expect(ev.errors.map((e) => e.key)).toContain('margin.insufficient');
    expect(ev.request).toBeUndefined();
  });
  it('requires a price for limit orders', () => {
    expect(evaluateTicket({ ...base, type: 'limit' }, ctx()).errors.map((e) => e.key)).toContain('price.required');
  });
});
