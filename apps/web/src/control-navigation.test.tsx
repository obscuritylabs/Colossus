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
  "Network access",
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
      .filter((item) => item.group === "Administration")
      .map((item) => item.label),
  ).toEqual(["Project Admin"]);
  expect(items.find((item) => item.id === "project-admin")?.href).toBe(
    "/projects/p/access",
  );
  expect(items).toHaveLength(7);
  expect(controlNavigation(account, home, "p").secondary).toBeUndefined();
  for (const project of ["q", "unavailable"]) {
    expect(
      controlNavigation(account, home, project).items.some(
        (item) => item.id === "project-admin",
      ),
    ).toBe(false);
  }
  expect(items.some((item) => item.id === "admin")).toBe(false);
});

it("keeps Control Plane administration available without manufacturing a selected project", () => {
  const account = { ...identity(true), projects: [], memberships: [] };
  const { items, current, secondary } = controlNavigation(
    account,
    { kind: "global", surface: "admin", view: "users" },
    "",
  );
  expect(
    items
      .filter((item) => item.group === "Administration")
      .map((item) => item.label),
  ).toEqual(["Control Plane Admin"]);
  expect(secondary?.items.map((item) => item.label)).toEqual([
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
  expect(secondary?.items.find((item) => item.id === "admin-users")?.href).toBe(
    "/admin/users",
  );
  expect(
    secondary?.items.find((item) => item.id === "admin-projects")?.href,
  ).toBe("/admin/projects");
  expect(items.find((item) => item.id === "settings")?.href).toBe(
    "/settings/appearance",
  );
  expect(items.some((item) => item.id === "project-admin")).toBe(false);
  expect(current).toBe("admin");
  expect(secondary?.current).toBe("admin-users");
});

it("distinguishes active global, project and personal settings destinations", () => {
  const account = identity(true, true);
  const cases: [Route, string, string | undefined][] = [
    [
      { kind: "global", surface: "admin", view: "providers" },
      "admin",
      "admin-providers",
    ],
    [
      { kind: "project-settings", project: "p", section: "providers" },
      "project-admin",
      "project-providers",
    ],
    [
      { kind: "project-settings", project: "p", section: "network" },
      "project-admin",
      "project-network",
    ],
    [
      { kind: "project", project: "p", view: "access" },
      "project-admin",
      "project-members",
    ],
    [
      { kind: "project", project: "p", view: "settings" },
      "project-admin",
      "project-settings",
    ],
    [
      {
        kind: "global",
        surface: "settings",
        section: "appearance",
        project: "p",
      },
      "settings",
      undefined,
    ],
  ];
  for (const [route, current, secondaryCurrent] of cases) {
    const navigation = controlNavigation(account, route, "p");
    expect(navigation.current).toBe(current);
    expect(navigation.secondary?.current).toBe(secondaryCurrent);
  }
});

it("limits the primary rail to two administration destinations and moves settings into the authorized scope", () => {
  const account = identity(true, true);
  const navigation = controlNavigation(
    account,
    { kind: "project-settings", project: "p", section: "models" },
    "p",
  );
  expect(navigation.items).toHaveLength(8);
  expect(
    navigation.items
      .filter((item) => item.group === "Administration")
      .map((item) => item.id),
  ).toEqual(["project-admin", "admin"]);
  expect(
    navigation.items.some(
      (item) => item.id === "project-models" || item.id === "admin-models",
    ),
  ).toBe(false);
  expect(navigation.secondary?.items.map((item) => item.label)).toEqual(
    projectLabels,
  );
  expect(navigation.secondary?.context).toBe("Project p");
  expect(
    navigation.secondary?.items.find((item) => item.id === "project-models")
      ?.href,
  ).toBe("/projects/p/settings/models");
  expect(
    controlNavigation(
      identity(false, true),
      { kind: "project-settings", project: "q", section: "models" },
      "q",
    ).secondary,
  ).toBeUndefined();
  expect(
    controlNavigation(
      account,
      { kind: "project-settings", project: "q", section: "models" },
      "p",
    ).secondary,
  ).toBeUndefined();
  expect(
    controlNavigation(
      identity(),
      { kind: "global", surface: "admin", view: "users" },
      "p",
    ).secondary,
  ).toBeUndefined();
});
