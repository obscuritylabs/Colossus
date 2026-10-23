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
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(() => true),
  }));
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  onNavigate.mockReset();
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
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

const administration: ControlPlaneNavigation[] = [
  {
    id: "project-admin",
    group: "Administration",
    label: "Project Admin",
    href: "/projects/p/access",
    icon: <svg aria-hidden="true" />,
  },
  {
    id: "admin",
    group: "Administration",
    label: "Control Plane Admin",
    href: "/admin/users",
    icon: <svg aria-hidden="true" />,
  },
];
const sections: ControlPlaneNavigation[] = [
  {
    id: "admin-users",
    label: "Users",
    href: "/admin/users",
    icon: <svg aria-hidden="true" />,
  },
  {
    id: "admin-models",
    label: "Models",
    href: "/admin/models",
    icon: <svg aria-hidden="true" />,
  },
];
async function renderAdministration(scope = "control-plane") {
  await act(() =>
    root.render(
      <ControlPlaneFrame
        current="admin"
        navigation={administration}
        onNavigate={onNavigate}
        sidebar={<p>Conversation list must not duplicate administration</p>}
        secondaryNavigation={{
          id: scope,
          title: "Control Plane Admin",
          context: "Instance-wide settings",
          navigation: sections,
          current: "admin-models",
          onNavigate,
        }}
      >
        <h1>Models</h1>
      </ControlPlaneFrame>,
    ),
  );
}

it("keeps administration sections in one scoped sidebar while the compact primary rail has only scope destinations", async () => {
  await renderAdministration();
  const primary = container.querySelector('nav[aria-label="Application"]')!;
  const secondary = container.querySelector(
    'aside[aria-label="Control Plane Admin navigation"]',
  )!;
  expect(primary.querySelectorAll("a")).toHaveLength(2);
  expect(primary.textContent).not.toContain("Models");
  expect(secondary.textContent).toContain("Instance-wide settings");
  expect(container.textContent).not.toContain(
    "Conversation list must not duplicate administration",
  );
  expect(secondary.querySelector('a[aria-current="page"]')?.textContent).toBe(
    "Models",
  );
  expect(primary.querySelector('a[aria-current="page"]')?.textContent).toBe(
    "Control Plane Admin",
  );
  await act(() => disclosure("Toggle app sidebar").click());
  expect(
    container
      .querySelector(".control-app-sidebar")
      ?.getAttribute("data-collapsible"),
  ).toBe("icon");
  expect(primary.querySelectorAll("a")).toHaveLength(2);
  expect(secondary.querySelectorAll("a")).toHaveLength(2);
  const breadcrumb = container.querySelector('nav[aria-label="Breadcrumb"]')!;
  expect(breadcrumb.querySelector("a")?.getAttribute("href")).toBe(
    "/admin/users",
  );
  expect(breadcrumb.querySelector('[aria-current="page"]')?.textContent).toBe(
    "Models",
  );
});

it("preserves section URLs and modified clicks and restores a hidden secondary sidebar when its scope changes", async () => {
  await renderAdministration();
  const users = container.querySelector<HTMLAnchorElement>(
    '.control-secondary-link[href="/admin/users"]',
  )!;
  const modified = new MouseEvent("click", {
    bubbles: true,
    cancelable: true,
    ctrlKey: true,
  });
  await act(() => users.dispatchEvent(modified));
  expect(modified.defaultPrevented).toBe(false);
  expect(onNavigate).not.toHaveBeenCalled();
  await act(() => users.click());
  expect(onNavigate).toHaveBeenCalledWith("admin-users");
  const toggle = disclosure("Toggle administration navigation");
  await act(() => toggle.click());
  expect(toggle.getAttribute("aria-expanded")).toBe("false");
  expect(
    container.querySelector<HTMLElement>("#control-administration-navigation")
      ?.hidden,
  ).toBe(true);
  await renderAdministration("project:p");
  expect(toggle.getAttribute("aria-expanded")).toBe("true");
  expect(
    container.querySelector<HTMLElement>("#control-administration-navigation")
      ?.hidden,
  ).toBe(false);
});

it("uses a labeled modal navigation sheet at compact widths and restores keyboard focus when dismissed", async () => {
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: query.includes("1100"),
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(() => true),
  }));
  await renderAdministration();
  const toggle = disclosure("Toggle administration navigation");
  expect(
    container.querySelector("#control-administration-navigation"),
  ).toBeNull();
  toggle.focus();
  await act(() => toggle.click());
  const sheet = document.querySelector<HTMLElement>(
    '[data-slot="sheet-content"]',
  )!;
  expect(sheet.getAttribute("role")).toBe("dialog");
  expect(sheet.textContent).toContain("Control Plane Admin navigation");
  expect(
    sheet.querySelector('nav[aria-label="Control Plane Admin sections"]'),
  ).not.toBeNull();
  const close = sheet.querySelector<HTMLButtonElement>(
    ".ui-sheet-close-button",
  )!;
  await act(async () => {
    close.click();
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  expect(document.querySelector('[data-slot="sheet-content"]')).toBeNull();
  expect(document.activeElement).toBe(toggle);
});
