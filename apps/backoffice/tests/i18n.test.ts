import { describe, expect, it } from "vitest";
import { en, tr, translate } from "@/lib/i18n";

describe("i18n", () => {
  it("Turkish covers every English key with a non-empty string", () => {
    expect(Object.keys(tr).sort()).toEqual(Object.keys(en).sort());
    for (const v of Object.values(tr)) expect(v.trim()).not.toBe("");
  });
  it("interpolates variables", () => {
    expect(translate("en", "positions.confirmClose", { n: 3 })).toContain("3 position");
    expect(translate("tr", "positions.confirmClose", { n: 3 })).toContain("3 pozisyon");
  });
});
