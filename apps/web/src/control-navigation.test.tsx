import { expect, it } from "vitest";
import { controlNavigation } from "./control-navigation";
import type { Me } from "./control-api";
import type { Route } from "./routes";

const timestamp = "2026-10-22T10:00:00Z";
function identity(globalAdmin = false, projectAdmin = false): Me {
  return {
    user: {
      id: "navigation-user",
      display_name: "Navigation operator",
      email: null,
      active: true,
      is_admin: globalAdmin,
      revision: 1,
      created_at: timestamp,
      updated_at: timestamp,
    },
    memberships: [
      {
        project_id: "p",
        user_id: "navigation-user",
        subject: "navigation-user",
        role: projectAdmin ? "project_admin" : "operator",
        permissions: projectAdmin
          ? ["read", "execute", "control", "administer"]
          : ["read", "execute", "control"],
      },
      {
        project_id: "q",
        user_id: "navigation-user",
        subject: "navigation-user",
        role: "viewer",
        permissions: ["read"],
      },
    ],
    projects: ["p", "q"].map((id) => ({
      id,
      name: `Project ${id}`,
      description: "",
      parent_project_id: null,
      archived: false,
      revision: 1,
      created_at: timestamp,
      updated_at: timestamp,
    })),
  };
}
const home: Route = { kind: "global", surface: "home", project: "p" };
const projectLabels = [
  "Members",
  "Providers",
  "Models",
  "MCP servers",
  "Model routing",
  "Search",
  "Telemetry",
  "Secrets",
  "Variables",
  "General",
];

it("keeps workspace browsing and personal settings separate from administrator navigation", () => {
  const { items } = controlNavigation(identity(), home, "p");
  expect(
    items
      .filter((item) => item.group === "Workspace")
      .map((item) => item.label),
  ).toEqual(["Home", "Chat", "Hosted", "Fleet", "Projects"]);
  expect(
    items
      .filter((item) => item.group === "Personal")
      .map((item) => [item.label, item.href]),
  ).toEqual([["Settings", "/settings/appearance?project=p"]]);
  expect(items.some((item) => item.group?.includes("Admin"))).toBe(false);
  expect(items.find((item) => item.id === "projects")?.href).toBe("/projects");
});

it("shows project administration only for the selected authorized project", () => {
  const account = identity(false, true);
  const { items } = controlNavigation(account, home, "p");
  expect(
    items
      .filter((item) => item.group === "Project Administration")
      .map((item) => item.label),
  ).toEqual(projectLabels);
  expect(items.find((item) => item.id === "project-members")?.href).toBe(
    "/projects/p/access",
  );
  expect(items.find((item) => item.id === "project-providers")?.href).toBe(
    "/projects/p/settings/providers",
  );
  for (const project of ["q", "unavailable"]) {
    expect(
      controlNavigation(account, home, project).items.some(
        (item) => item.group === "Project Administration",
      ),
    ).toBe(false);
  }
  expect(items.some((item) => item.group === "Control Plane Admin")).toBe(
    false,
  );
});

it("keeps Control Plane administration available without manufacturing a selected project", () => {
  const account = { ...identity(true), projects: [], memberships: [] };
  const { items, current } = controlNavigation(
    account,
    { kind: "global", surface: "admin", view: "users" },
    "",
  );
  expect(
    items
      .filter((item) => item.group === "Control Plane Admin")
      .map((item) => item.label),
  ).toEqual([
    "Users",
    "Projects",
    "Providers",
    "Models",
    "MCP servers",
    "Model routing",
    "Search",
    "Telemetry",
    "Secrets",
    "Variables",
    "Announcements",
    "Appearance",
    "Authentication & settings",
  ]);
  expect(items.find((item) => item.id === "admin-users")?.href).toBe(
    "/admin/users",
  );
  expect(items.find((item) => item.id === "admin-projects")?.href).toBe(
    "/admin/projects",
  );
  expect(items.find((item) => item.id === "settings")?.href).toBe(
    "/settings/appearance",
  );
  expect(items.some((item) => item.group === "Project Administration")).toBe(
    false,
  );
  expect(current).toBe("admin-users");
});

it("distinguishes active global, project and personal settings destinations", () => {
  const account = identity(true, true);
  const cases: [Route, string][] = [
    [
      { kind: "global", surface: "admin", view: "providers" },
      "admin-providers",
    ],
    [
      { kind: "project-settings", project: "p", section: "providers" },
      "project-providers",
    ],
    [{ kind: "project", project: "p", view: "access" }, "project-members"],
    [{ kind: "project", project: "p", view: "settings" }, "project-settings"],
    [
      {
        kind: "global",
        surface: "settings",
        section: "appearance",
        project: "p",
      },
      "settings",
    ],
  ];
  for (const [route, current] of cases) {
    expect(controlNavigation(account, route, "p").current).toBe(current);
  }
});
