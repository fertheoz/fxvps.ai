import { z } from "zod";
import { ROLES } from "./rbac";
import { CURRENCY_MINOR_DIGITS } from "./money";

/** Integer amount in currency minor units. */
export const MinorAmount = z.number().int().refine(Number.isSafeInteger, "unsafe integer");
export const PositiveMinor = MinorAmount.refine((n) => n > 0, "must be > 0");
export const Currency = z.string().refine((c) => c in CURRENCY_MINOR_DIGITS, "unsupported currency");

export const KycStatus = z.enum(["none", "pending", "approved", "rejected"]);
export const AccountStatus = z.enum(["active", "disabled", "readonly"]);
export const Book = z.enum(["A", "B"]);
export const MarginMode = z.enum(["retail_hedged", "retail_netting", "exchange"]);
/** symbol: the symbol's own per-lot commission; per_lot / per_million: minor units of the group currency. */
export const CommissionType = z.enum(["symbol", "per_lot", "per_million"]);
/** What happens to the part of an A-book order the LP did not fill. */
export const PartialFillPolicy = z.enum(["cancel", "retry", "all_or_none"]);
/** ESMA leverage cap applied on top of the group leverage (retail: FX majors 1:30, minors/gold 1:20…). */
export const EsmaCap = z.enum(["none", "retail", "professional"]);

export const Group = z
  .object({
    id: z.string().min(1),
    name: z.string().min(2).max(64).regex(/^[A-Za-z0-9_\\/-]+$/, "letters, digits, _ - / only"),
    currency: Currency,
    leverage: z.number().int().min(1).max(3000),
    marginMode: MarginMode,
    marginCallPct: z.number().min(0).max(1000),
    stopOutPct: z.number().min(0).max(1000),
    commissionType: CommissionType,
    /** per_lot: minor units per lot; per_million: minor per 1M notional; percent: basis points */
    commissionValue: z.number().int().min(0),
    markupPoints: z.number().int().min(0).max(1000),
    swapMultiplier: z.number().min(0).max(10),
    book: Book,
    symbols: z.array(z.string()),
    esma: z.preprocess((v) => (v === null || v === undefined ? "none" : v), EsmaCap),
    partialFill: z.preprocess((v) => v ?? "cancel", PartialFillPolicy),
    maxAttempts: z.preprocess((v) => v ?? 3, z.number().int().min(1).max(10)),
    /** Per-side markups; null = `markupPoints` on that side. */
    markupBidPoints: z.preprocess((v) => (v === undefined ? null : v), z.number().int().min(0).max(1000).nullable()),
    markupAskPoints: z.preprocess((v) => (v === undefined ? null : v), z.number().int().min(0).max(1000).nullable()),
    /** symbol -> points, overrides the group markups on both sides. */
    symbolMarkups: z.preprocess((v) => v ?? {}, z.record(z.string(), z.number().int().min(0).max(1000))),
    /** A-book market orders: max points past the requested price (null = no cap). */
    maxSlippagePoints: z.preprocess((v) => (v === undefined ? null : v), z.number().int().min(0).max(1000).nullable()),
    /** false: a better fill is given at the requested price, the difference is ours. */
    passPriceImprovement: z.preprocess((v) => v ?? true, z.boolean()),
    /** A-book TP and pending limit entries rest at the LP as GTC limit orders. */
    lpResting: z.preprocess((v) => v ?? false, z.boolean()),
    /** Leverage cap Friday 20:00 - Sunday 22:00 UTC (null = none). */
    weekendLeverage: z.preprocess((v) => (v === undefined || v === 0 ? null : v), z.number().int().min(1).max(1000).nullable()),
    /** News windows: leverage cap between two instants (ms since epoch). */
    /** Volume tiers: notional above `from` (group currency) gets at most 1:leverage. */
    leverageTiers: z.preprocess((v) => v ?? [], z.array(z.object({ from: z.number().int().min(1), leverage: z.number().int().min(1).max(1000) })).max(20)),
    /** Swap-free groups (swap multiplier 0): flat fee per lot per night after grace days. */
    swapFreeFee: z.preprocess((v) => v ?? 0, z.number().min(0).max(1000)),
    swapFreeGraceDays: z.preprocess((v) => v ?? 0, z.number().int().min(0).max(365)),
    leverageWindows: z.preprocess((v) => v ?? [], z.array(z.object({ fromMs: z.number().int(), toMs: z.number().int(), leverage: z.number().int().min(1).max(1000) }).refine((w) => w.toMs > w.fromMs, { message: "end must be after start" })).max(50)),
  })
  .refine((g) => g.stopOutPct < g.marginCallPct, {
    message: "Stop-out level must be below margin call level",
    path: ["stopOutPct"],
  });
export type Group = z.infer<typeof Group>;

export const Weekday = z.enum(["mon", "tue", "wed", "thu", "fri", "sat", "sun"]);
const HHMM = z.string().regex(/^([01]\d|2[0-4]):[0-5]\d$/, "HH:MM");
export const Session = z.object({ day: Weekday, open: HHMM, close: HHMM });

export const SymbolSpec = z
  .object({
    name: z.string().min(1).max(32),
    description: z.string(),
    category: z.enum(["fx", "metal", "index", "energy", "crypto"]),
    digits: z.number().int().min(0).max(8),
    contractSize: z.number().int().positive(),
    tickSize: z.number().positive(),
    marginPct: z.number().min(0).max(100),
    minLot: z.number().positive(),
    maxLot: z.number().positive(),
    lotStep: z.number().positive(),
    swapLong: z.number(),
    swapShort: z.number(),
    swapType: z.enum(["points", "percent", "money"]),
    tripleSwapDay: Weekday,
    tradeSessions: z.array(Session),
    enabled: z.boolean(),
    lp: z.string(),
  })
  .refine((s) => s.minLot <= s.maxLot, { message: "minLot must be <= maxLot", path: ["minLot"] });
