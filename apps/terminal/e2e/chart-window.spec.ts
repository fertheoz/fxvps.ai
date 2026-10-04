import { expect, test } from '@playwright/test';

test('detached chart view renders a single chart', async ({ page }) => {
  await page.goto('/?view=chart&symbol=XAUUSD&tf=H1');
  await expect(page.getByTestId('chart-window')).toBeVisible();
  await expect(page.getByTestId('chart-0')).toBeVisible();
  await expect(page.getByTestId('chart-1')).toHaveCount(0);
  await expect(page.getByLabel('Symbol')).toHaveValue('XAUUSD');
  // browser build: no desktop-only Detach button
  await expect(page.getByTestId('chart-detach-0')).toHaveCount(0);
  await expect(page).toHaveTitle(/XAUUSD/);
});
