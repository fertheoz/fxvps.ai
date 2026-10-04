import { describe, expect, it } from "vitest";
import { can, canAccessRoute, normalizePath, permissionForRoute, requiresSecondApproval, ROLES, ROLE_PERMISSIONS, ROUTE_PERMISSIONS } from "@/lib/rbac";
import { NAV } from "@/components/shell/nav";

describe("RBAC", () => {
  it("admin can do everything", () => {
    for (const p of ROLE_PERMISSIONS.admin) expect(can("admin", p)).toBe(true);
    for (const route of Object.keys(ROUTE_PERMISSIONS)) expect(canAccessRoute("admin", route)).toBe(true);
  });

  it("read-only cannot mutate anything", () => {
    for (const p of ROLE_PERMISSIONS.readonly) expect(p.endsWith(".view")).toBe(true);
    expect(can("readonly", "balance.deposit")).toBe(false);
    expect(can("readonly", "positions.forceClose")).toBe(false);
  });

  it("guards routes per role", () => {
    expect(canAccessRoute("support", "/clients")).toBe(true);
    expect(canAccessRoute("support", "/lp")).toBe(false);
    expect(canAccessRoute("support", "/risk")).toBe(false);
    expect(canAccessRoute("dealer", "/positions/")).toBe(true);
    expect(canAccessRoute("dealer", "/users")).toBe(false);
    expect(canAccessRoute("risk", "/settings")).toBe(false);
    expect(canAccessRoute("readonly", "/users")).toBe(false);
    expect(canAccessRoute("readonly", "/audit")).toBe(true);
  });

  it("fails closed for unknown routes and does not prefix-match siblings", () => {
    expect(permissionForRoute("/nope")).toBeNull();
    expect(canAccessRoute("admin", "/nope")).toBe(false);
    expect(permissionForRoute("/usersx")).toBeNull();
    expect(permissionForRoute("/clients/abc")).toBe("clients.view");
  });

  it("normalizes basePath, trailing slash and query", () => {
    expect(normalizePath("/fxvps/clients/?x=1", "/fxvps")).toBe("/clients");
    expect(normalizePath("/fxvps", "/fxvps")).toBe("/");
  });

  it("separates money duties: dealer cannot move money, support cannot force-close", () => {
    expect(can("dealer", "balance.deposit")).toBe(false);
    expect(can("support", "positions.forceClose")).toBe(false);
    expect(can("support", "balance.credit")).toBe(false);
    expect(can("risk", "balance.approve")).toBe(true);
  });

  it("every role can see the dashboard and every nav item is mapped", () => {
    for (const r of ROLES) expect(canAccessRoute(r, "/")).toBe(true);
    for (const n of NAV) expect(permissionForRoute(n.href)).not.toBeNull();
  });

  it("4-eyes threshold", () => {
    expect(requiresSecondApproval(999_999)).toBe(false);
    expect(requiresSecondApproval(1_000_000)).toBe(true);
  });
});
