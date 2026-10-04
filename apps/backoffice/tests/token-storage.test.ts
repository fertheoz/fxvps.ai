// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import { getToken, resetTokenCacheForTests, setToken } from "@/lib/auth";

describe("admin token storage", () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    resetTokenCacheForTests();
  });

  it("keeps the bearer token in sessionStorage, never localStorage", () => {
    setToken("a.b.c");
    expect(sessionStorage.getItem("fxvps-bo-token")).toBe("a.b.c");
    expect(localStorage.getItem("fxvps-bo-token")).toBeNull();
    resetTokenCacheForTests();
    expect(getToken()).toBe("a.b.c");
    setToken(null);
    expect(sessionStorage.getItem("fxvps-bo-token")).toBeNull();
  });

  it("drops a token persisted in localStorage by an older build", () => {
    localStorage.setItem("fxvps-bo-token", "old.token.x");
    expect(getToken()).toBeNull();
    expect(localStorage.getItem("fxvps-bo-token")).toBeNull();
  });
});