export type SymbolSpec = z.infer<typeof SymbolSpec>;

export const Client = z.object({
  id: z.string(),
  login: z.number().int(),
  parentId: z.string().nullable(),
  name: z.string(),
  email: z.string().email(),
  country: z.string().length(2),
  group: z.string(),
  status: AccountStatus,
  kyc: KycStatus,
  currency: Currency,
  leverage: z.number().int(),
  balance: MinorAmount,
  credit: MinorAmount,
  equity: MinorAmount,
  margin: MinorAmount,
  createdAt: z.string(),
  lastIp: z.string(),
  /** Legal Entity Identifier (transaction reporting). */
  lei: z.string().nullable().optional(),
  /** Introducing broker (stage 12). */
  ibSharePct: z.number().int().min(0).max(100).optional(),
  ibAccount: z.number().int().nullable().optional(),
  /** IB terms: rebate per closed lot (minor units), override on sub-IBs (%), referral code. */
  ibPlan: z.object({ perLotCents: z.number().int(), overridePct: z.number().int(), code: z.string() }).nullable().optional(),
  kycDocs: z.number().int().optional(),
});
export type Client = z.infer<typeof Client>;

export const BalanceOpType = z.enum(["deposit", "withdraw", "credit"]);
export const BalanceOpRequest = z.object({
  clientId: z.string().min(1),
  type: BalanceOpType,
  amount: PositiveMinor,
  currency: Currency,
  reason: z.string().trim().min(5, "reason is required (min 5 chars)").max(500),
  idempotencyKey: z.string().uuid(),
});
export type BalanceOpRequest = z.infer<typeof BalanceOpRequest>;

export const Side = z.enum(["buy", "sell"]);
export const Position = z.object({
  id: z.string(),
  clientId: z.string(),
  login: z.number().int(),
  symbol: z.string(),
  side: Side,
  lots: z.number().positive(),
  openPrice: z.number(),
  currentPrice: z.number(),
  pnl: MinorAmount,
  swap: MinorAmount,
  book: Book,
  openedAt: z.string(),
});
export type Position = z.infer<typeof Position>;

export const Order = z.object({
  id: z.string(),
  clientId: z.string(),
  login: z.number().int(),
  symbol: z.string(),
  side: Side,
  type: z.enum(["limit", "stop", "stop_limit"]),
  lots: z.number().positive(),
  price: z.number(),
  status: z.enum(["working", "partially_filled", "pending_lp"]),
  createdAt: z.string(),
});
export type Order = z.infer<typeof Order>;

export const FixSession = z.object({
  id: z.string(),
  lp: z.string(),
  kind: z.enum(["MD", "TRADING"]),
  senderCompId: z.string(),
  targetCompId: z.string(),
  status: z.enum(["logged_on", "connecting", "disconnected"]),
  inSeq: z.number().int().nonnegative(),
  outSeq: z.number().int().nonnegative(),
  latencyMs: z.number().nonnegative(),
  rejects24h: z.number().int().nonnegative(),
  lastHeartbeat: z.string(),
  /** Last disconnect / halt reason (e.g. repeated failed logons). */
  lastError: z.string().nullable().optional(),
});
export type FixSession = z.infer<typeof FixSession>;

export const AuditEntry = z.object({
  id: z.string(),
  at: z.string(),
  actor: z.string(),
  role: z.enum(ROLES),
  action: z.string(),
  target: z.string(),
  details: z.string(),
  hash: z.string().optional(),
});
export type AuditEntry = z.infer<typeof AuditEntry>;

export const AdminUser = z.object({
  id: z.string(),
  name: z.string().min(2),
  email: z.string().email(),
  role: z.enum(ROLES),
  mfa: z.boolean(),
  active: z.boolean(),
  lastLogin: z.string().nullable(),
  /** Tenant scope (stage 14); null = all tenants. */
  tenant: z.string().nullable().optional(),
});
export type AdminUser = z.infer<typeof AdminUser>;

export const Trade = z.object({
  id: z.string(),
  login: z.number().int(),
  symbol: z.string(),
  side: Side,
  lots: z.number(),
  openPrice: z.number(),
  closePrice: z.number(),
  pnl: MinorAmount,
  commission: MinorAmount,
  swap: MinorAmount,
  book: Book,
  closedAt: z.string(),
});
export type Trade = z.infer<typeof Trade>;

export const EsmaPreset = z.object({
  id: z.string(),
  label: z.string(),
  maxLeverage: z.number().int().positive(),
  marginCallPct: z.number(),
  stopOutPct: z.number(),
  negativeBalanceProtection: z.boolean(),
});
export type EsmaPreset = z.infer<typeof EsmaPreset>;

export const Settings = z.object({
  brokerName: z.string().min(2),
  baseCurrency: Currency,
  fourEyesThreshold: PositiveMinor,
  sessionTimeoutMin: z.number().int().min(5).max(480),
  requireMfa: z.boolean(),
  defaultBook: Book,
  brokerLei: z.string().optional(),
  funding: z.object({ usdtTrc20Address: z.string(), bankDetails: z.string(), minDepositMinor: z.number().int().min(0), minWithdrawMinor: z.number().int().min(0) }).optional(),
});
export type Settings = z.infer<typeof Settings>;
