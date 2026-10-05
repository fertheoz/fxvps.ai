import { describe, expect, it, vi } from "vitest";
import { ApiError, createHttpApi } from "@/lib/api/http";
import { createMockApi } from "@/lib/api/mock";
import { decodeToken, fetchDevToken } from "@/lib/auth";

type Call = { url: string; init: RequestInit };

function fakeFetch(responder: (url: string, init: RequestInit) => { status?: number; body?: unknown }) {
  const calls: Call[] = [];
  const fn = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input);
    calls.push({ url, init });
    const r = responder(url, init);
    const status = r.status ?? 200;
    return new Response(status === 204 ? null : JSON.stringify(r.body ?? {}), { status, headers: { "content-type": "application/json" } });
  });
  return { fn: fn as unknown as typeof fetch, calls };
}

const headers = (c: Call) => c.init.headers as Record<string, string>;
const actor = { name: "Sam Support", role: "support" as const };

describe("HTTP AdminApi adapter", () => {
  it("sends bearer token, JSON body and Idempotency-Key for balance ops", async () => {
    const { fn, calls } = fakeFetch(() => ({ body: { id: "op-1", status: "applied", newBalance: 100, newCredit: 0 } }));
    const api = createHttpApi("http://core:8090/", () => "tok-123", { fetchImpl: fn });
    const req = { clientId: "1001", type: "deposit" as const, amount: 12345, currency: "USD", reason: "wire in", idempotencyKey: "0f8a4c1e-1b7a-4c33-9b8e-111111111111" };
    const r = await api.balanceOp(req, actor);
    expect(r.status).toBe("applied");
    expect(calls[0]!.url).toBe("http://core:8090/v1/accounts/1001/balance-ops");
    expect(calls[0]!.init.method).toBe("POST");
    expect(headers(calls[0]!)).toMatchObject({ authorization: "Bearer tok-123", "content-type": "application/json", "idempotency-key": req.idempotencyKey, "x-actor-hint": "Sam%20Support" });
    expect(JSON.parse(calls[0]!.init.body as string)).toEqual(req);
  });

  it("maps every method to the documented route", async () => {
    const { fn, calls } = fakeFetch(() => ({ body: [] }));
    const api = createHttpApi("http://x", () => null, { fetchImpl: fn });
    await api.dashboard();
    await api.exposure();
    await api.listClients({ search: "a b" });
    await api.listPositions();
    await api.listOrders();
    await api.forceClose(["5", "6"], actor);
    await api.marginCalls();
    await api.esmaPresets();
    await api.applyPreset("esma-fx-major", "retail/a", actor);
    await api.setKyc("7", "approved", actor);
    await api.listApprovals();
    await api.listApprovals("all");
    await api.approve("op-3", actor);
    await api.reject("op-4", "no docs", actor);
    await api.listAudit();
    await api.statements();
    await api.getSettings();
    const routes = calls.map((c) => `${c.init.method} ${c.url.replace("http://x", "")}`);
    expect(routes).toEqual([
      "GET /v1/dashboard",
      "GET /v1/exposure",
      "GET /v1/accounts?search=a%20b",
      "GET /v1/positions",
      "GET /v1/orders",
      "POST /v1/positions/force-close",
      "GET /v1/risk/margin-calls",
      "GET /v1/risk/presets",
      "POST /v1/groups/retail%2Fa/apply-preset",
      "PATCH /v1/accounts/7/kyc",
      "GET /v1/approvals?status=pending_approval",
      "GET /v1/approvals?status=all",
      "POST /v1/approvals/op-3/approve",
      "POST /v1/approvals/op-4/reject",
      "GET /v1/audit",
      "GET /v1/reports/statements",
      "GET /v1/settings",
    ]);
    expect(JSON.parse(calls[5]!.init.body as string)).toEqual({ positionIds: ["5", "6"] });
    expect(JSON.parse(calls[13]!.init.body as string)).toEqual({ reason: "no docs" });
    // no token: no authorization header
    expect(headers(calls[0]!).authorization).toBeUndefined();
  });

  it("turns error bodies into ApiError (403 carries the permission)", async () => {
    const { fn } = fakeFetch(() => ({ status: 403, body: { error: { code: "forbidden", message: "missing permission balance.credit", permission: "balance.credit" } } }));
    const api = createHttpApi("http://x", () => "t", { fetchImpl: fn });
    const err = await api.balanceOp({ clientId: "1", type: "credit", amount: 1, currency: "USD", reason: "bonus credit", idempotencyKey: "k" }, actor).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 403, code: "forbidden", permission: "balance.credit", message: "missing permission balance.credit" });
  });

  it("returns null for unknown accounts and signals 401", async () => {
    const onUnauthorized = vi.fn();
    let status = 404;
    const { fn } = fakeFetch(() => ({ status, body: { error: { code: status === 404 ? "not_found" : "unauthorized", message: "x" } } }));
    const api = createHttpApi("http://x", () => "t", { fetchImpl: fn, onUnauthorized });
    expect(await api.getClient("99")).toBeNull();
    status = 401;
    await expect(api.listClients()).rejects.toMatchObject({ status: 401, code: "unauthorized" });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it("subscribes to the SSE stream with the token and forwards topics", async () => {
    class FakeES {
      static last: FakeES;
      listeners: Record<string, (e: MessageEvent) => void> = {};
      onerror: (() => void) | null = null;
      closed = false;
      constructor(public url: string) {
        FakeES.last = this;
      }
      addEventListener(name: string, fn: (e: MessageEvent) => void) {
        this.listeners[name] = fn;
      }
      close() {
        this.closed = true;
      }
    }
    const calls: { url: string; auth: string | null; method: string }[] = [];
    const fetchImpl = (async (url: string, init: RequestInit) => {
      calls.push({ url, auth: new Headers(init.headers).get("authorization"), method: init.method ?? "GET" });
      return new Response(JSON.stringify({ ticket: "t1 x", expiresInMs: 30000 }), { status: 200 });
    }) as unknown as typeof fetch;
    const api = createHttpApi("http://x", () => "a.b c", { eventSource: FakeES as unknown as typeof EventSource, fetchImpl });
    const topics: string[][] = [];
    const status: boolean[] = [];
    const stop = api.subscribe!((t) => topics.push(t), (s) => status.push(s));
    await vi.waitFor(() => expect(FakeES.last).toBeDefined());
    const es = FakeES.last;
    // the bearer token goes in a header to the ticket endpoint, never in the URL
    expect(calls).toEqual([{ url: "http://x/v1/stream/ticket", auth: "Bearer a.b c", method: "POST" }]);
    expect(es.url).toBe("http://x/v1/stream?ticket=t1%20x");
    expect(es.url).not.toContain("a.b");
    es.listeners.hello!(new MessageEvent("hello", { data: "{}" }));
    es.listeners.invalidate!(new MessageEvent("invalidate", { data: JSON.stringify({ topics: ["listClients", "listAudit"] }) }));
    expect(topics).toEqual([["listClients", "listAudit"]]);
    stop();
    expect(es.closed).toBe(true);
    expect(status).toEqual([true, false]);
  });
});

