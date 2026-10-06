import type {
  AdminUser, AuditEntry, Client, EsmaPreset, FixSession, Group, Order, Position, Settings, SymbolSpec, Trade,
} from "../schemas";

/** Deterministic PRNG so the demo (and screenshots) are stable. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const SEED_NOW = Date.UTC(2026, 9, 2, 14, 30);

const FX_SESS = (["mon", "tue", "wed", "thu", "fri"] as const).map((day) => ({ day, open: "00:00", close: "24:00" }));
const IDX_SESS = (["mon", "tue", "wed", "thu", "fri"] as const).map((day) => ({ day, open: "01:05", close: "23:55" }));
const CRYPTO_SESS = (["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const).map((day) => ({ day, open: "00:00", close: "24:00" }));

type Cat = SymbolSpec["category"];
const SYMBOL_DEFS: [string, string, Cat, number, number, number][] = [
  // name, description, category, digits, contractSize, price
  ["EURUSD", "Euro vs US Dollar", "fx", 5, 100000, 1.0842],
  ["GBPUSD", "Pound vs US Dollar", "fx", 5, 100000, 1.2715],
  ["USDJPY", "US Dollar vs Yen", "fx", 3, 100000, 149.32],
  ["USDCHF", "US Dollar vs Franc", "fx", 5, 100000, 0.8841],
  ["AUDUSD", "Aussie vs US Dollar", "fx", 5, 100000, 0.6588],
  ["USDCAD", "US Dollar vs Loonie", "fx", 5, 100000, 1.3612],
  ["EURGBP", "Euro vs Pound", "fx", 5, 100000, 0.8527],
  ["USDTRY", "US Dollar vs Lira", "fx", 4, 100000, 41.215],
  ["XAUUSD", "Gold vs US Dollar", "metal", 2, 100, 2648.5],
  ["XAGUSD", "Silver vs US Dollar", "metal", 3, 5000, 31.42],
  ["US500", "S&P 500 Index", "index", 1, 1, 5812.4],
  ["NAS100", "Nasdaq 100 Index", "index", 1, 1, 20431.7],
  ["GER40", "DAX 40 Index", "index", 1, 1, 19284.2],
  ["USOIL", "WTI Crude Oil", "energy", 2, 1000, 71.35],
  ["BTCUSD", "Bitcoin vs US Dollar", "crypto", 2, 1, 64120.5],
  ["ETHUSD", "Ether vs US Dollar", "crypto", 2, 1, 2581.3],
];

export const SYMBOL_PRICES: Record<string, number> = Object.fromEntries(SYMBOL_DEFS.map((d) => [d[0], d[5]]));

export function seedSymbols(): SymbolSpec[] {
  return SYMBOL_DEFS.map(([name, description, category, digits, contractSize]) => ({
    name, description, category, digits, contractSize,
    tickSize: 1 / 10 ** digits,
    marginPct: category === "fx" ? 3.33 : category === "metal" ? 5 : category === "crypto" ? 50 : 10,
    minLot: 0.01, maxLot: category === "crypto" ? 10 : 100, lotStep: 0.01,
    swapLong: category === "crypto" ? -20 : -6.2, swapShort: category === "crypto" ? -20 : 1.4,
    swapType: category === "crypto" ? "percent" : "points",
    tripleSwapDay: "wed",
    tradeSessions: category === "crypto" ? CRYPTO_SESS : category === "fx" || category === "metal" ? FX_SESS : IDX_SESS,
    enabled: name !== "USDTRY",
    lp: category === "crypto" ? "LP-B" : "LMAX",
  }));
}

export function seedGroups(): Group[] {
  const all = SYMBOL_DEFS.map((d) => d[0]);
  return [
    { id: "g1", name: "retail/eu-standard", currency: "EUR", leverage: 30, marginMode: "retail_hedged", marginCallPct: 100, stopOutPct: 50, commissionType: "per_lot", commissionValue: 350, markupPoints: 5, swapMultiplier: 1, book: "A", symbols: all, partialFill: "cancel", maxAttempts: 3, esma: "retail" },
    { id: "g2", name: "retail/global-pro", currency: "USD", leverage: 500, marginMode: "retail_hedged", marginCallPct: 100, stopOutPct: 30, commissionType: "per_lot", commissionValue: 0, markupPoints: 12, swapMultiplier: 1.2, book: "B", symbols: all, partialFill: "cancel", maxAttempts: 3, esma: "none" },
    { id: "g3", name: "retail/tr-standard", currency: "USD", leverage: 100, marginMode: "retail_netting", marginCallPct: 120, stopOutPct: 50, commissionType: "per_million", commissionValue: 2500, markupPoints: 8, swapMultiplier: 1, book: "A", symbols: all, partialFill: "cancel", maxAttempts: 3, esma: "none" },
    { id: "g4", name: "pro/ecn", currency: "USD", leverage: 200, marginMode: "retail_hedged", marginCallPct: 80, stopOutPct: 40, commissionType: "per_lot", commissionValue: 600, markupPoints: 0, swapMultiplier: 1, book: "A", symbols: all, partialFill: "cancel", maxAttempts: 3, esma: "none" },
    { id: "g5", name: "demo/default", currency: "USD", leverage: 100, marginMode: "retail_hedged", marginCallPct: 100, stopOutPct: 50, commissionType: "per_lot", commissionValue: 0, markupPoints: 10, swapMultiplier: 1, book: "B", symbols: all, partialFill: "cancel", maxAttempts: 3, esma: "none" },
  ];
}

const FIRST = ["Ayşe", "Mehmet", "John", "Emma", "Lukas", "Sofia", "Can", "Elif", "Oliver", "Mia", "Ahmet", "Zeynep", "Noah", "Hannah", "Marco", "Chloe", "Burak", "Deniz", "Ivan", "Lea"];
const LAST = ["Yılmaz", "Kaya", "Smith", "Müller", "Rossi", "Demir", "Brown", "Schmidt", "Öztürk", "Dubois", "Garcia", "Arslan", "Novak", "Weber"];
const COUNTRIES = ["TR", "DE", "GB", "FR", "IT", "ES", "NL", "AE", "PL", "CY"];
const KYC: Client["kyc"][] = ["approved", "approved", "approved", "pending", "none", "rejected"];

export interface SeedData {
  clients: Client[];
  positions: Position[];
  orders: Order[];
  trades: Trade[];
  groups: Group[];
  symbols: SymbolSpec[];
  fix: FixSession[];
  audit: AuditEntry[];
  users: AdminUser[];
  presets: EsmaPreset[];
  settings: Settings;
}

/** USD value of 1 unit of quote currency for the symbol (approximation for the demo). */
export function quoteToUsd(symbol: string, price: number): number {
  if (symbol.endsWith("USD") || symbol.length !== 6) return 1;
  if (symbol.startsWith("USD")) return 1 / price;
  if (symbol.endsWith("GBP")) return SYMBOL_PRICES.GBPUSD ?? 1.27;
  return 1;
}

