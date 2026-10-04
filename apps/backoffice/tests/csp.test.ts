import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { applyCsp, buildPolicy } from "../scripts/csp.mjs";

const sha = (s: string) => createHash("sha256").update(s, "utf8").digest("base64");

describe("static export CSP", () => {
  const html = `<!DOCTYPE html><html><head><meta charset="utf-8"><script>var a=1</script><script src="/_next/x.js" async></script></head><body><script>self.__next_f.push([1])</script></body></html>`;

  it("allows exactly the page's inline scripts by hash and the API origin", () => {
    const out = applyCsp(html, "https://api.fxvps.test/base");
    const meta = /<meta http-equiv="Content-Security-Policy" content="([^"]+)">/.exec(out)?.[1] ?? "";
    expect(out.indexOf("Content-Security-Policy")).toBeLessThan(out.indexOf("<script"));
    expect(meta).toContain(`'sha256-${sha("var a=1")}'`);
    expect(meta).toContain(`'sha256-${sha("self.__next_f.push([1])")}'`);
    expect(meta).not.toContain("unsafe-inline' 'sha");
    expect(meta).toMatch(/script-src 'self' 'sha256-/);
    expect(meta).not.toMatch(/script-src[^;]*unsafe-inline/);
    expect(meta).toContain("connect-src 'self' https://api.fxvps.test");
    expect(meta).toContain("object-src 'none'");
    // idempotent
    expect(applyCsp(out, "https://api.fxvps.test")).toBe(out);
  });

  it("without an API URL connects to self only", () => {
    expect(buildPolicy([], undefined)).toContain("connect-src 'self';");
    expect(buildPolicy([], "not a url")).toContain("connect-src 'self';");
  });
});
