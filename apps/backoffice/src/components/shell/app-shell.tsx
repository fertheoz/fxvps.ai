"use client";
import * as React from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Command, Languages, LogOut, Menu, Moon, Radio, ShieldOff, Sun } from "lucide-react";
import { NAV } from "./nav";
import { CommandPalette } from "./command-palette";
import { Button, Select } from "@/components/ui/primitives";
import { setPrefs, usePrefs } from "@/lib/prefs";
import { useActor, useT } from "@/lib/hooks";
import { decodeToken, identityLogout, identityUrl, isLive, securityUrl, setToken, startIdentitySession, useToken } from "@/lib/auth";
import { useMfaOk } from "@/lib/queries";
import { LiveUpdates, useLiveConnected } from "@/lib/queries";
import { LoginScreen } from "./login";
import { canAccessRoute, ROLES, type Role } from "@/lib/rbac";
import { LOCALES, type Locale } from "@/lib/i18n";
import { cn } from "@/lib/utils";

export function RouteGuard({ role, pathname, children }: { role: Role; pathname: string; children: React.ReactNode }) {
  const t = useT();
  if (canAccessRoute(role, pathname)) return <>{children}</>;
  return (
    <div className="mx-auto mt-24 max-w-md text-center" data-testid="access-denied">
      <ShieldOff className="mx-auto h-10 w-10 text-muted-foreground" />
      <h1 className="mt-3 text-lg font-semibold">{t("guard.title")}</h1>
      <p className="mt-1 text-sm text-muted-foreground">{t("guard.body")}</p>
      <Link href="/" className="mt-4 inline-block text-sm text-primary underline">{t("nav.dashboard")}</Link>
    </div>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname() ?? "/";
  const prefs = usePrefs();
  const t = useT();
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const [mobileNav, setMobileNav] = React.useState(false);
  const live = isLive();
  const token = useToken();
  const signedIn = !live || decodeToken(token) !== null;
  const actor = useActor();
  const pushed = useLiveConnected();

  // Identity sessions: keep the short-lived admin token fresh (no-op otherwise).
  React.useEffect(() => {
    if (live) startIdentitySession();
  }, [live]);

  React.useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const items = signedIn ? NAV.filter((n) => canAccessRoute(actor.role, n.href)) : [];
  const isActive = (href: string) => (href === "/" ? pathname === "/" : pathname.startsWith(href));

  const nav = (
    <nav className="grid gap-0.5 p-2" aria-label="main">
      {items.map((n) => (
        <Link
          key={n.href}
          href={n.href}
          onClick={() => setMobileNav(false)}
          className={cn("flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm", isActive(n.href) ? "bg-primary/10 font-medium text-primary" : "text-muted-foreground hover:bg-muted hover:text-foreground")}
        >
          <n.icon className="h-4 w-4" />
          {t(n.key)}
        </Link>
      ))}
    </nav>
  );

  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 hidden h-screen w-56 shrink-0 flex-col border-r border-border bg-card lg:flex">
        <div className="flex h-14 items-center gap-2 border-b border-border px-4 font-semibold">
          <span className="grid h-7 w-7 place-items-center rounded bg-primary text-xs text-primary-foreground">fx</span>
          <span>fxvps<span className="text-primary">.ai</span></span>
        </div>
        <div className="flex-1 overflow-y-auto">{nav}</div>
        <p className="flex items-center gap-1.5 border-t border-border p-3 text-[11px] text-muted-foreground" data-testid="data-source">
          {live ? <><Radio className={cn("h-3 w-3", pushed ? "text-emerald-500" : "text-muted-foreground")} />{t("common.live")}</> : t("common.demo")}
        </p>
      </aside>
      {mobileNav && (
        <div className="fixed inset-0 z-40 bg-black/40 lg:hidden" onClick={() => setMobileNav(false)}>
          <aside className="h-full w-64 bg-card" onClick={(e) => e.stopPropagation()}>{nav}</aside>
        </div>
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b border-border bg-background/85 px-3 backdrop-blur sm:px-5">
          <Button variant="ghost" size="icon" className="lg:hidden" onClick={() => setMobileNav(true)} aria-label="menu"><Menu className="h-4 w-4" /></Button>
          <button
            onClick={() => setPaletteOpen(true)}
            className="flex h-9 flex-1 items-center gap-2 rounded-md border border-border bg-muted/40 px-3 text-sm text-muted-foreground sm:max-w-sm cursor-pointer"
            data-testid="palette-trigger"
          >
            <Command className="h-3.5 w-3.5" />
            <span className="truncate">{t("palette.placeholder")}</span>
            <kbd className="ml-auto hidden rounded border border-border px-1.5 text-[10px] sm:inline">⌘K</kbd>
          </button>
          <div className="ml-auto flex items-center gap-1.5">
            {live ? (
              signedIn && (
                <>
                  <span className="hidden text-xs sm:inline" data-testid="current-user">{actor.name} <span className="text-muted-foreground">({actor.role})</span></span>
                  <Button variant="ghost" size="icon" aria-label={t("auth.logout")} title={t("auth.logout")} onClick={() => (identityUrl() ? void identityLogout() : setToken(null))} data-testid="logout">
                    <LogOut className="h-4 w-4" />
                  </Button>
                </>
              )
            ) : (
              <Select aria-label={t("common.role")} value={prefs.role} onChange={(e) => setPrefs({ role: e.target.value as Role })} data-testid="role-select">
                {ROLES.map((r) => <option key={r} value={r}>{r}</option>)}
              </Select>
            )}
            <Button variant="ghost" size="icon" aria-label={t("common.language")} onClick={() => setPrefs({ locale: prefs.locale === "en" ? "tr" : "en" })} title={LOCALES[prefs.locale === "en" ? "tr" : "en"].label}>
              <Languages className="h-4 w-4" /><span className="sr-only">{prefs.locale}</span>
            </Button>
            <span className="hidden text-xs font-medium uppercase text-muted-foreground sm:inline" data-testid="locale">{prefs.locale as Locale}</span>
            <Button variant="ghost" size="icon" aria-label={t("common.theme")} onClick={() => setPrefs({ theme: prefs.theme === "dark" ? "light" : "dark" })}>
              {prefs.theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </Button>
          </div>
        </header>
        <main className="min-w-0 flex-1 p-3 sm:p-5">
          {signedIn ? (
            <>
              {live && <LiveUpdates key={token ?? ""} />}
              {live && <MfaBanner />}
              <RouteGuard role={actor.role} pathname={pathname}>{children}</RouteGuard>
            </>
          ) : (
            <LoginScreen />
          )}
        </main>
      </div>
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
    </div>
  );
}

/**
 * Session without a 2FA login: changes (money, settings, LP) will be refused, so
 * say so up front with a link to the 2FA setup. Admins are sent there once per
 * browser session right after signing in.
 */
function MfaBanner() {
  const t = useT();
  const mfaOk = useMfaOk();
  const actor = useActor();
  const url = securityUrl();
  React.useEffect(() => {
    if (mfaOk || actor.role !== "admin" || !url) return;
    try {
      if (window.sessionStorage.getItem("fxvps-mfa-redirected")) return;
      window.sessionStorage.setItem("fxvps-mfa-redirected", "1");
    } catch {
      return;
    }
    window.location.assign(url);
  }, [mfaOk, actor.role, url]);
  if (mfaOk) return null;
  return (
    <div role="alert" data-testid="mfa-banner" className="mb-4 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
      <strong>{t("mfa.title")}</strong> {t("mfa.body")}{" "}
      {url && <a className="font-medium underline" href={url}>{t("mfa.setup")}</a>}
    </div>
  );
}
