"use client";
import { useCallback, useMemo } from "react";
import { usePrefs } from "./prefs";
import { LOCALES, translate, type MessageKey } from "./i18n";
import { can, type Permission } from "./rbac";
import { formatMoney } from "./money";
import type { Actor } from "./api";
import { decodeToken, isLive, useToken } from "./auth";
import type { Role } from "./rbac";

const ROLE_NAMES = { admin: "Admin Demo", dealer: "Deniz Dealer", risk: "Rita Risk", support: "Sam Support", readonly: "Olga Observer", partner: "Pat Partner" } as const;

export function useT() {
  const { locale } = usePrefs();
  return useCallback((key: MessageKey, vars?: Record<string, string | number>) => translate(locale, key, vars), [locale]);
}

export function useFormat() {
  const { locale } = usePrefs();
  return useMemo(() => {
  const intl = LOCALES[locale].intl;
  return {
    intl,
    money: (minor: number, ccy = "USD", o?: Parameters<typeof formatMoney>[3]) => formatMoney(minor, ccy, intl, o),
    num: (n: number, digits = 2) => new Intl.NumberFormat(intl, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(n),
    date: (iso: string) => new Intl.DateTimeFormat(intl, { dateStyle: "short", timeStyle: "medium", timeZone: "UTC" }).format(new Date(iso)),
  };
  }, [locale]);
}

/**
 * Current operator. Mock mode: the role picked in the header. Live mode: the
 * claims of the bearer token (the server re-checks everything).
 */
export function useActor(): Actor & { can: (p: Permission) => boolean } {
  const prefs = usePrefs();
  const token = useToken();
  const claims = isLive() ? decodeToken(token) : null;
  const role: Role = claims?.role ?? prefs.role;
  return { name: claims?.name ?? ROLE_NAMES[role], sub: claims?.sub, role, can: (p: Permission) => can(role, p) };
}
