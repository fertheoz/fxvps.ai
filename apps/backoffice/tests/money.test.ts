import { describe, expect, it } from "vitest";
import { formatMinorPlain, formatMoney, parseToMinor, sumMinor } from "@/lib/money";

describe("money (integer minor units)", () => {
  it("formats plain decimals without floats", () => {
    expect(formatMinorPlain(12345, "USD")).toBe("123.45");
    expect(formatMinorPlain(-5, "USD")).toBe("-0.05");
    expect(formatMinorPlain(0, "EUR")).toBe("0.00");
    expect(formatMinorPlain(1500, "JPY")).toBe("1500");
    expect(formatMinorPlain(1234, "KWD")).toBe("1.234");
    expect(formatMinorPlain(1, "BTC")).toBe("0.00000001");
  });

  it("formats with locale grouping and currency symbol", () => {
    expect(formatMoney(123456789, "USD", "en-US")).toBe("$1,234,567.89");
    expect(formatMoney(-123456789, "USD", "en-US")).toBe("-$1,234,567.89");
    expect(formatMoney(150000, "JPY", "en-US")).toBe("¥150,000");
    const tr = formatMoney(123456789, "TRY", "tr-TR");
    expect(tr).toContain("1.234.567,89");
    expect(formatMoney(100, "USD", "en-US", { signDisplay: "always" })).toBe("+$1.00");
  });

  it("keeps precision for amounts where float division would round", () => {
    // 2^53 - 1 cents: dividing by 100 as a float loses the last digit.
    expect(formatMinorPlain(Number.MAX_SAFE_INTEGER, "USD")).toBe("90071992547409.91");
    expect(formatMinorPlain(1_000_000_000_000_01, "USD")).toBe("1000000000000.01");
  });

  it("rejects non-integer or unsafe minor amounts", () => {
    expect(() => formatMinorPlain(1.5, "USD")).toThrow();
    expect(() => formatMoney(Number.MAX_SAFE_INTEGER + 2, "USD")).toThrow();
    expect(() => formatMinorPlain(1, "XXX")).toThrow(/Unknown currency/);
  });

  it("parses user input to minor units", () => {
    expect(parseToMinor("1,234.5", "USD")).toBe(123450);
    expect(parseToMinor("0.1", "USD")).toBe(10);
    expect(parseToMinor("1.234,56", "TRY", ",")).toBe(123456);
    expect(parseToMinor("-2", "USD")).toBe(-200);
    expect(parseToMinor("150", "JPY")).toBe(150);
    expect(() => parseToMinor("1.001", "USD")).toThrow(/Too many decimals/);
    expect(() => parseToMinor("abc", "USD")).toThrow();
    expect(() => parseToMinor("1.5", "JPY")).toThrow();
  });

  it("0.1 + 0.2 problem does not exist in minor units", () => {
    expect(sumMinor([parseToMinor("0.1", "USD"), parseToMinor("0.2", "USD")])).toBe(parseToMinor("0.3", "USD"));
    expect(() => sumMinor([Number.MAX_SAFE_INTEGER, 1])).toThrow();
  });
});
