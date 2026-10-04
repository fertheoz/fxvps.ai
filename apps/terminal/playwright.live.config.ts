import { defineConfig, devices } from '@playwright/test';

/**
 * End-to-end against a real services/client-gateway in --demo mode (lp-simulator +
 * fix-gateway in-process). global-setup starts it on an ephemeral port and exports
 * FXVPS_WS_URL / FXVPS_DEMO_TOKEN to the tests. Set FXVPS_GATEWAY_BIN to a prebuilt
 * binary to skip `cargo run`.
 */
const port = 4174;

export default defineConfig({
  testDir: './e2e-live',
  timeout: 60_000,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  globalSetup: './e2e-live/global-setup.ts',
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: { width: 1600, height: 960 },
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium-live', use: { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 960 } } }],
  webServer: {
    command: `pnpm exec vite preview --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
