/**
 * Captures docs/screenshot.png. Run with: pnpm build && pnpm screenshot
 * (uses the same preview server config as the e2e tests).
 */
import { chromium } from '@playwright/test';

const url = process.argv[2] ?? 'http://localhost:4173';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 960 }, deviceScaleFactor: 1 });
await page.goto(url);
await page.getByTestId('connection').and(page.locator('[data-state="connected"]')).waitFor();
await page.getByTestId('ticket-volume').fill('0.50');
await page.getByTestId('ticket-submit').click();
await page.getByTestId('oneclick-sell-0').click();
await page.locator('button', { hasText: 'RSI 14' }).click();
await page.locator('button', { hasText: 'BB 20,2' }).click();
await page.waitForTimeout(2500);
await page.screenshot({ path: 'docs/screenshot.png' });
await browser.close();
