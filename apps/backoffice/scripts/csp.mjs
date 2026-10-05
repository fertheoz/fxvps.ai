// Post-build: adds a Content-Security-Policy <meta> to every exported HTML page
// (finding G6). A static export cannot send per-response headers with nonces, so
// every inline <script> of the page is allowed by its SHA-256 hash and nothing
// else inline may run. `connect-src` is limited to the page origin and the admin
// API (NEXT_PUBLIC_API_URL). frame-ancestors / HSTS come from nginx (meta CSP
// cannot carry frame-ancestors).
//
//   node scripts/csp.mjs [outDir]       (default: out)
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const INLINE_SCRIPT = /<script(?![^>]*\bsrc\s*=)[^>]*>([\s\S]*?)<\/script>/gi;

/** Origin of an URL, or null. */
export function originOf(url) {
  try {
    return url ? new URL(url).origin : null;
  } catch {
    return null;
  }
}

/** The policy for one page given the hashes of its inline scripts. */
export function buildPolicy(hashes, apiUrl) {
  // apiUrl: one URL or a list (admin API + identity service).
  const api = [...new Set((Array.isArray(apiUrl) ? apiUrl : [apiUrl]).map(originOf).filter(Boolean))].join(" ");
  const scripts = ["'self'", ...hashes.map((h) => `'sha256-${h}'`)];
  return [
    "default-src 'self'",
    `script-src ${scripts.join(" ")}`,
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    `connect-src 'self'${api ? ` ${api}` : ""}`,
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join("; ");
}

/** Returns `html` with a CSP meta tag (inline scripts allowed by hash). */
export function applyCsp(html, apiUrl) {
  if (/http-equiv=["']Content-Security-Policy["']/i.test(html)) return html;
  const hashes = [];
  for (const m of html.matchAll(INLINE_SCRIPT)) {
    const body = m[1] ?? "";
    if (body.length === 0) continue;
    hashes.push(createHash("sha256").update(body, "utf8").digest("base64"));
  }
  const meta = `<meta http-equiv="Content-Security-Policy" content="${buildPolicy([...new Set(hashes)], apiUrl)}">`;
  // First element of <head>: the policy must precede every script.
  return html.replace(/<head(\s[^>]*)?>/i, (h) => `${h}${meta}`);
}

function walk(dir) {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : p.endsWith(".html") ? [p] : [];
  });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const out = process.argv[2] ?? "out";
  const files = walk(out);
  for (const f of files) writeFileSync(f, applyCsp(readFileSync(f, "utf8"), [process.env.NEXT_PUBLIC_API_URL, process.env.NEXT_PUBLIC_IDENTITY_URL]));
  console.log(`csp: ${files.length} page(s) in ${out}`);
}
