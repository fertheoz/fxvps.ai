import { describe, expect, it } from "vitest";
import { BalanceOpRequest, Group, SymbolSpec } from "@/lib/schemas";
import { seedGroups, seedSymbols } from "@/lib/api/seed";

const base = { clientId: "c1", type: "deposit", amount: 10000, currency: "USD", reason: "wire 123 received", idempotencyKey: "6f1c1b7e-4a5e-4c4b-9e7c-1f2d3c4b5a69" };

describe("zod schemas", () => {
  it("accepts a valid balance op", () => {
    expect(BalanceOpRequest.parse(base)).toMatchObject({ amount: 10000 });
  });
  it("rejects float, zero, negative and unsafe amounts", () => {
    for (const amount of [100.5, 0, -1, Number.MAX_SAFE_INTEGER + 2]) {
      expect(BalanceOpRequest.safeParse({ ...base, amount }).success).toBe(false);
    }
  });
  it("requires a reason, a known currency and a uuid idempotency key", () => {
    expect(BalanceOpRequest.safeParse({ ...base, reason: "  " }).success).toBe(false);
    expect(BalanceOpRequest.safeParse({ ...base, currency: "XYZ" }).success).toBe(false);
    expect(BalanceOpRequest.safeParse({ ...base, idempotencyKey: "abc" }).success).toBe(false);
    expect(BalanceOpRequest.safeParse({ ...base, type: "steal" }).success).toBe(false);
  });
  it("seed data conforms to schemas", () => {
    for (const g of seedGroups()) expect(Group.safeParse(g).success).toBe(true);
    for (const s of seedSymbols()) expect(SymbolSpec.safeParse(s).success).toBe(true);
  });
  it("group: stop-out must be below margin call; leverage bounds", () => {
    const g = seedGroups()[0]!;
    const r = Group.safeParse({ ...g, stopOutPct: 120, marginCallPct: 100 });
    expect(r.success).toBe(false);
    expect(r.error?.issues[0]?.path).toEqual(["stopOutPct"]);
    expect(Group.safeParse({ ...g, leverage: 0 }).success).toBe(false);
    expect(Group.safeParse({ ...g, leverage: 30.5 }).success).toBe(false);
  });
  it("symbol: minLot <= maxLot and HH:MM sessions", () => {
    const s = seedSymbols()[0]!;
    expect(SymbolSpec.safeParse({ ...s, minLot: 200 }).success).toBe(false);
    expect(SymbolSpec.safeParse({ ...s, tradeSessions: [{ day: "mon", open: "25:00", close: "24:00" }] }).success).toBe(false);
  });
});
