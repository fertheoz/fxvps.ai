import { beforeEach, describe, expect, it } from 'vitest';
import { selectPositions, useTerminal } from './terminal';
import { q, specs } from '../test/fixtures';

describe('terminal store', () => {
  beforeEach(() => {
    useTerminal.setState({ quotes: {}, tickDir: {}, positions: {}, journal: [], accounts: [], activeAccountId: null });
  });

  it('tracks tick direction', () => {
    const s = useTerminal.getState();
    s.applyQuotes([q('EURUSD', 1.1, 1.1001)]);
    s.applyQuotes([q('EURUSD', 1.1002, 1.1003)]);
    expect(useTerminal.getState().tickDir.EURUSD).toBe(1);
    s.applyQuotes([q('EURUSD', 1.0999, 1.1)]);
    expect(useTerminal.getState().tickDir.EURUSD).toBe(-1);
  });

  it('applies trading events and selects active account data', () => {
    const s = useTerminal.getState();
    s.setReference(Object.values(specs), [
      { id: 'a', name: 'A', currency: 'USD', balance: 1, leverage: 100, isDemo: true },
      { id: 'b', name: 'B', currency: 'USD', balance: 1, leverage: 100, isDemo: true, parentId: 'a' },
    ]);
    expect(useTerminal.getState().activeAccountId).toBe('a');
    const pos = { id: '1', accountId: 'b', symbol: 'EURUSD', side: 'buy' as const, volume: 1, openPrice: 1, openTime: 0, commission: 0, swap: 0 };
    s.applyEvent({ type: 'positions', accountId: 'b', positions: [pos] });
    expect(selectPositions(useTerminal.getState())).toEqual([]);
    s.setActiveAccount('b');
    expect(selectPositions(useTerminal.getState())).toEqual([pos]);
    s.applyEvent({ type: 'account', account: { id: 'b', name: 'B', currency: 'USD', balance: 99, leverage: 100, isDemo: true } });
    expect(useTerminal.getState().accounts[1]!.balance).toBe(99);
  });

  it('caps journal length', () => {
    const s = useTerminal.getState();
    for (let i = 0; i < 2100; i++) s.applyEvent({ type: 'journal', entry: { id: String(i), time: i, level: 'info', message: 'x' } });
    const j = useTerminal.getState().journal;
    expect(j).toHaveLength(2000);
    expect(j.at(-1)!.id).toBe('2099');
  });

  it('layout clamps active chart and theme toggles', () => {
    const s = useTerminal.getState();
    s.setLayout(4);
    s.setActiveChart(3);
    s.setLayout(2);
    expect(useTerminal.getState().activeChart).toBe(1);
    const before = useTerminal.getState().theme;
    s.toggleTheme();
    expect(useTerminal.getState().theme).not.toBe(before);
  });
});
