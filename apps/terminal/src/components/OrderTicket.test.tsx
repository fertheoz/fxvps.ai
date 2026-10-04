import { describe, expect, it, beforeAll, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { OrderTicket } from './OrderTicket';
import { useTerminal } from '../store/terminal';
import { bootstrap } from '../store/api';
import { MockTradingApi } from '@fxvps/trading-core';

describe('<OrderTicket />', () => {
  let api: MockTradingApi;
  beforeAll(async () => {
    api = new MockTradingApi({ latencyMs: 0, tickIntervalMs: 0 });
    await bootstrap(api);
    await new Promise((r) => setTimeout(r, 30)); // let the rAF batcher flush quotes
  });

  it('shows margin preview and blocks invalid volume', async () => {
    const user = userEvent.setup();
    const spy = vi.spyOn(api, 'placeOrder');
    render(<OrderTicket preset={{ symbol: 'EURUSD', side: 'buy', type: 'market' }} />);
    expect(screen.getByTestId('ticket-margin').textContent).toMatch(/^\d{1,3}(,\d{3})*\.\d{2}$/);
    const vol = screen.getByTestId('ticket-volume');
    await user.clear(vol);
    await user.type(vol, '0.001');
    await user.click(screen.getByTestId('ticket-submit'));
    expect(await screen.findByText('Enter a volume')).toBeInTheDocument();
    expect(spy).not.toHaveBeenCalled();
  });

  it('places a valid market order', async () => {
    const user = userEvent.setup();
    render(<OrderTicket preset={{ symbol: 'EURUSD', side: 'sell', type: 'market' }} />);
    const vol = screen.getAllByTestId('ticket-volume').at(-1)!;
    await user.clear(vol);
    await user.type(vol, '0.20');
    await user.click(screen.getAllByTestId('ticket-submit').at(-1)!);
    await vi.waitFor(() => {
      const acc = useTerminal.getState().activeAccountId!;
      expect(useTerminal.getState().positions[acc]?.some((p) => p.side === 'sell' && p.volume === 20)).toBe(true);
    });
  });
});