describe("auth helpers", () => {
  const b64 = (o: object) => Buffer.from(JSON.stringify(o), "utf8").toString("base64url");
  const jwt = (claims: object) => `${b64({ alg: "HS256" })}.${b64(claims)}.sig`;

  it("decodes role and name from a JWT, rejects expired/malformed", () => {
    expect(decodeToken(jwt({ sub: "u1", name: "Rita Ö", role: "risk", exp: 2_000_000_000 }), 1_000)).toEqual({ sub: "u1", name: "Rita Ö", role: "risk", exp: 2_000_000_000 });
    expect(decodeToken(jwt({ sub: "u1", role: "risk", exp: 10 }), 1_000)).toBeNull();
    expect(decodeToken(jwt({ sub: "u1", role: "root", exp: 2_000_000_000 }), 1_000)).toBeNull();
    expect(decodeToken("nope")).toBeNull();
  });

  it("fetches a dev token", async () => {
    const { fn, calls } = fakeFetch(() => ({ body: { token: "dev.tok.en" } }));
    expect(await fetchDevToken("http://x", "support", "Sam", fn)).toBe("dev.tok.en");
    expect(calls[0]!.url).toBe("http://x/auth/dev-token");
    expect(JSON.parse(calls[0]!.init.body as string)).toMatchObject({ role: "support", name: "Sam" });
    const off = fakeFetch(() => ({ status: 404 }));
    await expect(fetchDevToken("http://x", "admin", undefined, off.fn)).rejects.toThrow(/CORE_DEV_AUTH/);
  });
});

