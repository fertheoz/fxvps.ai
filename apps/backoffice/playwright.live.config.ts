import { defineConfig } from "@playwright/test";

/** Back office (static export with NEXT_PUBLIC_API_URL) against a real core-engine. */
const port = 4311;
const core = "127.0.0.1:8091";

export default defineConfig({
  testDir: "./e2e-live",
  timeout: 60_000,
  workers: 1,
  fullyParallel: false,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1440, height: 900 },
    launchOptions: process.env.PW_CHROMIUM_PATH ? { executablePath: process.env.PW_CHROMIUM_PATH } : {},
  },
  webServer: [
    {
      command: `node e2e-live/start-core.mjs ${core} http://127.0.0.1:${port}`,
      url: `http://${core}/health`,
      timeout: 600_000,
      reuseExistingServer: false,
      stdout: "pipe",
    },
    {
      command: `node e2e/serve.mjs ${port} out-live`,
      port,
      reuseExistingServer: false,
    },
  ],
});
