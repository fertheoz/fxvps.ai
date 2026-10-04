import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { MockTradingApi } from '../api/mock';
import { useTerminal } from '../store/terminal';
import { prepareChartWindow } from '../native';
import { ChartWindow } from './ChartWindow';

// lightweight-charts needs a real canvas; the panel itself is covered by e2e.
vi.mock('./ChartPanel', () => ({
  ChartPanel: ({ detached }: { detached?: boolean }) => <div data-testid="panel">{detached ? 'detached' : 'docked'}</div>,
}));

describe('ChartWindow', () => {
  it('renders a single chart for the requested symbol without touching localStorage', async () => {
    localStorage.clear();
    const view = { symbol: 'XAUUSD', timeframe: 'H1' as const };
    prepareChartWindow(view);
    render(<ChartWindow api={new MockTradingApi()} view={view} />);
    expect(await screen.findByTestId('chart-window', {}, { timeout: 5000 })).toBeInTheDocument();
    expect(useTerminal.getState().charts[0]).toEqual({ symbol: 'XAUUSD', timeframe: 'H1' });
    expect(screen.getByTestId('panel')).toHaveTextContent('detached');
    expect(localStorage.getItem('fxvps-terminal')).toBeNull();
  });
});
