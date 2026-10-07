import {
  Activity, BarChart3, CheckCheck, Cable, Network, FileText, GitBranch, LayoutDashboard, Layers, ScrollText, Settings, ShieldAlert, Tag, UserCog, Users,
  type LucideIcon,
} from "lucide-react";
import type { MessageKey } from "@/lib/i18n";
import { permissionForRoute } from "@/lib/rbac";

export interface NavItem { href: string; key: MessageKey; icon: LucideIcon }

export const NAV: NavItem[] = [
  { href: "/", key: "nav.dashboard", icon: LayoutDashboard },
  { href: "/clients", key: "nav.clients", icon: Users },
  { href: "/groups", key: "nav.groups", icon: Layers },
  { href: "/rules", key: "nav.rules", icon: GitBranch },
  { href: "/symbols", key: "nav.symbols", icon: Tag },
  { href: "/positions", key: "nav.positions", icon: Activity },
  { href: "/risk", key: "nav.risk", icon: ShieldAlert },
  { href: "/lp", key: "nav.lp", icon: Cable },
  { href: "/bridge", key: "nav.bridge", icon: Network },
  { href: "/reports", key: "nav.reports", icon: BarChart3 },
  { href: "/approvals", key: "nav.approvals", icon: CheckCheck },
  { href: "/audit", key: "nav.audit", icon: ScrollText },
  { href: "/users", key: "nav.users", icon: UserCog },
  { href: "/settings", key: "nav.settings", icon: Settings },
];

export const DOCS_ICON = FileText;

// Every nav entry must be covered by the RBAC route map.
for (const n of NAV) if (!permissionForRoute(n.href)) throw new Error(`No permission mapped for ${n.href}`);
