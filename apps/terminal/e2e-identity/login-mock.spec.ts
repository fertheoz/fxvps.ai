import { expect, test, type Route } from '@playwright/test';

const ID = 'http://localhost:18090';
const WS = 'ws://127.0.0.1:9/ws';

function b64url(s: string): string {
  return Buffer.from(s).toString('base64url');
}

function token(accounts: string[], amr: string[]) {
  const now = Math.floor(Date.now() / 1000);
  const payload = { sub: 'u1', email: 'trader@example.com', exp: now + 300, iat: now, accounts, roles: ['client'], amr, iss: ID, aud: 'fxvps' };
  return {
    access_token: `${b64url('{"alg":"RS256","kid":"k"}')}.${b64url(JSON.stringify(payload))}.sig`,
    token_type: 'Bearer',
    expires_in: 300,
    refresh_expires_in: 3600,
    accounts,
    roles: ['client'],
  };
}

const json = (route: Route, status: number, body?: unknown) =>
  route.fulfill({
    status,
    contentType: 'application/json',
    headers: { 'access-control-allow-origin': 'http://localhost:4175', 'access-control-allow-credentials': 'true' },
    body: body === undefined ? '' : JSON.stringify(body),
  });

test('login with 2FA, account picker, then the terminal connects with the identity token', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const seen: string[] = [];
  let loggedIn = false;
  await page.route(`${ID}/**`, async (route) => {
    const req = route.request();
    if (req.method() === 'OPTIONS')
      return route.fulfill({
        status: 204,
        headers: {
          'access-control-allow-origin': 'http://localhost:4175',
          'access-control-allow-credentials': 'true',
          'access-control-allow-headers': 'content-type,x-fxvps-csrf,authorization',
          'access-control-allow-methods': 'GET,POST',
        },
      });
    const path = new URL(req.url()).pathname;
    seen.push(path);
    switch (path) {
      case '/v1/token/refresh':
        return loggedIn ? json(route, 200, token(['DEMO-1', 'DEMO-2'], ['pwd', 'mfa', 'otp'])) : json(route, 401, { error: 'unauthenticated', message: 'no session' });
      case '/v1/login': {
        const b = req.postDataJSON() as { password: string; session: string };
        expect(b.session).toBe('cookie');
        if (b.password !== 'correct horse battery') return json(route, 401, { error: 'invalid_credentials', message: 'x' });
        return json(route, 200, { mfa_required: true, mfa_token: 'mfa-1', methods: ['totp', 'recovery_code'] });
      }
      case '/v1/login/2fa': {
        const b = req.postDataJSON() as { code?: string };
        if (b.code !== '123456') return json(route, 401, { error: 'invalid_code', message: 'x' });
        loggedIn = true;
        return json(route, 200, token(['DEMO-1', 'DEMO-2'], ['pwd', 'mfa', 'otp']));
      }
      case '/v1/token/revoke':
        loggedIn = false;
        return json(route, 204);
      default:
        return json(route, 404, { error: 'not_found', message: path });
    }
  });
  let wsOpened = false;
  await page.routeWebSocket(WS, () => {
    wsOpened = true;
  });

  await page.goto(`/?api=ws&url=${encodeURIComponent(WS)}`);
  await expect(page.getByTestId('login-form')).toBeVisible();

  await page.getByTestId('login-email').fill('trader@example.com');
  await page.getByTestId('login-password').fill('wrong password');
  await page.getByTestId('login-submit').click();
  await expect(page.getByTestId('auth-error')).toContainText('Wrong email or password');

  await page.getByTestId('login-password').fill('correct horse battery');
  await page.getByTestId('login-submit').click();
  await expect(page.getByTestId('mfa-form')).toBeVisible();
  await page.getByTestId('mfa-code').fill('000000');
  await page.getByTestId('mfa-submit').click();
  await expect(page.getByTestId('auth-error')).toContainText('Invalid code');
  await page.getByTestId('mfa-code').fill('123456');
  await page.getByTestId('mfa-submit').click();

  // Two accounts in the token → picker.
  await expect(page.getByTestId('account-picker')).toBeVisible();
  await page.getByTestId('account-option-DEMO-2').check();
  await page.getByTestId('account-continue').click();
  await expect(page.getByTestId('auth-screen')).toHaveCount(0);
  await expect.poll(() => wsOpened).toBe(true);

  // Reload: silent sign-in from the (mocked) refresh cookie, choice remembered.
  await page.reload();
  await expect.poll(() => seen.filter((p) => p === '/v1/token/refresh').length).toBeGreaterThanOrEqual(2);
  await expect(page.getByTestId('auth-screen')).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('registration, and the demo simulator stays available without login', async ({ page }) => {
  await page.route(`${ID}/**`, (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/v1/register') return json(route, 202, { status: 'verification_sent' });
    return json(route, 401, { error: 'unauthenticated', message: 'x' });
  });
  await page.goto(`/?api=ws&url=${encodeURIComponent(WS)}`);
  await page.getByTestId('register-link').click();
  await page.getByTestId('register-email').fill('new@example.com');
  await page.getByTestId('register-password').fill('a long enough password');
  await page.getByTestId('register-submit').click();
  await expect(page.getByTestId('auth-info')).toContainText('verify your email');

  // Demo simulator: no login.
  await page.getByTestId('auth-use-demo').click();
  await expect(page.getByTestId('connection')).toHaveAttribute('data-state', 'connected');
  await expect(page.getByTestId('gateway-button')).toHaveAttribute('data-api', 'mock');
  await expect(page.getByTestId('user-menu')).toHaveCount(0);
});
