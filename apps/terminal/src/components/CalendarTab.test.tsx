import { describe, expect, it, beforeAll } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { CalendarTab } from './CalendarTab';
import { useTerminal } from '../store/terminal';
import { bootstrap } from '../store/api';
import { MockTradingApi } from '@fxvps/trading-core';

describe('<CalendarTab />', () => {
  beforeAll(async () => {
    await bootstrap(new MockTradingApi({ latencyMs: 0, tickIntervalMs: 0 }));
    useTerminal.setState({ charts: [{ symbol: 'EURUSD', timeframe: 'M5' }], activeChart: 0 });
  });

  it('lists the (demo) calendar by day and filters by currency and impact', async () => {
    const user = userEvent.setup();
    render(<CalendarTab />);
    // mock terminal (no gateway): the demo calendar, from yesterday on
    const rows = await screen.findAllByTestId(/^calendar-event-/);
    expect(rows.length).toBeGreaterThan(3);
    expect(screen.getAllByTestId(/^calendar-day-/).length).toBeGreaterThan(0);
    const tab = screen.getByTestId('calendar-tab');

    await user.selectOptions(screen.getByTestId('calendar-currency'), 'USD');
    const usd = within(tab).getAllByTestId(/^calendar-event-/);
    expect(usd.length).toBeGreaterThan(0);
    expect(usd.every((r) => r.textContent?.includes('USD'))).toBe(true);

    // the active chart's pair: EUR and USD releases only
    await user.selectOptions(screen.getByTestId('calendar-currency'), '__chart__');
    const pair = within(tab).getAllByTestId(/^calendar-event-/);
    expect(pair.every((r) => /EUR|USD/.test(r.textContent ?? ''))).toBe(true);

    await user.selectOptions(screen.getByTestId('calendar-currency'), '');
    await user.selectOptions(screen.getByTestId('calendar-impact'), 'high');
    const high = within(tab).getAllByTestId(/^calendar-event-/);
    expect(high.length).toBeLessThan(rows.length);
    expect(high.every((r) => r.getAttribute('data-impact') === 'high')).toBe(true);
  });
});
