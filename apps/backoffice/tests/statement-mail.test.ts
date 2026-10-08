import { describe, expect, it, vi } from "vitest";
import { createHttpApi } from "@/lib/api/http";
import { createMockApi, ForbiddenError } from "@/lib/api/mock";
import { sparkPath } from "@/components/charts";

const admin = { name: "A", role: "admin" as const };
const support = { name: "S", role: "support" as const };

describe("monthly statement e-mail (console)", () => {
  it("maps status and test send to the core routes", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchImpl = vi.fn(async (url: RequestInfo | URL, init: RequestInit = {}) => {
      calls.push({ url: String(url), init });
      return new Response(JSON.stringify({ ok: true }), { status: 200, headers: { "content-type": "application/json" } });
    }) as unknown as typeof fetch;
    const api = createHttpApi("http://x", () => "t", { fetchImpl });
    await api.getStatementMail();
    await api.sendTestStatement(null, admin);
    await api.sendTestStatement(100001, admin);
    expect(calls.map((c) => `${c.init.method} ${c.url.replace("http://x", "")}`)).toEqual([
      "GET /v1/settings/statement-email",
      "POST /v1/settings/statement-email/test",
      "POST /v1/settings/statement-email/test",
    ]);
    expect(JSON.parse(calls[1]!.init.body as string)).toEqual({});
    expect(JSON.parse(calls[2]!.init.body as string)).toEqual({ account: 100001 });
  });

  it("demo API: off, no channel, last month; test send is guarded", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const s = await api.getStatementMail();
    expect(s).toMatchObject({ enabled: false, configured: false, runDays: 3, run: null });
    expect(s.month).toMatch(/^\d{4}-\d{2}$/);
    await expect(api.sendTestStatement(null, support)).rejects.toBeInstanceOf(ForbiddenError);
    await expect(api.sendTestStatement(null, admin)).rejects.toThrow(/SMTP/);
  });

  it("strategy sparkline path spans the cell", () => {
    expect(sparkPath([0, 10, 5], 100, 20)).toBe("M0.0,18.0 L50.0,2.0 L100.0,10.0");
    expect(sparkPath([3, 3], 10, 20)).toBe("M0.0,10.0 L10.0,10.0");
    expect(sparkPath([], 10, 20)).toBe("");
  });
});
