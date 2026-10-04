"use client";
import { useCallback, useMemo } from "react";
import { usePrefs } from "./prefs";
import { LOCALES, translate, type MessageKey } from "./i18n";
import { can, type Permission } from "./rbac";
import { formatMoney } from "./money";
import type { Actor } from "./api";

const ROLE_NAMES = { admin: "Admin Demo", dealer: "Deniz Dealer", risk: "Rita Risk", support: "Sam Support", readonly: "Olga Observer" } as const;

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

export function useActor(): Actor & { can: (p: Permission) => boolean } {
  const { role } = usePrefs();
  return { name: ROLE_NAMES[role], role, can: (p: Permission) => can(role, p) };
}
