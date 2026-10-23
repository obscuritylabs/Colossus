// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ControlPlaneFrame, type ControlPlaneNavigation } from "@colossus/ui";

const groups = ["Project Administration", "Control Plane Admin"];
const navigation: ControlPlaneNavigation[] = [
  { id: "home", group: "Workspace", label: "Home", href: "/" },
  {
    id: "members",
    group: "Project Administration",
    label: "Members",
    href: "/projects/p/access",
  },
  {
    id: "users",
    group: "Control Plane Admin",
    label: "Users",
    href: "/admin/users",
  },
  {
    id: "announcements",
    group: "Control Plane Admin",
    label: "Announcements",
    href: "/admin/announcements",
  },
  {
    id: "settings",
    group: "Personal",
    label: "Settings",
    href: "/settings",
  },
].map((item) => ({ ...item, icon: <svg aria-hidden="true" /> }));

let root: Root, container: HTMLDivElement;
const onNavigate = vi.fn();
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  onNavigate.mockReset();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
});

async function render(current = "home", collapsible?: readonly string[]) {
  await act(() =>
    root.render(
      <ControlPlaneFrame
        current={current}
        navigation={navigation}
        onNavigate={onNavigate}
        {...(collapsible ? { collapsibleNavigationGroups: collapsible } : {})}
      >
        <p>Selected workspace content</p>
      </ControlPlaneFrame>,
    ),
  );
}
function disclosure(name: string) {
  const button = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${name}"]`,
  );
  if (!button) throw new Error(`Missing disclosure ${name}`);
  return button;
}
function childrenOf(button: HTMLButtonElement) {
  const id = button.getAttribute("aria-controls");
  const children = id ? document.getElementById(id) : null;
  if (!children) throw new Error("Disclosure must control its existing list");
  return children;
}

it("keeps existing navigation groups flat unless the host opts into disclosure", async () => {
  await render();
  expect(
    container.querySelectorAll(".control-navigation-disclosure"),
  ).toHaveLength(0);
  expect(
    container.querySelectorAll(".control-navigation-children"),
  ).toHaveLength(4);
  for (const list of container.querySelectorAll<HTMLElement>(
    ".control-navigation-children",
  )) {
    expect(list.hidden).toBe(false);
  }
  expect(
    container.querySelector('a[aria-label="Users"]')?.getAttribute("href"),
  ).toBe("/admin/users");
});

it("defaults opted-in groups closed and reveals a newly active child without defeating manual collapse", async () => {
  await render("home", groups);
  const admin = disclosure("Control Plane Admin");
  const project = disclosure("Project Administration");
  expect(admin.type).toBe("button");
  expect(admin.getAttribute("aria-expanded")).toBe("false");
  expect(childrenOf(admin).hidden).toBe(true);
  expect(childrenOf(project).hidden).toBe(true);
  await act(() => admin.click());
  expect(admin.getAttribute("aria-expanded")).toBe("true");
  expect(childrenOf(admin).hidden).toBe(false);
  await act(() => admin.click());
  await render("users", groups);
  expect(admin.getAttribute("aria-expanded")).toBe("true");
  expect(
    childrenOf(admin)
      .querySelector('a[aria-label="Users"]')
      ?.getAttribute("aria-current"),
  ).toBe("page");
  await act(() => admin.click());
  await render("users", groups);
  expect(admin.getAttribute("aria-expanded")).toBe("false");
  await render("announcements", groups);
  expect(admin.getAttribute("aria-expanded")).toBe("true");
  expect(childrenOf(project).hidden).toBe(true);
  expect(onNavigate).not.toHaveBeenCalled();
});

it("retains a labeled group control in the icon rail and expands the sidebar and group together", async () => {
  await render("home", groups);
  await act(() => disclosure("Toggle app sidebar").click());
  const sidebar = container.querySelector(".control-app-sidebar")!;
  const admin = disclosure("Control Plane Admin");
  expect(sidebar.getAttribute("data-collapsible")).toBe("icon");
  expect(admin.getAttribute("title")).toBe("Control Plane Admin");
  expect(admin.getAttribute("aria-expanded")).toBe("false");
  expect(childrenOf(admin).hidden).toBe(true);
  admin.focus();
  await act(() => admin.click());
  expect(sidebar.getAttribute("data-collapsible")).toBe("");
  expect(admin.getAttribute("aria-expanded")).toBe("true");
  expect(childrenOf(admin).hidden).toBe(false);
  expect(document.activeElement).toBe(admin);
  expect(onNavigate).not.toHaveBeenCalled();
});
