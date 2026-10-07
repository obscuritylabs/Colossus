import { expect, it } from "vitest";
import { projectPermissions, type Me } from "./control-api";
it("unions administrator management rights with explicit membership and strips archived operation rights", () => {
  const me = {
    user: { is_admin: true },
    projects: [{ id: "project", archived: false }],
    memberships: [{ project_id: "project", permissions: ["read"] }],
  } as Me;
  expect(new Set(projectPermissions(me, "project"))).toEqual(
    new Set(["read", "administer"]),
  );
  me.memberships[0]!.permissions = ["read", "execute", "control"];
  expect(new Set(projectPermissions(me, "project"))).toEqual(
    new Set(["read", "execute", "control", "administer"]),
  );
  expect(projectPermissions(me, "project")).not.toContain("approve");
  me.memberships[0]!.permissions = ["read", "control", "approve"];
  expect(projectPermissions(me, "project")).not.toContain("execute");
  expect(projectPermissions(me, "project")).toContain("administer");
  me.projects[0]!.archived = true;
  expect(new Set(projectPermissions(me, "project"))).toEqual(
    new Set(["read", "administer"]),
  );
});
