"use client";
import { useSyncExternalStore } from "react";
import { ROLES, type Role } from "./rbac";

/**
 * Bearer token for the HTTP adapter (live core-engine). The server derives the
 * actor and role from the token and re-checks every permission; the decoded
 * claims here are only used to render the UI for that role.
 *
 * Storage (finding G6): memory + sessionStorage (this tab only, gone when the
 * tab closes), never localStorage. A token left in localStorage by an older
 * build is deleted on first read.
 */
export interface TokenClaims {
  sub: string;
  name: string;
  role: Role;
  exp: number;
}

const KEY = "fxvps-bo-token";
let token: string | null = null;
let loaded = false;
const listeners = new Set<() => void>();

export const apiUrl = (): string | undefined => process.env.NEXT_PUBLIC_API_URL || undefined;
export const isLive = (): boolean => !!apiUrl();
/** Dev-token button: in development builds or when NEXT_PUBLIC_DEV_AUTH=1. */
/** "Continue with Cloudflare Access" (console published behind Tunnel + Access). */
export const accessAuthUi = (): boolean => process.env.NEXT_PUBLIC_CF_ACCESS === "1";
export const devAuthUi = (): boolean => process.env.NODE_ENV !== "production" || process.env.NEXT_PUBLIC_DEV_AUTH === "1";

function b64urlDecode(s: string): string {
  const pad = s.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(pad + "===".slice((pad.length + 3) % 4));
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/** Decodes (does not verify) a JWT payload; null when malformed or expired. */
export function decodeToken(t: string | null, nowSec = Date.now() / 1000): TokenClaims | null {
  if (!t) return null;
  try {
    const p = JSON.parse(b64urlDecode(t.split(".")[1] ?? "")) as Partial<TokenClaims> & { roles?: unknown; email?: unknown };
    // Back-office tokens carry `role`; identity tokens a `roles` array (highest wins).
    const roles = Array.isArray(p.roles) ? p.roles : [];
    const role = ROLES.includes(p.role as Role) ? (p.role as Role) : ROLES.find((r) => roles.includes(r));
    if (typeof p.sub !== "string" || !role) return null;
    if (typeof p.exp === "number" && p.exp < nowSec) return null;
    const name = typeof p.name === "string" ? p.name : typeof p.email === "string" ? p.email : p.sub;
    return { sub: p.sub, name, role, exp: p.exp ?? 0 };
  } catch {
    return null;
  }
}

export function getToken(): string | null {
  if (!loaded && typeof window !== "undefined") {
    loaded = true;
    try {
      window.localStorage.removeItem(KEY);
    } catch {
      /* storage unavailable */
    }
    try {
      token = window.sessionStorage.getItem(KEY);
    } catch {
      /* storage unavailable */
    }
  }
  return token;
}

export function setToken(t: string | null) {
  token = t;
  loaded = true;
  try {
    if (t) window.sessionStorage.setItem(KEY, t);
    else window.sessionStorage.removeItem(KEY);
  } catch {
    /* ignore: the token stays in memory only */
  }
  listeners.forEach((l) => l());
}

/** Test hook: forget the in-memory token so the next read hits storage again. */
export function resetTokenCacheForTests() {
  token = null;
  loaded = false;
}

export function useToken(): string | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    getToken,
    () => null,
  );
}

/**
 * `POST /auth/dev-token` (only served by core-engine with CORE_DEV_AUTH=1 on a
 * loopback address). The server derives `sub` from the role.
 */
export async function fetchDevToken(baseUrl: string, role: Role, name?: string, fetchImpl: typeof fetch = fetch): Promise<string> {
  // `/` (same-origin console build) must not become a protocol-relative `//auth/...`.
  const res = await fetchImpl(`${baseUrl.replace(/\/+$/, "")}/auth/dev-token`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role, name }),
  });
  if (!res.ok) throw new Error(res.status === 404 ? "Dev auth is disabled on the server (CORE_DEV_AUTH=1)" : `dev-token → ${res.status}`);
  const body = (await res.json()) as { token: string };
  return body.token;
}

