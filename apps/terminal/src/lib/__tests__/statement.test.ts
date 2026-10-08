import { describe, expect, it } from 'vitest';
import { cashSigned, type Statement } from '../../api/clientApi';
import { statementHtml, type StatementLabelKey } from '../statement';

const labels = Object.fromEntries(
  (
    ['title', 'generated', 'summary', 'balance', 'equity', 'closed', 'pnl', 'commission', 'commissionOpen', 'swap', 'cash', 'date', 'amount',
      'deposit', 'withdraw', 'copy_fee', 'copy_fee_income', 'nbp', 'price', 'open'] as StatementLabelKey[]
  ).map((k) => [k, `L:${k}`]),
) as Record<StatementLabelKey, string>;

const stmt = (over: Partial<Statement> = {}): Statement => ({
  account: 5,
  name: 'Ada <script>',
  broker: 'Broker',
  group: 'c',
  currency: 'USD',
  from: '2026-09-01T00:00:00.000Z',
  to: '2026-09-30T23:59:59.999Z',
  generatedAt: '2026-10-08T10:00:00.000Z',
  balance: 100_000,
  equity: 100_000,
  margin: 0,
  trades: [],
  totals: { trades: 1, lots: 1, pnl: 4_900, commission: -1_050, commissionOpen: -700, swap: 0 },
  positions: [],
  cash: [
    { at: '2026-09-02T00:00:00.000Z', kind: 'deposit', amount: 10_000, reason: '' },
    { at: '2026-09-03T00:00:00.000Z', kind: 'copy_fee', amount: 980, reason: '' },
    { at: '2026-09-04T00:00:00.000Z', kind: 'nbp', amount: 120, reason: '' },
  ],
  ...over,
});

describe('statement', () => {
  it('signs cash rows by kind', () => {
    expect(cashSigned({ kind: 'deposit', amount: 5 })).toBe(5);
    expect(cashSigned({ kind: 'withdraw', amount: 5 })).toBe(-5);
    expect(cashSigned({ kind: 'copy_fee', amount: 5 })).toBe(-5);
    expect(cashSigned({ kind: 'copy_fee_income', amount: 5 })).toBe(5);
    expect(cashSigned({ kind: 'nbp', amount: 5 })).toBe(5);
  });

  it('prints copy fees and compensation as cash rows, and the opening commission', () => {
    const html = statementHtml(stmt(), labels);
    expect(html).toContain('<td>L:copy_fee</td><td>-9.80 USD</td>');
    expect(html).toContain('<td>L:nbp</td><td>1.20 USD</td>');
    expect(html).toContain('<td>L:deposit</td><td>100.00 USD</td>');
    expect(html).toContain('-10.50 USD (L:commissionOpen: -7.00 USD)');
    // client data is escaped
    expect(html).not.toContain('<script>');
  });

  it('omits the opening part when the server does not send it', () => {
    const html = statementHtml(stmt({ totals: { trades: 0, lots: 0, pnl: 0, commission: 0, swap: 0 } }), labels);
    expect(html).not.toContain('L:commissionOpen');
  });
});