/** P&L in USD cents (integer). */
export function positionPnlMinor(symbol: string, side: "buy" | "sell", lots: number, open: number, current: number, contractSize: number): number {
  const dir = side === "buy" ? 1 : -1;
  return Math.round((current - open) * dir * lots * contractSize * quoteToUsd(symbol, current) * 100);
}

export function notionalMinor(symbol: string, lots: number, price: number, contractSize: number): number {
  const usd = symbol.startsWith("USD") && symbol.length === 6 ? lots * contractSize : lots * contractSize * price * quoteToUsd(symbol, price);
  return Math.round(usd * 100);
}

export function seed(seedValue = 42): SeedData {
  const r = mulberry32(seedValue);
  const pick = <T,>(arr: readonly T[]): T => arr[Math.floor(r() * arr.length)]!;
  const symbols = seedSymbols();
  const groups = seedGroups();
  const iso = (msAgo: number) => new Date(SEED_NOW - msAgo).toISOString();

  const clients: Client[] = [];
  let login = 700100;
  for (let i = 0; i < 48; i++) {
    const g = pick(groups);
    const first = pick(FIRST), last = pick(LAST);
    const id = `c${i + 1}`;
    const balance = Math.round((500 + r() * 80000) * 100);
    const master: Client = {
      id, login: login++, parentId: null, name: `${first} ${last}`,
      email: `${first}.${last}${i}@example.com`.toLowerCase().normalize("NFD").replace(/[^\x00-\x7f]/g, ""),
      country: pick(COUNTRIES), group: g.name, status: r() < 0.9 ? "active" : pick(["disabled", "readonly"] as const),
      kyc: pick(KYC), currency: "USD", leverage: g.leverage, balance, credit: r() < 0.2 ? 50000 : 0,
      equity: balance, margin: 0, createdAt: iso(r() * 400 * 864e5), lastIp: `85.${Math.floor(r() * 255)}.${Math.floor(r() * 255)}.${Math.floor(r() * 255)}`,
    };
    clients.push(master);
    const subs = r() < 0.35 ? 1 + Math.floor(r() * 2) : 0;
    for (let s = 0; s < subs; s++) {
      const sb = Math.round((200 + r() * 15000) * 100);
      clients.push({ ...master, id: `${id}-s${s + 1}`, login: login++, parentId: id, balance: sb, credit: 0, equity: sb, name: `${master.name} (sub ${s + 1})` });
    }
  }

  const positions: Position[] = [];
  let pid = 1;
  for (const c of clients) {
    if (c.status === "disabled") continue;
    const n = Math.floor(r() * 4);
    for (let k = 0; k < n; k++) {
      const sym = pick(symbols);
      const base = SYMBOL_PRICES[sym.name]!;
      const open = +(base * (1 + (r() - 0.5) * 0.01)).toFixed(sym.digits);
      const lotsRaw = sym.category === "fx" ? 0.1 + r() * 3 : sym.category === "index" || sym.category === "crypto" ? 0.5 + r() * 5 : 0.1 + r() * 2;
      const lots = Math.round(lotsRaw * 100) / 100;
      const side = r() < 0.55 ? "buy" : "sell";
      const grp = groups.find((g) => g.name === c.group)!;
      positions.push({
        id: `p${pid++}`, clientId: c.id, login: c.login, symbol: sym.name, side, lots, openPrice: open, currentPrice: base,
        pnl: positionPnlMinor(sym.name, side, lots, open, base, sym.contractSize), swap: -Math.round(r() * 1500),
        book: grp.book, openedAt: iso(r() * 10 * 864e5),
      });
    }
  }
  // Recompute equity/margin; force a few accounts into margin call / stop-out for the risk screen.
  const stressed = new Set(clients.filter((c) => c.parentId === null).slice(0, 6).map((c) => c.id));
  for (const c of clients) {
    const ps = positions.filter((p) => p.clientId === c.id);
    const sym = (n: string) => symbols.find((s) => s.name === n)!;
    c.margin = ps.reduce((a, p) => a + Math.round(notionalMinor(p.symbol, p.lots, p.currentPrice, sym(p.symbol).contractSize) / c.leverage), 0);
    const floating = ps.reduce((a, p) => a + p.pnl + p.swap, 0);
    if (stressed.has(c.id) && c.margin > 0) {
      const target = 0.35 + r() * 0.8; // margin level 35%..115%
      c.balance = Math.max(1000, Math.round(c.margin * target) - floating - c.credit);
    }
    c.equity = c.balance + c.credit + floating;
  }

  const orders: Order[] = [];
  for (let i = 0; i < 26; i++) {
    const c = pick(clients), sym = pick(symbols), base = SYMBOL_PRICES[sym.name]!;
    orders.push({
      id: `o${i + 1}`, clientId: c.id, login: c.login, symbol: sym.name, side: r() < 0.5 ? "buy" : "sell",
      type: pick(["limit", "stop", "stop_limit"] as const), lots: Math.round((0.1 + r() * 2) * 100) / 100,
      price: +(base * (1 + (r() - 0.5) * 0.02)).toFixed(sym.digits), status: pick(["working", "working", "partially_filled", "pending_lp"] as const),
      createdAt: iso(r() * 3 * 864e5),
    });
  }

  const trades: Trade[] = [];
  for (let i = 0; i < 160; i++) {
    const c = pick(clients), sym = pick(symbols), base = SYMBOL_PRICES[sym.name]!;
    const open = +(base * (1 + (r() - 0.5) * 0.01)).toFixed(sym.digits);
    const close = +(base * (1 + (r() - 0.5) * 0.01)).toFixed(sym.digits);
    const lots = Math.round((0.05 + r() * 2) * 100) / 100, side = r() < 0.5 ? "buy" : "sell";
    const grp = groups.find((g) => g.name === c.group)!;
    trades.push({
      id: `t${i + 1}`, login: c.login, symbol: sym.name, side, lots, openPrice: open, closePrice: close,
      pnl: positionPnlMinor(sym.name, side, lots, open, close, sym.contractSize), commission: -Math.round(lots * 350),
      swap: -Math.round(r() * 400), book: grp.book, closedAt: iso(r() * 30 * 864e5),
    });
  }
  trades.sort((a, b) => b.closedAt.localeCompare(a.closedAt));

  const fix: FixSession[] = [
    { id: "f1", lp: "LMAX", kind: "MD", senderCompId: "FXVPS_MD", targetCompId: "LMXBDM", status: "logged_on", inSeq: 4812233, outSeq: 1882, latencyMs: 0.8, rejects24h: 0, lastHeartbeat: iso(4000) },
    { id: "f2", lp: "LMAX", kind: "TRADING", senderCompId: "FXVPS_TR", targetCompId: "LMXBD", status: "logged_on", inSeq: 92211, outSeq: 92190, latencyMs: 1.4, rejects24h: 3, lastHeartbeat: iso(6000) },
    { id: "f3", lp: "LP-B", kind: "MD", senderCompId: "FXVPS_MD2", targetCompId: "LPB_PX", status: "logged_on", inSeq: 1209931, outSeq: 744, latencyMs: 3.2, rejects24h: 0, lastHeartbeat: iso(9000) },
    { id: "f4", lp: "LP-B", kind: "TRADING", senderCompId: "FXVPS_TR2", targetCompId: "LPB_OE", status: "disconnected", inSeq: 18832, outSeq: 18840, latencyMs: 0, rejects24h: 11, lastHeartbeat: iso(420000) },
    { id: "f5", lp: "LP-C (UAT)", kind: "MD", senderCompId: "FXVPS_UAT", targetCompId: "LPC", status: "connecting", inSeq: 1, outSeq: 1, latencyMs: 0, rejects24h: 0, lastHeartbeat: iso(60000) },
  ];

  const users: AdminUser[] = [
    { id: "u1", name: "Admin Demo", email: "admin@fxvps.example", role: "admin", mfa: true, active: true, lastLogin: iso(3600e3) },
    { id: "u2", name: "Deniz Dealer", email: "dealer@fxvps.example", role: "dealer", mfa: true, active: true, lastLogin: iso(7200e3) },
    { id: "u3", name: "Rita Risk", email: "risk@fxvps.example", role: "risk", mfa: true, active: true, lastLogin: iso(86400e3) },
    { id: "u4", name: "Sam Support", email: "support@fxvps.example", role: "support", mfa: false, active: true, lastLogin: iso(2 * 86400e3) },
    { id: "u5", name: "Olga Observer", email: "auditor@fxvps.example", role: "readonly", mfa: true, active: true, lastLogin: null },
  ];

  const audit: AuditEntry[] = [];
  const actions: [string, string][] = [["balance.deposit", "Deposit 1,000.00 USD"], ["group.update", "leverage 100 → 200"], ["symbol.update", "swapLong -5.8 → -6.2"], ["position.forceClose", "Closed 2 positions"], ["kyc.approve", "KYC approved"], ["lp.reconnect", "LP-B TRADING reconnect"], ["user.login", "MFA ok"]];
  for (let i = 0; i < 40; i++) {
    const u = pick(users), [action, details] = pick(actions), c = pick(clients);
    audit.push({ id: `a${i + 1}`, at: iso(i * 3.3 * 3600e3 + r() * 3600e3), actor: u.name, role: u.role, action, target: action.startsWith("lp") ? "LP-B" : `#${c.login}`, details });
  }

  const presets: EsmaPreset[] = [
    { id: "esma-fx-major", label: "ESMA retail — FX majors (1:30)", maxLeverage: 30, marginCallPct: 100, stopOutPct: 50, negativeBalanceProtection: true },
    { id: "esma-fx-minor", label: "ESMA retail — FX minors, gold, major indices (1:20)", maxLeverage: 20, marginCallPct: 100, stopOutPct: 50, negativeBalanceProtection: true },
    { id: "esma-commodity", label: "ESMA retail — other commodities, minor indices (1:10)", maxLeverage: 10, marginCallPct: 100, stopOutPct: 50, negativeBalanceProtection: true },
    { id: "esma-equity", label: "ESMA retail — single equities (1:5)", maxLeverage: 5, marginCallPct: 100, stopOutPct: 50, negativeBalanceProtection: true },
    { id: "esma-crypto", label: "ESMA retail — crypto (1:2)", maxLeverage: 2, marginCallPct: 100, stopOutPct: 50, negativeBalanceProtection: true },
  ];

  const settings: Settings = { brokerName: "fxvps.ai (demo)", baseCurrency: "USD", fourEyesThreshold: 1_000_000, sessionTimeoutMin: 30, requireMfa: true, defaultBook: "A" };

  return { clients, positions, orders, trades, groups, symbols, fix, audit, users, presets, settings };
}
