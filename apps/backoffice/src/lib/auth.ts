"use client";
import { useSyncExternalStore } from "react";
import { ROLES, type Role } from "./rbac";

/**
 * Bearer token for the HTTP adapter (live core-engine). The server derives the
 * actor and role from the token and re-checks every permission; the decoded
 * claims here are only used to render the UI for that role.
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
    const p = JSON.parse(b64urlDecode(t.split(".")[1] ?? "")) as Partial<TokenClaims>;
    if (typeof p.sub !== "string" || !ROLES.includes(p.role as Role)) return null;
    if (typeof p.exp === "number" && p.exp < nowSec) return null;
    return { sub: p.sub, name: typeof p.name === "string" ? p.name : p.sub, role: p.role as Role, exp: p.exp ?? 0 };
  } catch {
    return null;
  }
}

export function getToken(): string | null {
  if (!loaded && typeof window !== "undefined") {
    loaded = true;
    try {
      token = window.localStorage.getItem(KEY);
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
    if (t) window.localStorage.setItem(KEY, t);
    else window.localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
  listeners.forEach((l) => l());
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

/** `POST /auth/dev-token` (only served by core-engine with CORE_DEV_AUTH=1). */
export async function fetchDevToken(baseUrl: string, role: Role, name?: string, fetchImpl: typeof fetch = fetch): Promise<string> {
  const res = await fetchImpl(`${baseUrl}/auth/dev-token`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role, name, sub: `dev-${role}` }),
  });
  if (!res.ok) throw new Error(res.status === 404 ? "Dev auth is disabled on the server (CORE_DEV_AUTH=1)" : `dev-token → ${res.status}`);
  const body = (await res.json()) as { token: string };
  return body.token;
}
