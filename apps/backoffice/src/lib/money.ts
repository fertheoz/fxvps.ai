/**
 * Money helpers. All monetary amounts in the back office are integers in the
 * currency's minor unit (e.g. cents). Floats are never used for balances.
 */

export const CURRENCY_MINOR_DIGITS: Record<string, number> = {
  USD: 2, EUR: 2, GBP: 2, TRY: 2, CHF: 2, AUD: 2, CAD: 2, JPY: 0, KWD: 3, BTC: 8,
};

export function minorDigits(currency: string): number {
  const d = CURRENCY_MINOR_DIGITS[currency.toUpperCase()];
  if (d === undefined) throw new Error(`Unknown currency: ${currency}`);
  return d;
}

export function assertMinor(amount: number): void {
  if (!Number.isSafeInteger(amount)) {
    throw new Error(`Amount must be a safe integer in minor units, got ${amount}`);
  }
}

/** Split integer minor units into sign, whole part and fractional digits (string math, no floats). */
function splitMinor(amount: number, digits: number): { neg: boolean; whole: string; frac: string } {
  assertMinor(amount);
  const neg = amount < 0;
  const abs = String(Math.abs(amount)).padStart(digits + 1, "0");
  const whole = digits === 0 ? abs : abs.slice(0, abs.length - digits);
  const frac = digits === 0 ? "" : abs.slice(abs.length - digits);
  return { neg, whole, frac };
}

/** Plain decimal string, e.g. formatMinorPlain(-12345, "USD") === "-123.45". */
export function formatMinorPlain(amount: number, currency: string): string {
  const { neg, whole, frac } = splitMinor(amount, minorDigits(currency));
  return `${neg ? "-" : ""}${whole}${frac ? "." + frac : ""}`;
}

/**
 * Locale-aware display. Grouping/decimal separators come from Intl but the
 * digits come from integer string math, so no precision is lost even for
 * amounts above 2^53 / 10^digits.
 */
export function formatMoney(
  amount: number,
  currency: string,
  locale = "en-US",
  opts: { compact?: boolean; signDisplay?: "auto" | "always" } = {},
): string {
  const digits = minorDigits(currency);
  if (opts.compact) {
    return new Intl.NumberFormat(locale, {
      style: "currency", currency, notation: "compact", maximumFractionDigits: 1,
      signDisplay: opts.signDisplay ?? "auto",
    }).format(amount / 10 ** digits);
  }
  const { neg, whole, frac } = splitMinor(amount, digits);
  const parts = new Intl.NumberFormat(locale, { style: "currency", currency, minimumFractionDigits: digits })
    .formatToParts(neg ? -1.5 : 1.5);
  const group = new Intl.NumberFormat(locale).formatToParts(1000000).find((p) => p.type === "group")?.value ?? ",";
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, group);
  let out = "";
  for (const p of parts) {
    if (p.type === "integer") out += grouped;
    else if (p.type === "fraction") out += frac;
    else if (p.type === "decimal") out += digits ? p.value : "";
    else out += p.value;
  }
  if (opts.signDisplay === "always" && !neg && amount !== 0) out = "+" + out;
  return out;
}

/**
 * Parse a user-typed decimal string into integer minor units.
 * Accepts "1234.5", "1,234.50" (en) or "1.234,50" (tr) depending on decimalSep.
 * Rejects more fractional digits than the currency allows.
 */
export function parseToMinor(input: string, currency: string, decimalSep: "." | "," = "."): number {
  const digits = minorDigits(currency);
  const groupSep = decimalSep === "." ? "," : ".";
  const s = input.trim().split(groupSep).join("").replace(/\s/g, "");
  const re = decimalSep === "." ? /^(-)?(\d+)(?:\.(\d+))?$/ : /^(-)?(\d+)(?:,(\d+))?$/;
  const m = re.exec(s);
  if (!m) throw new Error(`Invalid amount: "${input}"`);
  const [, sign, whole = "0", frac = ""] = m;
  if (frac.length > digits) throw new Error(`Too many decimals for ${currency} (max ${digits})`);
  const minor = Number(whole + frac.padEnd(digits, "0"));
  assertMinor(minor);
  return sign ? -minor : minor;
}

/** Sum integer minor amounts, guarding against unsafe results. */
export function sumMinor(values: readonly number[]): number {
  let total = 0;
  for (const v of values) {
    assertMinor(v);
    total += v;
  }
  assertMinor(total);
  return total;
}