describe("mock 4-eyes approvals", () => {
  const key = () => crypto.randomUUID();
  it("queues large ops for everyone and needs a different approver", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    const admin = { name: "Admin", role: "admin" as const };
    const risk = { name: "Rita", role: "risk" as const };
    const support = { name: "Sam", role: "support" as const };
    const r = await api.balanceOp({ clientId: c!.id, type: "deposit", amount: 2_000_000, currency: "USD", reason: "big wire in", idempotencyKey: key() }, admin);
    expect(r.status).toBe("pending_approval");
    const [pending] = await api.listApprovals();
    expect(pending).toMatchObject({ id: r.id, requestedBy: "Admin", status: "pending_approval" });
    await expect(api.approve(r.id, admin)).rejects.toThrow(/different user/);
    await expect(api.approve(r.id, support)).rejects.toThrow(/balance.approve/);
    const done = await api.approve(r.id, risk);
    expect(done.status).toBe("applied");
    expect((await api.getClient(c!.id))!.balance).toBe(c!.balance + 2_000_000);
    expect(await api.listApprovals()).toEqual([]);
    expect((await api.listAudit())[0]).toMatchObject({ action: "balance.deposit.approved", actor: "Rita" });
    const r2 = await api.balanceOp({ clientId: c!.id, type: "credit", amount: 3_000_000, currency: "USD", reason: "promo credit", idempotencyKey: key() }, admin);
    expect((await api.reject(r2.id, "not eligible", risk)).status).toBe("rejected");
    expect((await api.listApprovals("all")).length).toBe(2);
  });
});

describe("same-origin console build (NEXT_PUBLIC_API_URL=/)", () => {
  it("dev token and API calls stay on the page origin", async () => {
    const urls: string[] = [];
    const fetchImpl = (async (u: RequestInfo | URL) => {
      urls.push(String(u));
      return new Response(JSON.stringify({ token: "t", items: [] }), { status: 200, headers: { "content-type": "application/json" } });
    }) as typeof fetch;
    await fetchDevToken("/", "admin", undefined, fetchImpl).catch(() => undefined);
    await createHttpApi("/", () => "t", { fetchImpl }).getLpConfig().catch(() => undefined);
    expect(urls).toEqual(["/auth/dev-token", "/v1/lp/config"]);
  });
});

describe("Cloudflare Access login", () => {
  it("posts to the same-origin access-token endpoint", async () => {
    const { fetchAccessToken } = await import("@/lib/auth");
    const urls: string[] = [];
    const fetchImpl = (async (u: RequestInfo | URL) => {
      urls.push(String(u));
      return new Response(JSON.stringify({ token: "tok" }), { status: 200 });
    }) as typeof fetch;
    expect(await fetchAccessToken("/", fetchImpl)).toBe("tok");
    expect(urls).toEqual(["/auth/access-token"]);
  });
});

describe("identity login (console)", () => {
  it("accepts identity tokens with a roles array", async () => {
    const { decodeToken } = await import("@/lib/auth");
    const b64 = (o: object) => btoa(JSON.stringify(o)).replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
    const tok = `x.${b64({ sub: "u1", email: "boss@x.io", roles: ["client", "admin"], exp: 9e9 })}.s`;
    expect(decodeToken(tok)).toMatchObject({ sub: "u1", name: "boss@x.io", role: "admin" });
    const client = `x.${b64({ sub: "u2", roles: ["client"], exp: 9e9 })}.s`;
    expect(decodeToken(client)).toBeNull();
  });
});
