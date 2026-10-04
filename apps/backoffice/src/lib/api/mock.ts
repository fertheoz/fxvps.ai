import { BalanceOpRequest, Group as GroupSchema, Settings as SettingsSchema, SymbolSpec as SymbolSchema, AdminUser as UserSchema } from "../schemas";
import type { AdminUser, AuditEntry, Client, Group, Settings, SymbolSpec } from "../schemas";
import { balancePermission, can } from "../rbac";
import { formatMoney } from "../money";
import type { Actor, AdminApi, BalanceOpResult, DashboardStats, MarginCallRow, Statement, SymbolExposure } from "./types";
import { mulberry32, notionalMinor, positionPnlMinor, seed, SEED_NOW, type SeedData } from "./seed";

export class ForbiddenError extends Error {
  constructor(public permission: string) {
    super(`Forbidden: missing permission ${permission}`);
  }
}

const clone = <T,>(v: T): T => structuredClone(v);

/** In-browser fake backend. State lives in memory; seeded deterministically. */
export function createMockApi(opts: { seed?: number; latencyMs?: number } = {}): AdminApi & { _state: SeedData } {
  const s = seed(opts.seed ?? 42);
  const latency = opts.latencyMs ?? 120;
  const rnd = mulberry32(7);
  let seq = 1000;
  const delay = <T,>(v: T): Promise<T> =>
    latency > 0 ? new Promise((res) => setTimeout(() => res(clone(v)), latency * (0.5 + rnd()))) : Promise.resolve(clone(v));

  const guard = (actor: Actor, perm: Parameters<typeof can>[1]) => {
    if (!can(actor.role, perm)) throw new ForbiddenError(perm);
  };
  const audit = (actor: Actor, action: string, target: string, details: string) => {
    const e: AuditEntry = { id: `a${++seq}`, at: new Date().toISOString(), actor: actor.name, role: actor.role, action, target, details };
    s.audit.unshift(e);
  };
  const contract = (sym: string) => s.symbols.find((x) => x.name === sym)?.contractSize ?? 1;

  const recompute = (c: Client) => {
    const ps = s.positions.filter((p) => p.clientId === c.id);
    c.margin = ps.reduce((a, p) => a + Math.round(notionalMinor(p.symbol, p.lots, p.currentPrice, contract(p.symbol)) / c.leverage), 0);
    c.equity = c.balance + c.credit + ps.reduce((a, p) => a + p.pnl + p.swap, 0);
  };

  /** Small random walk on every positions read so the monitor feels "live". */
  const tick = () => {
    for (const p of s.positions) {
      const sym = s.symbols.find((x) => x.name === p.symbol)!;
      const drift = (rnd() - 0.5) * 0.0004 * p.currentPrice;
      p.currentPrice = +(p.currentPrice + drift).toFixed(sym.digits);
      p.pnl = positionPnlMinor(p.symbol, p.side, p.lots, p.openPrice, p.currentPrice, sym.contractSize);
    }
    for (const c of s.clients) recompute(c);
    for (const f of s.fix) {
      if (f.status === "logged_on") {
        f.inSeq += 1 + Math.floor(rnd() * (f.kind === "MD" ? 400 : 5));
        f.latencyMs = Math.max(0.3, +(f.latencyMs + (rnd() - 0.5) * 0.4).toFixed(2));
        f.lastHeartbeat = new Date().toISOString();
      }
    }
  };

  const exposure = (): SymbolExposure[] => {
    const map = new Map<string, SymbolExposure>();
    for (const p of s.positions) {
      const e = map.get(p.symbol) ?? { symbol: p.symbol, netLots: 0, notional: 0, aBookLots: 0, bBookLots: 0, lpLots: 0 };
      const signed = p.side === "buy" ? p.lots : -p.lots;
      e.netLots += signed;
      e.notional += (p.side === "buy" ? 1 : -1) * notionalMinor(p.symbol, p.lots, p.currentPrice, contract(p.symbol));
      if (p.book === "A") e.aBookLots += signed; else e.bBookLots += signed;
      map.set(p.symbol, e);
    }
    const out = [...map.values()].map((e) => ({
      ...e,
      netLots: +e.netLots.toFixed(2), aBookLots: +e.aBookLots.toFixed(2), bBookLots: +e.bBookLots.toFixed(2),
      // LP hedge mirrors the A-book except a deliberate mismatch on one symbol (demo alert).
      lpLots: +(e.symbol === "XAUUSD" ? e.aBookLots - 0.5 : e.aBookLots).toFixed(2),
    }));
    return out.sort((a, b) => Math.abs(b.notional) - Math.abs(a.notional));
  };

  const marginLevel = (c: Client) => (c.margin > 0 ? (c.equity / c.margin) * 100 : Infinity);

  const api: AdminApi & { _state: SeedData } = {
    _state: s,

    async dashboard(): Promise<DashboardStats> {
      tick();
      const day = 864e5;
      const pnlSeries = Array.from({ length: 24 }, (_, i) => {
        const r = mulberry32(i + 11);
        return { t: `${String(i).padStart(2, "0")}:00`, aBook: Math.round((r() - 0.45) * 90000), bBook: Math.round((r() - 0.4) * 350000) };
      });
      const depositSeries = Array.from({ length: 14 }, (_, i) => {
        const r = mulberry32(i + 101);
        const d = new Date(SEED_NOW - (13 - i) * day);
        return { day: d.toISOString().slice(5, 10), deposits: Math.round(r() * 9_000_000), withdrawals: Math.round(r() * 4_000_000) };
      });
      const last = depositSeries[depositSeries.length - 1]!;
      return delay({
        activeAccounts: s.clients.filter((c) => c.status === "active").length,
        totalAccounts: s.clients.length,
        depositsToday: last.deposits,
        withdrawalsToday: last.withdrawals,
        aBookPnl: pnlSeries.reduce((a, x) => a + x.aBook, 0),
        // Broker B-book P&L is the negative of client floating P&L on B-book.
        bBookPnl: -s.positions.filter((p) => p.book === "B").reduce((a, p) => a + p.pnl, 0),
        openPositions: s.positions.length,
        lpUp: s.fix.filter((f) => f.status === "logged_on").length,
        lpTotal: s.fix.length,
        pnlSeries,
        depositSeries,
      });
    },

    async exposure() { tick(); return delay(exposure()); },

    async listClients(q) {
      const term = q?.search?.trim().toLowerCase();
      const list = term
        ? s.clients.filter((c) => c.name.toLowerCase().includes(term) || c.email.includes(term) || String(c.login).includes(term))
        : s.clients;
      return delay(list);
    },
    async getClient(id) { return delay(s.clients.find((c) => c.id === id) ?? null); },

    async balanceOp(req, actor): Promise<BalanceOpResult> {
      const parsed = BalanceOpRequest.parse(req);
      guard(actor, balancePermission(parsed.type));
      const c = s.clients.find((x) => x.id === parsed.clientId);
      if (!c) throw new Error("Client not found");
      if (c.currency !== parsed.currency) throw new Error("Currency mismatch");
      const prior = s.audit.find((a) => a.details.includes(parsed.idempotencyKey));
      if (prior) return delay({ id: prior.id, status: "applied", newBalance: c.balance, newCredit: c.credit });
      const pending = Math.abs(parsed.amount) >= s.settings.fourEyesThreshold && !can(actor.role, "balance.approve");
      const amt = formatMoney(parsed.amount, parsed.currency);
      if (pending) {
        audit(actor, `balance.${parsed.type}.requested`, `#${c.login}`, `${amt} — ${parsed.reason} — awaiting 2nd approval [${parsed.idempotencyKey}]`);
        return delay({ id: `a${seq}`, status: "pending_approval", newBalance: c.balance, newCredit: c.credit });
      }
      if (parsed.type === "withdraw") {
        const free = c.equity - c.margin - c.credit;
        if (parsed.amount > free) throw new Error("Insufficient free margin for withdrawal");
        c.balance -= parsed.amount;
      } else if (parsed.type === "deposit") c.balance += parsed.amount;
      else c.credit += parsed.amount;
      recompute(c);
      audit(actor, `balance.${parsed.type}`, `#${c.login}`, `${amt} — ${parsed.reason} [${parsed.idempotencyKey}]`);
      return delay({ id: `a${seq}`, status: "applied", newBalance: c.balance, newCredit: c.credit });
    },

    async setKyc(id, kyc, actor) {
      guard(actor, "clients.edit");
      const c = s.clients.find((x) => x.id === id);
      if (!c) throw new Error("Client not found");
      audit(actor, "kyc.update", `#${c.login}`, `${c.kyc} → ${kyc}`);
      c.kyc = kyc;
      return delay(c);
    },

    async listGroups() { return delay(s.groups); },
    async saveGroup(g: Group, actor) {
      guard(actor, "groups.edit");
      const v = GroupSchema.parse(g);
      const i = s.groups.findIndex((x) => x.id === v.id);
      if (i >= 0) s.groups[i] = v; else s.groups.push(v);
      audit(actor, "group.update", v.name, `leverage 1:${v.leverage}, MC ${v.marginCallPct}%, SO ${v.stopOutPct}%, book ${v.book}`);
      return delay(v);
    },

    async listSymbols() { return delay(s.symbols); },
    async saveSymbol(sym: SymbolSpec, actor) {
      guard(actor, "symbols.edit");
      const v = SymbolSchema.parse(sym);
      const i = s.symbols.findIndex((x) => x.name === v.name);
      if (i >= 0) s.symbols[i] = v; else s.symbols.push(v);
      audit(actor, "symbol.update", v.name, `swap ${v.swapLong}/${v.swapShort}, margin ${v.marginPct}%, ${v.enabled ? "enabled" : "disabled"}`);
      return delay(v);
    },

    async listPositions() { tick(); return delay(s.positions); },
    async listOrders() { return delay(s.orders); },
    async forceClose(ids, actor) {
      guard(actor, "positions.forceClose");
      const set = new Set(ids);
      const closing = s.positions.filter((p) => set.has(p.id));
      for (const p of closing) {
        const c = s.clients.find((x) => x.id === p.clientId);
        if (c) c.balance += p.pnl + p.swap;
        s.trades.unshift({ id: `t${++seq}`, login: p.login, symbol: p.symbol, side: p.side, lots: p.lots, openPrice: p.openPrice, closePrice: p.currentPrice, pnl: p.pnl, commission: 0, swap: p.swap, book: p.book, closedAt: new Date().toISOString() });
      }
      s.positions = s.positions.filter((p) => !set.has(p.id));
      for (const c of s.clients) recompute(c);
      audit(actor, "position.forceClose", closing.map((p) => `#${p.login}`).join(", ") || "-", `Force-closed ${closing.length} position(s): ${closing.map((p) => p.id).join(", ")}`);
      return delay({ closed: closing.length });
    },

    async marginCalls(): Promise<MarginCallRow[]> {
      const rows: MarginCallRow[] = [];
      for (const c of s.clients) {
        const g = s.groups.find((x) => x.name === c.group);
        const ml = marginLevel(c);
        if (!g || !Number.isFinite(ml)) continue;
        if (ml <= g.stopOutPct) rows.push({ client: c, marginLevel: ml, state: "stop_out" });
        else if (ml <= g.marginCallPct) rows.push({ client: c, marginLevel: ml, state: "margin_call" });
      }
      return delay(rows.sort((a, b) => a.marginLevel - b.marginLevel));
    },
    async esmaPresets() { return delay(s.presets); },
    async applyPreset(presetId, groupId, actor) {
      guard(actor, "risk.edit");
      const p = s.presets.find((x) => x.id === presetId);
      const g = s.groups.find((x) => x.id === groupId);
      if (!p || !g) throw new Error("Preset or group not found");
      Object.assign(g, { leverage: p.maxLeverage, marginCallPct: p.marginCallPct, stopOutPct: p.stopOutPct });
      audit(actor, "risk.applyPreset", g.name, `${p.label}`);
      return delay(g);
    },

    async listFixSessions() { tick(); return delay(s.fix); },
    async reconnect(id, actor) {
      guard(actor, "lp.reconnect");
      const f = s.fix.find((x) => x.id === id);
      if (!f) throw new Error("Session not found");
      f.status = "logged_on";
      f.latencyMs = 1 + +(rnd() * 2).toFixed(2);
      f.outSeq += 1; // Logon (35=A)
      f.lastHeartbeat = new Date().toISOString();
      audit(actor, "lp.reconnect", `${f.lp} ${f.kind}`, `Logon sent, outSeq=${f.outSeq}`);
      return delay(f);
    },

    async listTrades() { return delay(s.trades); },
    async statements(): Promise<Statement[]> {
      const rows: Statement[] = s.clients.filter((c) => c.parentId === null).slice(0, 30).map((c) => {
        const ts = s.trades.filter((t) => t.login === c.login);
        const pnl = ts.reduce((a, t) => a + t.pnl, 0), commission = ts.reduce((a, t) => a + t.commission, 0), swap = ts.reduce((a, t) => a + t.swap, 0);
        const deposits = Math.round(c.balance * 0.1), withdrawals = Math.round(c.balance * 0.03);
        const opening = c.balance - deposits + withdrawals - pnl - commission - swap;
        return { login: c.login, name: c.name, currency: c.currency, opening, deposits, withdrawals, pnl, commission, swap, closing: c.balance };
      });
      return delay(rows);
    },

    async listAudit() { return delay(s.audit); },

    async listUsers() { return delay(s.users); },
    async saveUser(u: AdminUser, actor) {
      guard(actor, "users.edit");
      const v = UserSchema.parse(u);
      const i = s.users.findIndex((x) => x.id === v.id);
      const before = i >= 0 ? s.users[i]! : null;
      if (i >= 0) s.users[i] = v; else s.users.push(v);
      audit(actor, before ? "user.update" : "user.create", v.email, before && before.role !== v.role ? `role ${before.role} → ${v.role}` : `role ${v.role}, active=${v.active}`);
      return delay(v);
    },

    async getSettings() { return delay(s.settings); },
    async saveSettings(v: Settings, actor) {
      guard(actor, "settings.edit");
      s.settings = SettingsSchema.parse(v);
      audit(actor, "settings.update", "global", JSON.stringify(s.settings));
      return delay(s.settings);
    },
  };
  return api;
}
