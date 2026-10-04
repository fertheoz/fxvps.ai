import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

/**
 * Real services/identity + client-gateway (--demo, JWKS URL auth):
 * register → verify (emailed link) → account linked by the service API → login →
 * connect → trade → enable TOTP → sign in with the second factor → add a passkey
 * (virtual authenticator) → sign in with the passkey.
 */
const ID = 'http://localhost:18090';
const live = process.env.FXVPS_IDENTITY_LIVE_READY === '1';

function mailLink(to: string, param: string): string {
  const lines = readFileSync(process.env.FXVPS_IDENTITY_MAIL_FILE!, 'utf8').trim().split('\n');
  const m = lines
    .map((l) => JSON.parse(l) as { to: string; link: string | null })
    .reverse()
    .find((x) => x.to === to && x.link?.includes(`${param}=`));
  if (!m?.link) throw new Error(`no ${param} mail for ${to}`);
  return m.link;
}

function base32Decode(s: string): Buffer {
  const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  let bits = '';
  for (const c of s.replace(/=+$/, '').toUpperCase()) bits += A.indexOf(c).toString(2).padStart(5, '0');
  const out: number[] = [];
  for (let i = 0; i + 8 <= bits.length; i += 8) out.push(parseInt(bits.slice(i, i + 8), 2));
  return Buffer.from(out);
}

/** RFC 6238 TOTP (SHA1, 6 digits, 30 s) at step offset `off`. */
function totp(secret: string, off = 0): string {
  const step = Math.floor(Date.now() / 1000 / 30) + off;
  const msg = Buffer.alloc(8);
  msg.writeBigUInt64BE(BigInt(step));
  const h = createHmac('sha1', base32Decode(secret)).update(msg).digest();
  const o = h[h.length - 1]! & 0xf;
  const n = ((h.readUInt32BE(o) & 0x7fffffff) % 1_000_000).toString();
  return n.padStart(6, '0');
}

async function login(page: Page, email: string, password: string) {
  await page.getByTestId('login-email').fill(email);
  await page.getByTestId('login-password').fill(password);
  await page.getByTestId('login-submit').click();
}

test('register → verify → login → connect → trade → 2FA → passkey', async ({ page, request }, testInfo) => {
  test.skip(!live, 'set FXVPS_IDENTITY_E2E_LIVE=1 to run against real services');
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const ws = process.env.FXVPS_WS_URL!;
  const email = `trader-${Date.now()}@example.com`;
  const password = 'correct horse battery staple';

  await page.goto(`/?api=ws&url=${encodeURIComponent(ws)}`);
  await page.getByTestId('register-link').click();
  await page.getByTestId('register-email').fill(email);
  await page.getByTestId('register-password').fill(password);
  await page.getByTestId('register-submit').click();
  await expect(page.getByTestId('auth-info')).toBeVisible();

  // Emailed verification link (dev mail file). The gateway choice is in sessionStorage.
  const link = new URL(mailLink(email, 'verify_email'));
  await page.goto(`/${link.search}`);
  await expect(page.getByTestId('auth-info')).toContainText('Email verified');
  expect(page.url()).not.toContain('verify_email');

  // Back office / core-engine links the trading account through the service API.
  const r = await request.post(`${ID}/v1/admin/accounts/link`, {
    headers: { Authorization: `Bearer ${process.env.FXVPS_IDENTITY_SERVICE_TOKEN}` },
    // A fresh demo account per attempt: a retry must not re-link one owned by the
    // previous attempt's user (409 = owned by another user).
    data: { email, account_id: `DEMO-${testInfo.retry + 1}` },
  });
  expect(r.status()).toBe(200);

  await login(page, email, password);
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 30_000 });
  await expect(page.getByTestId('user-menu')).toHaveText(email);
  await expect(page.getByTestId('bid-EURUSD')).toHaveText(/^\d\.\d{5}$/, { timeout: 20_000 });

  await page.getByTestId('ticket-volume').fill('0.25');
  await page.getByTestId('ticket-submit').click();
  await expect(page.getByTestId('toast').first()).toContainText('Filled BUY 0.25 EURUSD', { timeout: 15_000 });
  await expect(page.getByTestId('positions-table').getByRole('row').filter({ hasText: 'EURUSD' })).toHaveCount(1);

  // Reload: silent refresh from the HttpOnly cookie, no login form.
  await page.reload();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 30_000 });
  await expect(page.getByTestId('auth-screen')).toHaveCount(0);

  // Enable TOTP from the security dialog.
  await page.getByTestId('user-menu').click();
  await page.getByTestId('totp-enable').click();
  const secret = (await page.getByTestId('totp-secret').textContent())!.trim();
  await page.getByTestId('totp-code').fill(totp(secret));
  await page.getByTestId('totp-confirm').click();
  await expect(page.getByTestId('recovery-codes')).toBeVisible();
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click();

  // Sign out → sign in again with the second factor.
  await page.getByTestId('sign-out').click();
  await expect(page.getByTestId('login-form')).toBeVisible();
  await login(page, email, password);
  await expect(page.getByTestId('mfa-form')).toBeVisible();
  // The enrolment code's step is spent (replay protection): use the next one.
  await page.getByTestId('mfa-code').fill(totp(secret, 1));
  await page.getByTestId('mfa-submit').click();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 30_000 });

  // Passkey: register with a CDP virtual authenticator, then sign in with it.
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true },
  });
  await page.getByTestId('user-menu').click();
  await page.getByTestId('passkey-add').click();
  await expect(page.getByTestId('security-dialog')).toContainText('Passkey added');
  await page.getByRole('dialog').getByRole('button', { name: 'Close', exact: true }).click();
  await page.getByTestId('sign-out').click();
  await expect(page.getByTestId('login-form')).toBeVisible();
  await page.getByTestId('login-email').fill(email);
  await page.getByTestId('passkey-login').click();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected', { timeout: 30_000 });
  expect(errors).toEqual([]);
});
