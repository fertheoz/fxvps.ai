"use client";
import { useSyncExternalStore } from "react";
import type { Role } from "./rbac";
import type { Locale } from "./i18n";

export interface Prefs {
  role: Role;
  locale: Locale;
  theme: "light" | "dark";
}

const KEY = "fxvps-bo-prefs";
const DEFAULTS: Prefs = { role: "admin", locale: "en", theme: "dark" };
let current: Prefs = DEFAULTS;
let loaded = false;
const listeners = new Set<() => void>();

function load(): Prefs {
  if (!loaded && typeof window !== "undefined") {
    loaded = true;
    try {
      const raw = window.localStorage.getItem(KEY);
      if (raw) current = { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Prefs>) };
    } catch {
      /* storage unavailable: keep defaults */
    }
  }
  return current;
}

export function setPrefs(patch: Partial<Prefs>) {
  current = { ...load(), ...patch };
  try {
    window.localStorage.setItem(KEY, JSON.stringify(current));
  } catch {
    /* ignore */
  }
  document.documentElement.classList.toggle("dark", current.theme === "dark");
  document.documentElement.lang = current.locale;
  listeners.forEach((l) => l());
}

export function usePrefs(): Prefs {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    load,
    () => DEFAULTS,
  );
}
