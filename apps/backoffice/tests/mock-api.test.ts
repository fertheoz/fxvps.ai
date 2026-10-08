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

  it("keeps a USDT payment that arrived after the decision open until handled", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const open = async () => (await api.listFunding("open")).map((f) => f.id);
    // rejected, but hazine bound a payment to its card afterwards
    expect(await open()).toContain("fr-3");
    await expect(api.decideFunding("fr-3", "handled", undefined, admin)).rejects.toThrow(/note/);
    const f = await api.decideFunding("fr-3", "handled", "refunded to sender", admin);
    expect(f.lateHandledBy).toBe("A");
    expect(f.status).toBe("rejected");
    expect(await open()).not.toContain("fr-3");
    await expect(api.decideFunding("fr-3", "handled", "again", admin)).rejects.toThrow(/no unreviewed/);
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

  it("IB payouts move the paid-through date and cannot pay a day twice", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const [c] = await api.listClients();
    await api.setIb(c!.id, { sharePct: 30 }, admin);
    const ib = (await api.ibReport()).rows[0]!;
    expect(ib.ib).toBe(c!.login);
    const p = await api.ibPayoutPreview(ib.ib, "2026-09-30");
    expect(p.from).toBeNull();
    expect(p.amount).toBeGreaterThan(0);
    await expect(api.ibPayout(ib.ib, "2026-09-30", dealer)).rejects.toBeInstanceOf(ForbiddenError);
    const r = await api.ibPayout(ib.ib, "2026-09-30", admin);
    expect(r.op.status).toBe("applied");
    expect((await api.ibReport()).rows[0]!.paidThrough).toBe("2026-09-30");
    const again = await api.ibPayoutPreview(ib.ib, "2026-09-30");
    expect(again.amount).toBe(0);
    expect(again.from).toBe("2026-10-01");
    await expect(api.ibPayout(ib.ib, "2026-09-30", admin)).rejects.toThrow(/nothing to pay/);
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
  it("economic calendar: create, duplicate guard, idempotent import, RBAC", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const time = Date.UTC(2030, 0, 3, 13, 30);
    const ev = await api.createEconEvent({ time, currency: "usd", title: " CPI y/y ", impact: "high", forecast: "3.1%" }, admin);
    expect(ev).toMatchObject({ currency: "USD", title: "CPI y/y", impact: "high", forecast: "3.1%", actual: null, at: "2030-01-03T13:30:00.000Z" });
    await expect(api.createEconEvent({ time, currency: "USD", title: "cpi y/y", impact: "low" }, admin)).rejects.toThrow(/already exists/);
    await expect(api.createEconEvent({ time, currency: "USD", title: "x", impact: "low" }, dealer)).rejects.toBeInstanceOf(ForbiddenError);
    const listed = await api.listEconEvents("2030-01-01", "2030-01-10");
    expect(listed.events.map((e) => e.id)).toEqual([ev.id]);
    const first = await api.importEconWeek(admin);
    expect(first.added).toBeGreaterThan(0);
    const again = await api.importEconWeek(admin);
    expect(again).toMatchObject({ added: 0, unchanged: first.added + first.unchanged });
    await api.deleteEconEvent(ev.id, admin);
    expect((await api.listEconEvents("2030-01-01", "2030-01-10")).events).toEqual([]);
    const [last] = await api.listAudit();
    expect(last).toMatchObject({ action: "calendar.delete", target: ev.id });
  });
});
