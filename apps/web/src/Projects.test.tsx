// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Projects } from "./Projects";
import type { Me, Project } from "./control-api";

const project = (
  id: string,
  name: string,
  parent: string | null = null,
  archived = false,
): Project => ({
  id,
  name,
  parent_project_id: parent,
  archived,
  description: `${name} description`,
  revision: 1,
  created_at: "2026-10-07T10:00:00Z",
  updated_at: "2026-10-07T10:00:00Z",
});
const projects = [
  project("parent", "Alpha"),
  project("child", "Beta", "parent"),
  project("old", "Archived acceptance", null, true),
];
const me: Me = {
  user: {
    id: "user",
    display_name: "Operator",
    email: null,
    active: true,
    is_admin: true,
    revision: 1,
    created_at: "",
    updated_at: "",
  },
  projects,
  memberships: [
    {
      user_id: "user",
      subject: "user",
      project_id: "child",
      role: "viewer",
      permissions: ["read"],
    },
  ],
};
let container: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => {
  await act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});
async function render(selected = "", user = me) {
  await act(async () =>
    root.render(
      <Projects
        me={user}
        selected={selected}
        onSelect={() => {}}
        permissions={["read"]}
        onRefresh={() => {}}
        tasks={<div>Project tasks fixture</div>}
        view="tasks"
      />,
    ),
  );
}
it("shows an active searchable project directory without a selection placeholder or backend calls", async () => {
  const fetch = vi.fn();
  vi.stubGlobal("fetch", fetch);
  await render();
  expect(container.querySelector("h1")?.textContent).toBe("Projects");
  expect(
    container.querySelector('table[aria-label="Projects"]'),
  ).not.toBeNull();
  expect(container.textContent).toContain("Alpha");
  expect(container.textContent).toContain("Beta");
  expect(container.textContent).not.toContain("Archived acceptance");
  expect(container.textContent).not.toContain("Select a project");
  expect(container.textContent).toContain("Administrator visibility");
  expect(container.textContent).toContain("Viewer");
  expect(
    container.querySelector('a[href="/projects/child/overview"]'),
  ).not.toBeNull();
  expect(fetch).not.toHaveBeenCalled();
  const search = container.querySelector<HTMLInputElement>(
    'input[aria-label="Search projects"]',
  )!;
  await act(() => {
    Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )!.set!.call(search, "Beta");
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(container.querySelector("tbody")?.textContent).toContain("Beta");
  expect(
    container
      .querySelector("tbody")
      ?.querySelector('a[href="/projects/parent/overview"]'),
  ).not.toBeNull();
  expect(container.querySelectorAll("tbody tr")).toHaveLength(1);
});
it("makes archived projects an explicit filter instead of mixing them into the default directory", async () => {
  await render();
  await act(() =>
    container
      .querySelector<HTMLButtonElement>(
        '[role="combobox"][aria-label="Project status"]',
      )!
      .click(),
  );
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((entry) => entry.textContent?.trim() === "Archived projects")!;
  await act(() => option.click());
  expect(container.querySelector("tbody")?.textContent).toContain(
    "Archived acceptance",
  );
  expect(container.querySelector("tbody")?.textContent).not.toContain("Beta");
});
it("shows selected project context and existing content without repeating the directory", async () => {
  await render("child");
  expect(container.querySelector("h1")?.textContent).toBe("Beta");
  expect(container.querySelector('.back[href="/projects"]')).not.toBeNull();
  expect(
    container.querySelector('[aria-label="Project directory"]'),
  ).toBeNull();
  expect(container.textContent).toContain("Within Alpha");
  expect(container.textContent).toContain("Project tasks fixture");
  expect(
    container.querySelector('a[href="/projects/child/access"]'),
  ).not.toBeNull();
});
it("keeps project creation admin-only and gives a concise empty directory state", async () => {
  await render("", {
    ...me,
    user: { ...me.user, is_admin: false },
    projects: [],
  });
  expect(container.textContent).toContain("No visible projects");
  expect(container.textContent).not.toContain("New project");
  expect(container.textContent).not.toContain("Select a project");
});
