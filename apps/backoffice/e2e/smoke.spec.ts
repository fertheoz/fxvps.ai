import { expect, test } from "@playwright/test";

test("dashboard renders and screenshot is captured", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("page-dashboard")).toBeVisible();
  await expect(page.locator(".recharts-surface").first()).toBeVisible();
  await page.waitForTimeout(800);
  await page.screenshot({ path: "docs/screenshot.png", fullPage: false });
});

test("every page loads for admin", async ({ page }) => {
  for (const p of ["clients", "groups", "symbols", "positions", "risk", "lp", "reports", "audit", "users", "settings"]) {
    await page.goto(`/${p}/`);
    await expect(page.getByTestId(`page-${p}`)).toBeVisible();
  }
});

test("balance deposit with confirmation lands in audit log", async ({ page }) => {
  await page.goto("/clients/");
  await page.getByTestId("clients-table").locator("tbody tr", { hasText: "700100" }).click();
  // The row opens the full-page client card; balance operations live in its Access tab.
  await expect(page.getByTestId("page-client-card")).toBeVisible();
  await page.getByRole("tab", { name: "Access" }).click();
  const form = page.getByTestId("balance-form");
  await form.locator('input[name="amount"]').fill("250.50");
  await form.locator('input[name="reason"]').fill("Smoke test wire");
  await form.getByRole("button", { name: "Confirm" }).click();
  await page.getByTestId("confirm-balance").click();
  await expect(page.getByRole("status").filter({ hasText: "Operation applied" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("link", { name: "Audit Log" }).click();
  await expect(page.getByTestId("audit-table")).toContainText("Smoke test wire");
});

test("route guard blocks support from LP page; command palette + i18n work", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("role-select").selectOption("support");
  await expect(page.getByRole("link", { name: "LP Connections" })).toHaveCount(0);
  await page.goto("/lp/");
  await expect(page.getByTestId("access-denied")).toBeVisible();

  await page.getByTestId("role-select").selectOption("admin");
  await page.keyboard.press("Control+k");
  await page.getByTestId("palette-input").fill("Risk");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("page-risk")).toBeVisible();

  await page.getByRole("button", { name: "Language" }).click();
  await expect(page.getByRole("heading", { name: "Risk" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Müşteriler" })).toBeVisible();
});
