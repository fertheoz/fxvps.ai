import { expect, test } from '@playwright/test';

/**
 * Protocol v1.2 against a real client-gateway (--demo), hedging account DEMO-H1:
 * SL/TP placement and modify, several positions per symbol, partial close and the
 * server-side deal history.
 */
test('hedging account: SL/TP set + modify, partial close, history from server', async ({ page }) => {
  const url = process.env.FXVPS_WS_URL;
  const token = process.env.FXVPS_DEMO_TOKEN;
  expect(url, 'global-setup must start the gateway').toBeTruthy();
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));

  await page.goto(`/?api=ws&url=${encodeURIComponent(url!)}&token=${encodeURIComponent(token!)}`);
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 20_000 });
  const account = page.getByLabel('Account');
  await account.selectOption('DEMO-H1');
  // leverage and margin mode come from the server
  await expect(account.locator('option:checked')).toContainText('1:30');
  await expect(account.locator('option:checked')).toContainText('Hedging');
  await expect(page.getByTestId('bid-EURUSD')).toHaveText(/^\d\.\d{5}$/, { timeout: 20_000 });

  // The gateway outlives Playwright retries: work relative to what is already there.
  const positions = page.getByTestId('positions-table');
  const posIds = async () =>
    (await positions.locator('[data-testid^="modify-"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-testid')!.slice(7))));
  await page.waitForTimeout(500);
  const before = new Set(await posIds());
  const posRows = before.size;
  await page.getByTestId('tab-history').click();
  await page.getByTestId('history-refresh').click();
  await page.waitForTimeout(1_000);
  const historyRows = await page.getByTestId('history-table').getByRole('row').filter({ hasText: 'EURUSD' }).count();
  await page.getByTestId('tab-positions').click();

  // Market order with SL / TP in pips.
  await page.getByTestId('ticket-volume').fill('0.20');
  await page.getByLabel('Stop Loss').first().fill('30');
  await page.getByLabel('Take Profit').first().fill('40');
  await page.getByTestId('ticket-submit').click();
  await expect(page.getByTestId('toast').first()).toContainText('Filled BUY 0.20 EURUSD', { timeout: 15_000 });

  await expect.poll(async () => (await posIds()).length, { timeout: 10_000 }).toBe(posRows + 1);
  const id = (await posIds()).find((x) => !before.has(x))!;
  const slCell = positions.getByTestId(`sl-${id}`);
  await expect(slCell).toHaveText(/^\d\.\d{5}$/, { timeout: 10_000 });
  const tpCell = positions.getByTestId(`tp-${id}`);
  await expect(tpCell).toHaveText(/^\d\.\d{5}$/);
  const sl = Number(await slCell.textContent());
  const tp = Number(await tpCell.textContent());
  expect(tp - sl).toBeCloseTo(0.007, 4);

  // Hedging: a second buy is a second position in the same symbol.
  await page.getByLabel('Stop Loss').first().fill('');
  await page.getByLabel('Take Profit').first().fill('');
  await page.getByTestId('ticket-volume').fill('0.10');
  await page.getByTestId('ticket-submit').click();
  await expect(positions.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(posRows + 2, { timeout: 15_000 });

  // Modify TP of the first position (server ModifyPosition).
  const newTp = (tp + 0.001).toFixed(5);
  await page.getByTestId(`modify-${id}`).click();
  await page.getByTestId(`tp-input-${id}`).fill(newTp);
  await page.getByTestId(`save-${id}`).click();
  await expect(tpCell).toHaveText(newTp, { timeout: 10_000 });
  await expect(positions.getByTestId(`sl-${id}`)).toHaveText(sl.toFixed(5));

  // Partial close: 0.10 of 0.20.
  await page.getByTestId(`partial-input-${id}`).fill('0.10');
  await page.getByTestId(`partial-${id}`).click();
  await expect(page.getByTestId('toast').filter({ hasText: `Closed #${id}` })).toHaveCount(1, { timeout: 15_000 });
  const row = positions.getByRole('row').filter({ has: page.getByTestId(`sl-${id}`) });
  await expect(row).toContainText('0.10');
  await expect(positions.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(posRows + 2);

  // History tab, reloaded from the server: 2 entries + 1 exit with realized P/L.
  await page.getByTestId('tab-history').click();
  await page.getByTestId('history-refresh').click();
  const history = page.getByTestId('history-table');
  await expect(history.getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(historyRows + 3, { timeout: 10_000 });
  const mine = history.getByRole('row').filter({ has: page.getByTestId(`deal-pos-${id}`) });
  await expect(mine).toHaveCount(2);
  await expect(mine.filter({ has: page.getByTestId('deal-entry-out') })).toHaveCount(1);
  await expect(mine.filter({ has: page.getByTestId('deal-entry-in') })).toHaveCount(1);

  // After a reload the history still comes from the server.
  await page.reload();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 20_000 });
  await page.getByLabel('Account').selectOption('DEMO-H1');
  await page.getByTestId('tab-history').click();
  await expect(page.getByTestId('history-table').getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(historyRows + 3, { timeout: 10_000 });

  expect(errors).toEqual([]);
});
