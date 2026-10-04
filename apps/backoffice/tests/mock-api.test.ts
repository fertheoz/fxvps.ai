import { describe, expect, it } from "vitest";
import { createMockApi, ForbiddenError } from "@/lib/api/mock";
import { isTradingAt } from "@/lib/sessions";

const admin = { name: "A", role: "admin" as const };
const support = { name: "S", role: "support" as const };
const dealer = { name: "D", role: "dealer" as const };
const key = () => crypto.randomUUID();

describe("mock AdminApi", () => {
  it("is deterministic for a given seed", async () => {
    const a = await createMockApi({ latencyMs: 0 }).listClients();
    const b = await createMockApi({ latencyMs: 0 }).listClients();
    expect(a).toEqual(b);
    expect(a.length).toBeGreaterThan(40);
    expect(a.some((c) => c.parentId !== null)).toBe(true);
    for (const c of a) expect(Number.isSafeInteger(c.balance)).toBe(true);
  });

  it("deposits update balance and write an audit entry", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    const r = await api.balanceOp({ clientId: c!.id, type: "deposit", amount: 12345, currency: "USD", reason: "test deposit", idempotencyKey: key() }, support);
    expect(r.status).toBe("applied");
    expect(r.newBalance).toBe(c!.balance + 12345);
    const [last] = await api.listAudit();
    expect(last).toMatchObject({ action: "balance.deposit", actor: "S", target: `#${c!.login}` });
  });

  it("is idempotent on the idempotency key", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    const k = key();
    const req = { clientId: c!.id, type: "deposit" as const, amount: 100, currency: "USD", reason: "dup check", idempotencyKey: k };
    await api.balanceOp(req, admin);
    const second = await api.balanceOp(req, admin);
    expect(second.newBalance).toBe(c!.balance + 100);
  });

  it("enforces RBAC server-side", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    await expect(api.balanceOp({ clientId: c!.id, type: "deposit", amount: 100, currency: "USD", reason: "dealer try", idempotencyKey: key() }, dealer)).rejects.toBeInstanceOf(ForbiddenError);
    await expect(api.balanceOp({ clientId: c!.id, type: "credit", amount: 100, currency: "USD", reason: "support credit", idempotencyKey: key() }, support)).rejects.toBeInstanceOf(ForbiddenError);
    const [p] = await api.listPositions();
    await expect(api.forceClose([p!.id], support)).rejects.toBeInstanceOf(ForbiddenError);
  });

  it("large ops by a non-approver go to 4-eyes and do not change balance", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    const r = await api.balanceOp({ clientId: c!.id, type: "deposit", amount: 5_000_000, currency: "USD", reason: "big wire", idempotencyKey: key() }, support);
    expect(r.status).toBe("pending_approval");
    expect(r.newBalance).toBe(c!.balance);
  });

  it("rejects withdrawals above free margin", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    await expect(api.balanceOp({ clientId: c!.id, type: "withdraw", amount: c!.equity + 100, currency: "USD", reason: "too much", idempotencyKey: key() }, admin)).rejects.toThrow(/free margin/);
  });

  it("force close removes positions and realises P&L", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const before = await api.listPositions();
    const target = before[0]!;
    const r = await api.forceClose([target.id], dealer);
    expect(r.closed).toBe(1);
    expect((await api.listPositions()).find((p) => p.id === target.id)).toBeUndefined();
    expect((await api.listTrades())[0]!.login).toBe(target.login);
  });

  it("risk lists contain stressed accounts sorted by margin level", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const rows = await api.marginCalls();
    expect(rows.length).toBeGreaterThan(0);
    for (let i = 1; i < rows.length; i++) expect(rows[i]!.marginLevel).toBeGreaterThanOrEqual(rows[i - 1]!.marginLevel);
  });

  it("LP reconnect logs on a disconnected session", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const down = (await api.listFixSessions()).find((f) => f.status === "disconnected")!;
    const up = await api.reconnect(down.id, dealer);
    expect(up.status).toBe("logged_on");
    expect(up.outSeq).toBe(down.outSeq + 1);
  });
});

describe("trading sessions", () => {
  const fx = (["mon", "tue", "wed", "thu", "fri"] as const).map((day) => ({ day, open: "00:00", close: "24:00" }));
  it("open on weekdays, closed on weekends (UTC)", () => {
    expect(isTradingAt(fx, new Date("2026-10-05T10:00:00Z"))).toBe(true); // Monday
    expect(isTradingAt(fx, new Date("2026-10-04T10:00:00Z"))).toBe(false); // Sunday
    expect(isTradingAt([{ day: "mon", open: "01:05", close: "23:55" }], new Date("2026-10-05T00:30:00Z"))).toBe(false);
  });
});
