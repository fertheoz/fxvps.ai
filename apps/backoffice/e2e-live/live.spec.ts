import { expect, test, type Page } from "@playwright/test";

const API = "http://127.0.0.1:8091";

async function devLogin(page: Page, role: string, name: string) {
  await page.goto("/");
  await expect(page.getByTestId("login")).toBeVisible();
  await page.locator('select[name="dev-role"]').selectOption(role);
  await page.locator('input[name="dev-name"]').fill(name);
  await page.getByTestId("dev-token").click();
  await expect(page.getByTestId("current-user")).toContainText(name);
}

/** Calls the admin API with the token the UI stored. */
async function apiCall(page: Page, method: string, path: string, body?: unknown) {
  return page.evaluate(
    async ([api, m, p, b]) => {
      const token = sessionStorage.getItem("fxvps-bo-token");
      const res = await fetch(`${api}${p}`, {
        method: m as string,
        headers: { authorization: `Bearer ${token}`, "content-type": "application/json", "idempotency-key": crypto.randomUUID() },
        body: b === undefined ? undefined : JSON.stringify(b),
      });
      return { status: res.status, body: await res.json() };
    },
    [API, method, path, body] as const,
  );
}

async function openClient(page: Page, login: string) {
  await page.goto("/clients/");
  await page.getByTestId("clients-table").locator("tbody tr", { hasText: login }).click();
  // The row opens the full-page client card; balance operations live in its Access tab.
  await expect(page.getByTestId("page-client-card")).toBeVisible();
  await page.getByRole("tab", { name: "Access" }).click();
}

async function balanceOp(page: Page, amount: string, reason: string) {
  const form = page.getByTestId("balance-form");
  await form.locator('input[name="amount"]').fill(amount);
  await form.locator('input[name="reason"]').fill(reason);
  await form.getByRole("button", { name: "Confirm" }).click();
  await page.getByTestId("confirm-balance").click();
}

test("deposit → balance updated → audit entry (admin, live core-engine)", async ({ page }) => {
  await devLogin(page, "admin", "Alice Admin");
  await expect(page.getByTestId("data-source")).toContainText("Live core-engine");
  await openClient(page, "1002");
  await expect(page.getByTestId("page-client-card")).toContainText("5,000.00");
  await balanceOp(page, "250.50", "Live e2e wire");
  await expect(page.getByRole("status").filter({ hasText: "Operation applied" })).toBeVisible();
  await expect(page.getByTestId("page-client-card")).toContainText("5,250.50");
  const acct = await apiCall(page, "GET", "/v1/accounts/1002");
  expect(acct.body.balance).toBe(525_050);

  await page.keyboard.press("Escape");
  await page.getByRole("link", { name: "Audit Log" }).click();
  const audit = page.getByTestId("audit-table");
  await expect(audit).toContainText("Live e2e wire");
  await expect(audit).toContainText("Alice Admin");
  await expect(audit).toContainText("balance.deposit");
});

test("support role is denied by the UI and by the server", async ({ page }) => {
  await devLogin(page, "support", "Sam Support");
  await expect(page.getByRole("link", { name: "Groups" })).toHaveCount(0);
  await page.goto("/groups/");
  await expect(page.getByTestId("access-denied")).toBeVisible();

  const close = await apiCall(page, "POST", "/v1/positions/force-close", { positionIds: ["1"] });
  expect(close.status).toBe(403);
  expect(close.body.error).toMatchObject({ code: "forbidden", permission: "positions.forceClose" });
  const credit = await apiCall(page, "POST", "/v1/accounts/1001/balance-ops", {
    type: "credit", amount: 100, currency: "USD", reason: "support tries credit",
  });
  expect(credit.status).toBe(403);
  expect(credit.body.error.permission).toBe("balance.credit");
});

test("4-eyes: a large deposit waits for a different approver", async ({ page }) => {
  await devLogin(page, "support", "Sam Support");
  await openClient(page, "1003");
  await balanceOp(page, "20000", "Large wire needs approval");
  await expect(page.getByRole("status").filter({ hasText: "Sent for second approval" })).toBeVisible();
  await expect(page.getByTestId("page-client-card")).toContainText("12,000.00");

  await page.keyboard.press("Escape");
  await page.getByTestId("logout").click();
  await devLogin(page, "risk", "Rita Risk");
  await page.getByRole("link", { name: "Approvals" }).click();
  const list = page.getByTestId("approvals-list");
  await expect(list).toContainText("Sam Support");
  await list.getByTestId("approve").first().click();
  await expect(page.getByRole("status").filter({ hasText: "Approved and applied" })).toBeVisible();
  await expect(page.getByTestId("approvals-empty")).toBeVisible();
  await openClient(page, "1003");
  await expect(page.getByTestId("page-client-card")).toContainText("32,000.00");
});
