import { expect, test } from '@playwright/test';

/** Terminal against a real client-gateway (--demo): live quotes, market order, position. */
test('live EURUSD quotes, market order fills, position appears', async ({ page }) => {
  const url = process.env.FXVPS_WS_URL;
  const token = process.env.FXVPS_DEMO_TOKEN;
  expect(url, 'global-setup must start the gateway').toBeTruthy();
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));

  await page.goto(`/?api=ws&url=${encodeURIComponent(url!)}&token=${encodeURIComponent(token!)}`);
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 20_000 });
  await expect(page.getByTestId('gateway-button')).toHaveAttribute('data-api', 'ws');
  // The bearer token does not stay in the address bar.
  expect(page.url()).not.toContain('token=');

  // Live quotes from the simulator via fix-gateway -> client-gateway.
  const bid = page.getByTestId('bid-EURUSD');
  await expect(bid).toHaveText(/^\d\.\d{5}$/, { timeout: 20_000 });
  const first = await bid.textContent();
  await expect.poll(async () => bid.textContent(), { timeout: 20_000 }).not.toBe(first);

  // Market order through the ticket.
  await page.getByTestId('ticket-volume').fill('0.25');
  await page.getByTestId('ticket-submit').click();
  await expect(page.getByTestId('toast').first()).toContainText('Filled BUY 0.25 EURUSD', { timeout: 15_000 });

  const positions = page.getByTestId('positions-table');
  await expect(positions.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(1);
  await expect(positions).toContainText('0.25');

  // Reload: the gateway is remembered (sessionStorage) and the position comes back
  // from the AccountSnapshot.
  await page.reload();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 20_000 });
  await expect(positions.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(1);
  await expect(positions).toContainText('0.25');

  expect(errors).toEqual([]);
});
