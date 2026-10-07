/** Role-based access control for the back office. Pure functions; unit tested. */

export const ROLES = ["admin", "dealer", "risk", "support", "readonly", "partner"] as const;
export type Role = (typeof ROLES)[number];

export const PERMISSIONS = [
  "dashboard.view",
  "clients.view",
  "clients.edit",
  "balance.deposit",
  "balance.withdraw",
  "balance.credit",
  "balance.approve",
  "groups.view",
  "groups.edit",
  "symbols.view",
  "symbols.edit",
  "positions.view",
  "positions.forceClose",
  "risk.view",
  "risk.edit",
  "lp.view",
  "lp.reconnect",
  "lp.manage",
  "reports.view",
  "reports.export",
  "audit.view",
  "users.view",
  "users.edit",
  "settings.view",
  "settings.edit",
  "partner.view",
] as const;
export type Permission = (typeof PERMISSIONS)[number];

const VIEW_ALL: Permission[] = PERMISSIONS.filter((p) => p.endsWith(".view"));

export const ROLE_PERMISSIONS: Record<Role, readonly Permission[]> = {
  admin: PERMISSIONS,
  dealer: [
    "dashboard.view", "clients.view", "groups.view", "symbols.view", "symbols.edit",
    "positions.view", "positions.forceClose", "risk.view", "lp.view", "lp.reconnect",
    "reports.view", "reports.export", "audit.view",
  ],
  risk: [
    "dashboard.view", "clients.view", "groups.view", "groups.edit", "symbols.view",
    "positions.view", "positions.forceClose", "risk.view", "risk.edit", "lp.view",
    "reports.view", "reports.export", "audit.view", "balance.approve",
  ],
  support: [
    "dashboard.view", "clients.view", "clients.edit", "balance.deposit", "balance.withdraw",
    "positions.view", "reports.view", "audit.view",
  ],
  readonly: VIEW_ALL.filter((p) => p !== "users.view" && p !== "settings.view" && p !== "partner.view"),
  partner: ["partner.view"],
};

export function can(role: Role, permission: Permission): boolean {
  return ROLE_PERMISSIONS[role].includes(permission);
}

/** Route → permission required to view it. Order: longest prefix wins. */
export const ROUTE_PERMISSIONS: Record<string, Permission> = {
  "/": "dashboard.view",
  "/clients": "clients.view",
  "/clients/statement": "clients.view",
  "/groups": "groups.view",
  "/rules": "groups.view",
  "/symbols": "symbols.view",
  "/positions": "positions.view",
  "/risk": "risk.view",
  "/lp": "lp.view",
  "/bridge": "lp.view",
  "/denetim": "lp.view",
  "/partner": "partner.view",
  "/reports": "reports.view",
  "/audit": "audit.view",
  "/users": "users.view",
  "/settings": "settings.view",
  "/approvals": "balance.approve",
};

export function normalizePath(pathname: string, basePath = ""): string {
  let p = pathname;
  if (basePath && p.startsWith(basePath)) p = p.slice(basePath.length);
  p = p.split(/[?#]/)[0] ?? "/";
  if (p.length > 1 && p.endsWith("/")) p = p.slice(0, -1);
  return p || "/";
}

export function permissionForRoute(pathname: string): Permission | null {
  const p = normalizePath(pathname);
  let best: string | null = null;
  for (const route of Object.keys(ROUTE_PERMISSIONS)) {
    const match = route === "/" ? p === "/" : p === route || p.startsWith(route + "/");
    if (match && (best === null || route.length > best.length)) best = route;
  }
  return best ? ROUTE_PERMISSIONS[best]! : null;
}

/** Unknown routes are denied (fail closed). */
export function canAccessRoute(role: Role, pathname: string): boolean {
  const perm = permissionForRoute(pathname);
  return perm !== null && can(role, perm);
}

/** Balance operations above this many minor units need a second approver (4-eyes). */
export const FOUR_EYES_THRESHOLD_MINOR = 1_000_000; // 10,000.00 in a 2-digit currency

export function balancePermission(type: "deposit" | "withdraw" | "credit"): Permission {
  return type === "deposit" ? "balance.deposit" : type === "withdraw" ? "balance.withdraw" : "balance.credit";
}

export function requiresSecondApproval(amountMinor: number): boolean {
  return Math.abs(amountMinor) >= FOUR_EYES_THRESHOLD_MINOR;
}