/**
 * `POST /auth/access-token`: core-engine verifies the Cloudflare Access assertion
 * that Cloudflare adds to the request and returns a 1 h session token.
 */
export async function fetchAccessToken(baseUrl: string, fetchImpl: typeof fetch = fetch): Promise<string> {
  const res = await fetchImpl(`${baseUrl.replace(/\/+$/, "")}/auth/access-token`, { method: "POST", credentials: "same-origin" });
  if (!res.ok) throw new Error(res.status === 404 ? "Cloudflare Access login is not enabled on the server" : `access-token → ${res.status}`);
  return ((await res.json()) as { token: string }).token;
}

// --- identity service login (console behind console.fxvps.ai) ----------------

/** NEXT_PUBLIC_IDENTITY_URL: identity service for e-mail/password/2FA login. */
export const identityUrl = (): string | undefined => process.env.NEXT_PUBLIC_IDENTITY_URL?.replace(/\/+$/, "") || undefined;

export type IdentityLogin = { token: string } | { mfaToken: string };

async function idCall<T>(path: string, body?: unknown, fetchImpl: typeof fetch = fetch): Promise<T> {
  const res = await fetchImpl(`${identityUrl()}${path}`, {
    method: "POST",
    credentials: "include", // HttpOnly refresh cookie (path /v1/token) on the identity host
    // Cookie endpoints require the CSRF header (identity service).
    headers: { ...(body === undefined ? {} : { "content-type": "application/json" }), "x-fxvps-csrf": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const v = (await res.json().catch(() => ({}))) as T & { error?: string; message?: string };
  if (!res.ok) throw new Error(v.message ?? v.error ?? `${path} → ${res.status}`);
  return v;
}

/** Password step; returns the access token or an MFA challenge. */
export async function identityLogin(email: string, password: string, fetchImpl?: typeof fetch): Promise<IdentityLogin> {
  const r = await idCall<{ access_token?: string; mfa_required?: boolean; mfa_token?: string }>("/v1/login", { email, password, session: "cookie" }, fetchImpl);
  if (r.mfa_required && r.mfa_token) return { mfaToken: r.mfa_token };
  if (r.access_token) return { token: r.access_token };
  throw new Error("unexpected login response");
}

export async function identity2fa(mfaToken: string, code: string, fetchImpl?: typeof fetch): Promise<string> {
  const r = await idCall<{ access_token: string }>("/v1/login/2fa", { mfa_token: mfaToken, code, session: "cookie" }, fetchImpl);
  return r.access_token;
}

/** Rotates the refresh cookie; null when there is no session. */
export async function identityRefresh(fetchImpl?: typeof fetch): Promise<string | null> {
  try {
    return (await idCall<{ access_token: string }>("/v1/token/refresh", undefined, fetchImpl)).access_token;
  } catch {
    return null;
  }
}

let refresher: ReturnType<typeof setInterval> | null = null;

/**
 * Keeps an identity session alive: privileged access tokens live ~60 s, so the
 * token is refreshed in the background (and once at start-up from the cookie).
 */
export function startIdentitySession() {
  if (!identityUrl() || typeof window === "undefined" || refresher) return;
  const tick = async () => {
    const t = await identityRefresh();
    if (t) setToken(t);
    else if (decodeToken(getToken()) === null) setToken(null);
  };
  void tick();
  refresher = setInterval(() => void tick(), 30_000);
}

export async function identityLogout() {
  if (refresher) clearInterval(refresher);
  refresher = null;
  await idCall("/v1/token/revoke").catch(() => undefined);
  setToken(null);
}

/** NEXT_PUBLIC_TERMINAL_URL: client terminal (2FA is set up there, in "Account security"). */
export const terminalUrl = (): string | undefined => process.env.NEXT_PUBLIC_TERMINAL_URL?.replace(/\/+$/, "") || undefined;
/** Terminal link that opens the account-security dialog. */
export const securityUrl = (): string | undefined => {
  const t = terminalUrl();
  return t ? `${t}/?security=1` : undefined;
};
