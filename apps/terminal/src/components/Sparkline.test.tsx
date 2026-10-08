import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Sparkline } from './Sparkline';
import { sparkPath } from '../lib/sparkline';
import { CopyTab } from './CopyTab';
import { clientApi, type ClientAccount, type CopyOverview } from '../api/clientApi';

describe('sparkPath', () => {
  it('spans the box: highest point at the top, lowest at the bottom', () => {
    expect(sparkPath([0, 10, 5], 100, 20)).toBe('M0.0,18.0 L50.0,2.0 L100.0,10.0');
  });
  it('centres a flat series and needs two points', () => {
    expect(sparkPath([3, 3], 10, 20)).toBe('M0.0,10.0 L10.0,10.0');
    expect(sparkPath([1], 10, 20)).toBe('');
  });
});

describe('<Sparkline />', () => {
  it('colours by the direction of the series', () => {
    const { rerender } = render(<Sparkline points={[0, -2, 1]} label="curve" />);
    expect(screen.getByTestId('sparkline').getAttribute('class')).toBe('text-up');
    rerender(<Sparkline points={[0, 2, -1]} label="curve" />);
    expect(screen.getByTestId('sparkline').getAttribute('class')).toBe('text-down');
  });
});

describe('<CopyTab /> showcase', () => {
  afterEach(() => vi.restoreAllMocks());
  const acc: ClientAccount = { externalId: 'DEMO-1', login: 100001, name: 'A', group: 'demo-retail', currency: 'USD', balance: 0, equity: 0, margin: 0, kyc: 'approved' };

  it('shows max drawdown and the equity curve', async () => {
    const overview: CopyOverview = {
      strategies: [
        { account: 7, name: 'Alpha', description: '', perfFeeBps: 2000, public: true, pnl30d: 5000, return30dPct: 5, deals30d: 3, winRate: 0.66, followers: 2, maxDrawdownPct: 4.2, returnCurvePct: [0, 1.5, -2.1, 5] },
        { account: 8, name: 'Beta', description: '', perfFeeBps: 0, public: true, pnl30d: 0, return30dPct: null, deals30d: 0, winRate: null, followers: 0 },
      ],
      subscriptions: [],
    };
    vi.spyOn(clientApi, 'copy').mockResolvedValue(overview);
    render(<CopyTab acc={acc} />);
    expect(await screen.findByTestId('copy-maxdd-7')).toHaveTextContent('4.2%');
    expect(screen.getByTestId('copy-curve-7').querySelector('path')?.getAttribute('d')).toMatch(/^M0\.0,/);
    // an older core without the fields: no curve, a dash
    expect(screen.getByTestId('copy-maxdd-8')).toHaveTextContent('—');
    expect(screen.queryByTestId('copy-curve-8')).toBeNull();
  });
});
