"use client";
import * as React from "react";
import { useRouter } from "next/navigation";
import { Settings2, Languages, Moon, UserCog } from "lucide-react";
import { NAV } from "./nav";
import { setPrefs, usePrefs } from "@/lib/prefs";
import { useActor, useT } from "@/lib/hooks";
import { isLive } from "@/lib/auth";
import { canAccessRoute, ROLES } from "@/lib/rbac";
import { cn } from "@/lib/utils";

interface Cmd { id: string; label: string; group: string; icon: React.ComponentType<{ className?: string }>; run: () => void }

export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return <PaletteBody onClose={onClose} />;
}

function PaletteBody({ onClose }: { onClose: () => void }) {
  const t = useT();
  const router = useRouter();
  const prefs = usePrefs();
  const actor = useActor();
  const [q, setQ] = React.useState("");
  const [idx, setIdx] = React.useState(0);

  const SETTING_LINKS: { key: import("@/lib/i18n").MessageKey; page: import("@/lib/i18n").MessageKey; href: string }[] = [
    { key: "groups.weekendLeverage", page: "nav.groups", href: "/groups" },
    { key: "groups.newsWindows", page: "nav.groups", href: "/groups" },
    { key: "groups.maxSlippage", page: "nav.groups", href: "/groups" },
    { key: "settings.fourEyes", page: "nav.settings", href: "/settings" },
    { key: "denetim.autoheal", page: "nav.denetim", href: "/denetim" },
    { key: "bridge.add", page: "nav.bridge", href: "/bridge" },
  ];
  const cmds: Cmd[] = [
    ...NAV.filter((n) => canAccessRoute(actor.role, n.href)).map((n) => ({ id: n.href, label: t(n.key), group: t("palette.hint"), icon: n.icon, run: () => router.push(n.href) })),
    // settings and fields that live inside pages ("weekend" -> Groups -> weekend leverage)
    ...SETTING_LINKS.filter((l) => canAccessRoute(actor.role, l.href)).map((l) => ({ id: `s-${l.key}`, label: `${t(l.key)} · ${t(l.page)}`, group: t("palette.settings"), icon: Settings2, run: () => router.push(l.href) })),
    { id: "theme", label: t("common.theme"), group: "⚙", icon: Moon, run: () => setPrefs({ theme: prefs.theme === "dark" ? "light" : "dark" }) },
    { id: "lang", label: `${t("common.language")}: ${prefs.locale === "en" ? "Türkçe" : "English"}`, group: "⚙", icon: Languages, run: () => setPrefs({ locale: prefs.locale === "en" ? "tr" : "en" }) },
    ...(isLive() ? [] : ROLES).map((r) => ({ id: `role-${r}`, label: `${t("common.role")}: ${r}`, group: "⚙", icon: UserCog, run: () => setPrefs({ role: r }) })),
  ];
  const filtered = cmds.filter((c) => c.label.toLowerCase().includes(q.toLowerCase()));
  const active = Math.min(idx, Math.max(0, filtered.length - 1));

  const exec = (c: Cmd | undefined) => {
    if (!c) return;
    c.run();
    onClose();
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[12vh]" onMouseDown={onClose}>
      <div role="dialog" aria-label="command palette" className="w-full max-w-lg overflow-hidden rounded-lg border border-border bg-card shadow-2xl" onMouseDown={(e) => e.stopPropagation()}>
        <input
          autoFocus
          value={q}
          onChange={(e) => { setQ(e.target.value); setIdx(0); }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") { e.preventDefault(); setIdx((i) => Math.min(i + 1, filtered.length - 1)); }
            else if (e.key === "ArrowUp") { e.preventDefault(); setIdx((i) => Math.max(i - 1, 0)); }
            else if (e.key === "Enter") exec(filtered[active]);
            else if (e.key === "Escape") onClose();
          }}
          placeholder={t("palette.placeholder")}
          className="h-12 w-full border-b border-border bg-transparent px-4 text-sm outline-none"
          data-testid="palette-input"
        />
        <ul className="max-h-80 overflow-y-auto p-1" role="listbox">
          {filtered.length === 0 && <li className="px-3 py-6 text-center text-sm text-muted-foreground">{t("common.noResults")}</li>}
          {filtered.map((c, i) => (
            <li
              key={c.id}
              role="option"
              aria-selected={i === active}
              onMouseEnter={() => setIdx(i)}
              onClick={() => exec(c)}
              className={cn("flex cursor-pointer items-center gap-2.5 rounded px-3 py-2 text-sm", i === active && "bg-muted")}
            >
              <c.icon className="h-4 w-4 text-muted-foreground" />
              <span>{c.label}</span>
              <span className="ml-auto text-[10px] text-muted-foreground">{c.group}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
