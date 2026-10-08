import { beforeAll, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { backtestMaCross, MockTradingApi } from '@fxvps/trading-core';
import { bootstrap } from '../store/api';
import { useTerminal } from '../store/terminal';
import { StrategyTester } from './StrategyTester';

// spy on the engine call to see the parameters the form passes
vi.mock('@fxvps/trading-core', async (importOriginal) => {
  const m = await importOriginal<typeof import('@fxvps/trading-core')>();
  return { ...m, backtestMaCross: vi.fn(m.backtestMaCross) };
});

describe('<StrategyTester />', () => {
  beforeAll(async () => {
    await bootstrap(new MockTradingApi({ latencyMs: 0, tickIntervalMs: 0 }));
    await new Promise((r) => setTimeout(r, 30)); // let the rAF batcher flush quotes
  });

  it('takes fractional lots typed key by key and runs with them', async () => {
    const user = userEvent.setup();
    render(<StrategyTester symbol="EURUSD" timeframe="H1" onClose={() => {}} />);
    const lots = screen.getByTestId('bt-lots');
    await user.clear(lots);
    await user.type(lots, '0.1'); // "0." must survive the keystroke
    expect(lots).toHaveValue('0.1');
    await user.click(screen.getByTestId('bt-run'));
    await screen.findByTestId('bt-result');
    const spec = useTerminal.getState().symbols.EURUSD!;
    expect(vi.mocked(backtestMaCross).mock.calls.at(-1)?.[1].units).toBeCloseTo(0.1 * spec.contractSize);
  });

  it('reads a comma decimal (TR keyboard) as the value shown', async () => {
    const user = userEvent.setup();
    render(<StrategyTester symbol="EURUSD" timeframe="H1" onClose={() => {}} />);
    const lots = screen.getByTestId('bt-lots');
    await user.clear(lots);
    await user.type(lots, '0,1');
    const sl = screen.getByTestId('bt-sl');
    await user.clear(sl);
    await user.type(sl, '15,5');
    expect(lots).toHaveValue('0,1');
    await user.click(screen.getByTestId('bt-run'));
    await screen.findByTestId('bt-result');
    const spec = useTerminal.getState().symbols.EURUSD!;
    const p = vi.mocked(backtestMaCross).mock.calls.at(-1)?.[1];
    expect(p?.units).toBeCloseTo(0.1 * spec.contractSize);
    expect(p?.stopPrice).toBeCloseTo(15.5 / 10 ** spec.digits);
  });

  it.each([
    ['bt-lots', 'abc'],
    ['bt-lots', '0'],
    ['bt-fast', '2.5'],
    ['bt-sl', '-3'],
  ])('refuses %s = %s instead of running with a substitute', async (id, text) => {
    const user = userEvent.setup();
    render(<StrategyTester symbol="EURUSD" timeframe="H1" onClose={() => {}} />);
    const before = vi.mocked(backtestMaCross).mock.calls.length;
    const field = screen.getByTestId(id);
    await user.clear(field);
    await user.type(field, text);
    await user.click(screen.getByTestId('bt-run'));
    expect(await screen.findByTestId('bt-err')).toBeInTheDocument();
    expect(screen.queryByTestId('bt-result')).toBeNull();
    expect(vi.mocked(backtestMaCross).mock.calls.length).toBe(before);
  });
});
