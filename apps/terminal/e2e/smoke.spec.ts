import { expect, test } from '@playwright/test';

test('quotes stream, market order fills, position appears', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto('/');

  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected');

  // quotes are visible and moving
  const bid = page.getByTestId('bid-EURUSD');
  await expect(bid).toHaveText(/^\d\.\d{5}$/);
  const first = await bid.textContent();
  await expect.poll(async () => bid.textContent(), { timeout: 15_000 }).not.toBe(first);

  // chart rendered (lightweight-charts draws into canvases)
  await expect(page.getByTestId('chart-0').locator('canvas').first()).toBeVisible();

  // place a market order via the ticket
  await page.getByTestId('ticket-volume').fill('0.25');
  await page.getByTestId('ticket-submit').click();
  await expect(page.getByTestId('toast').first()).toContainText('Filled BUY 0.25 EURUSD');

  const positions = page.getByTestId('positions-table');
  await expect(positions.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(1);
  await expect(positions).toContainText('0.25');
  await expect(page.getByTestId('margin')).not.toHaveText('0.00');

  // one-click sell from the chart overlay
  await page.getByTestId('oneclick-sell-0').click();
  await expect(positions.getByRole('row').filter({ hasText: 'sell' })).toHaveCount(1);

  expect(errors).toEqual([]);
});
