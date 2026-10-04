import { defineConfig } from "@playwright/test";

const port = 4310;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1440, height: 900 },
    launchOptions: process.env.PW_CHROMIUM_PATH ? { executablePath: process.env.PW_CHROMIUM_PATH } : {},
  },
  webServer: {
    command: `node e2e/serve.mjs ${port}`,
    port,
    reuseExistingServer: !process.env.CI,
  },
});
