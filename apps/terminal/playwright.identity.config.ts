import { defineConfig, devices } from '@playwright/test';

/**
 * Identity login flows. The terminal is built with VITE_IDENTITY_URL pointing at a
 * fixed local port (into dist-identity/, separate from the default build).
 *  - login-mock.spec.ts: identity HTTP API mocked with page.route (always runs).
 *  - login-live.spec.ts: real services/identity + client-gateway (JWKS URL auth),
 *    started by global-setup when FXVPS_IDENTITY_E2E_LIVE=1. Set
 *    FXVPS_IDENTITY_BIN / FXVPS_GATEWAY_BIN to prebuilt binaries to skip `cargo run`.
 */
export const PREVIEW_PORT = 4175;
export const IDENTITY_PORT = 18090;

export default defineConfig({
  testDir: './e2e-identity',
  timeout: 90_000,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  globalSetup: './e2e-identity/global-setup.ts',
  use: {
    baseURL: `http://localhost:${PREVIEW_PORT}`,
    viewport: { width: 1600, height: 960 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium-identity', use: { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 960 } } }],
  webServer: {
    command: `pnpm exec vite build --outDir dist-identity && pnpm exec vite preview --outDir dist-identity --port ${PREVIEW_PORT} --strictPort`,
    env: { VITE_IDENTITY_URL: `http://localhost:${IDENTITY_PORT}` },
    url: `http://localhost:${PREVIEW_PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
  },
});
