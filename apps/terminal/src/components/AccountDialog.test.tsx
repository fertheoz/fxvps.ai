import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ClientAccount, ClientMe, Statement } from '../api/clientApi';
import { useTerminal } from '../store/terminal';

const api = vi.hoisted(() => ({ me: vi.fn(), statement: vi.fn() }));
vi.mock('../api/clientApi', async (orig) => {
  const m = await orig<typeof import('../api/clientApi')>();
  return { ...m, clientApiBase: () => 'https://gw.test/api/client', clientApi: { ...m.clientApi, ...api } };
});
vi.mock('../lib/referral', () => ({ sendPendingReferral: vi.fn() }));

import { AccountDialog } from './AccountDialog';

const account = (externalId: string, login: number): ClientAccount => ({
  externalId, login, name: 'Ada', group: 'c', currency: 'USD', balance: 100_000, equity: 100_000, margin: 0, kyc: 'approved',
});
const me: ClientMe = {
  subject: 's', name: 'Ada', accounts: [account('A-1', 1), account('B-2', 2)], unknownAccounts: [], funding: [], documents: [],
  instructions: { usdtTrc20Address: '', bankDetails: '', minDepositMinor: 0, minWithdrawMinor: 0 },
};
const statement = (acc: number, trades: number): Statement => ({
  account: acc, name: 'Ada', broker: 'B', group: 'c', currency: 'USD', from: '2026-09-01T00:00:00.000Z', to: null,
  generatedAt: '2026-10-08T00:00:00.000Z', balance: 0, equity: 0, margin: 0, trades: [],
  totals: { trades, lots: trades, pnl: 0, commission: 0, swap: 0 }, positions: [], cash: [],
});

afterEach(() => {
  cleanup();
  useTerminal.getState().setAccountOpen(false);
});

describe('<AccountDialog /> statement tab', () => {
  it('starts empty after switching account and drops a late load of the old one', async () => {
    const user = userEvent.setup();
    api.me.mockResolvedValue(me);
    let finishA: (s: Statement) => void = () => {};
    api.statement.mockImplementation((acc: string) =>
      acc === 'A-1' ? new Promise<Statement>((r) => { finishA = r; }) : Promise.resolve(statement(2, 3)),
    );
    useTerminal.getState().setAccountOpen(true);
    render(<AccountDialog />);
    await user.click(screen.getByTestId('acct-tab-statement'));
    await screen.findByTestId('statement-tab');
    // account A: the load is still in flight when the client switches to B
    await user.click(screen.getByTestId('stmt-load'));
    expect(api.statement).toHaveBeenLastCalledWith('A-1', expect.any(String), expect.any(String));
    await user.selectOptions(screen.getByRole('combobox', { name: /account/i }), 'B-2');
    expect(screen.queryByTestId('stmt-summary')).toBeNull();
    await act(async () => finishA(statement(1, 9)));
    expect(screen.queryByTestId('stmt-summary')).toBeNull();
    // B's own statement shows up when asked for
    await user.click(screen.getByTestId('stmt-load'));
    expect(api.statement).toHaveBeenLastCalledWith('B-2', expect.any(String), expect.any(String));
    expect((await screen.findByTestId('stmt-summary')).textContent).toContain(': 3 ·');
  });

  it('keeps a loaded statement on screen only for its own account', async () => {
    const user = userEvent.setup();
    api.me.mockResolvedValue(me);
    api.statement.mockImplementation((acc: string) => Promise.resolve(acc === 'A-1' ? statement(1, 9) : statement(2, 3)));
    useTerminal.getState().setAccountOpen(true);
    render(<AccountDialog />);
    await user.click(screen.getByTestId('acct-tab-statement'));
    await screen.findByTestId('statement-tab');
    await user.click(screen.getByTestId('stmt-load'));
    expect((await screen.findByTestId('stmt-summary')).textContent).toContain(': 9 ·');
    await user.selectOptions(screen.getByRole('combobox', { name: /account/i }), 'B-2');
    expect(screen.queryByTestId('stmt-summary')).toBeNull();
  });
});
