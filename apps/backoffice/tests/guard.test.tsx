// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

vi.mock("next/navigation", () => ({ usePathname: () => "/", useRouter: () => ({ push: vi.fn() }) }));
import { RouteGuard } from "@/components/shell/app-shell";

afterEach(cleanup);

describe("<RouteGuard>", () => {
  it("renders children when the role may view the route", () => {
    render(<RouteGuard role="support" pathname="/clients"><p>secret clients</p></RouteGuard>);
    expect(screen.getByText("secret clients")).toBeTruthy();
  });
  it("blocks and shows access denied otherwise", () => {
    render(<RouteGuard role="support" pathname="/lp"><p>secret lp</p></RouteGuard>);
    expect(screen.queryByText("secret lp")).toBeNull();
    expect(screen.getByTestId("access-denied")).toBeTruthy();
  });
  it("blocks read-only from users admin", () => {
    render(<RouteGuard role="readonly" pathname="/users/"><p>user admin</p></RouteGuard>);
    expect(screen.queryByText("user admin")).toBeNull();
  });
});
