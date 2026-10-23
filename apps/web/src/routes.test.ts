import { describe, expect, it } from "vitest";
import {
  adminUserHref,
  settingsEditorHref,
  projectSettingsHref,
  personalSettingsHref,
  agentHref,
  chatThreadHref,
  chatConnectionsHref,
  globalHref,
  hostHref,
  hostedAgentHref,
  parseRoute,
  projectHref,
  projectCreateHref,
  safeReturnPath,
  routeSurface,
  taskHref,
  threadHref,
} from "./routes";
const origin = "https://control.example";
describe("Control Plane permalinks", () => {
  it("round-trips major pages and encoded, exact scoped resource identifiers", () => {
    expect(parseRoute(projectCreateHref(), origin)).toEqual({
      kind: "project-create",
    });
    expect(parseRoute("/projects/new?project=other", origin).kind).toBe(
      "invalid",
    );
    expect(parseRoute(globalHref("fleet", "project:a"), origin)).toEqual({
      kind: "global",
      surface: "fleet",
      project: "project:a",
    });
    expect(
      parseRoute(globalHref("admin", undefined, "settings"), origin),
    ).toEqual({ kind: "global", surface: "admin", view: "settings" });
    expect(parseRoute(projectHref("project:a", "access"), origin)).toEqual({
      kind: "project",
      project: "project:a",
      view: "access",
    });
    expect(
      parseRoute(agentHref("project:a", "node:b", "policy", true), origin),
    ).toEqual({
      kind: "agent",
      project: "project:a",
      node: "node:b",
      view: "policy",
      compose: true,
    });
    expect(parseRoute(threadHref("project:a", "thread:c"), origin)).toEqual({
      kind: "thread",
      project: "project:a",
      thread: "thread:c",
    });
    expect(parseRoute(taskHref("project:a", "task:d"), origin)).toEqual({
      kind: "task",
      project: "project:a",
      task: "task:d",
    });
    expect(parseRoute(hostHref("project:a", "host:b"), origin)).toEqual({
      kind: "host",
      project: "project:a",
      host: "host:b",
    });
    expect(parseRoute(globalHref("hosted", "project:a"), origin)).toEqual({
      kind: "global",
      surface: "hosted",
      project: "project:a",
    });
    const hosted = parseRoute(hostedAgentHref("project:a", "chat:b"), origin);
    expect(hosted).toEqual({
      kind: "hosted-agent",
      project: "project:a",
      node: "chat:b",
    });
    expect(routeSurface(hosted)).toBe("hosted");
    expect(parseRoute(globalHref("chat", "project:a"), origin)).toEqual({
      kind: "global",
      surface: "chat",
      project: "project:a",
    });
    expect(
      parseRoute(chatThreadHref("project:a", "personal:c"), origin),
    ).toEqual({
      kind: "chat",
      project: "project:a",
      thread: "personal:c",
    });
    expect(parseRoute(chatConnectionsHref("project:a"), origin)).toEqual({
      kind: "chat-connections",
      project: "project:a",
    });
  });
  it("round-trips user profiles and their project-scoped inventories", () => {
    expect(parseRoute(adminUserHref("user:a"), origin)).toEqual({
      kind: "global",
      surface: "admin",
      view: "users",
      user: "user:a",
      userView: "overview",
    });
    for (const view of [
      "overview",
      "projects",
      "workspaces",
      "chats",
    ] as const) {
      const route = parseRoute(
        adminUserHref("user:a", view, "project:b"),
        origin,
      );
      expect(route).toEqual({
        kind: "global",
        surface: "admin",
        view: "users",
        user: "user:a",
        userView: view,
        project: "project:b",
      });
      expect(routeSurface(route)).toBe("admin");
    }
  });
  it("rejects malformed, ambiguous, oversized, and cross-origin destinations", () => {
    for (const href of [
      "https://other.example/fleet",
      "//other.example/fleet",
      "/projects/p/threads/%2Fsecret",
      "/projects/p/threads/%ZZ",
      "/projects/p/threads/",
      "/fleet?project=p&project=q",
      "/fleet?permissions=execute",
      "/chat?project=p&user=other",
      "/projects/p/chat/%2Fsecret",
      "/projects/p/chat/t?node_id=other",
      "/admin/users/%2Fsecret",
      "/admin/users/%ZZ/chats",
      "/admin/users/u/unknown",
      "/admin/users/u/chats?owner=other",
      "/admin/users/u/chats?project=p&project=q",
      "/admin/users/u/chats?project=%2Fsecret",
      "/hosted?owner=other",
      "/projects/p/hosted/%2Fsecret",
      "/projects/p/hosted/n?owner=other",
      "/projects/p/agents/n/policy?compose=0",
      `/projects/p/threads/${"x".repeat(257)}`,
      `/fleet?${"x".repeat(2048)}`,
    ]) {
      expect(parseRoute(href, origin).kind, href).toBe("invalid");
      expect(safeReturnPath(href, origin), href).toBeNull();
    }
    expect(
      safeReturnPath(`${origin}/projects/p/threads/t#ignored`, origin),
    ).toBe("/projects/p/threads/t");
  });
});

it("round-trips scoped resource editors without credentials in URLs", () => {
  for (const section of [
    "providers",
    "models",
    "mcp",
    "search",
    "credentials",
    "variables",
  ] as const) {
    for (const editor of [
      { action: "new" as const },
      { action: "edit" as const, id: "resource:a" },
    ]) {
      expect(
        parseRoute(
          settingsEditorHref(projectSettingsHref("project:a", section), editor),
          origin,
        ),
      ).toEqual({
        kind: "project-settings",
        project: "project:a",
        section,
        editor,
      });
      expect(
        parseRoute(
          settingsEditorHref(globalHref("admin", undefined, section), editor),
          origin,
        ),
      ).toEqual({ kind: "global", surface: "admin", view: section, editor });
    }
  }
  expect(
    parseRoute(
      settingsEditorHref(personalSettingsHref("credentials", "p"), {
        action: "new",
      }),
      origin,
    ),
  ).toEqual({
    kind: "global",
    surface: "settings",
    section: "credentials",
    project: "p",
    editor: { action: "new" },
  });
  expect(parseRoute("/admin/users/new", origin)).toEqual({
    kind: "global",
    surface: "admin",
    view: "users",
    editor: { action: "new" },
  });
  for (const section of ["routing", "search", "telemetry"])
    expect(parseRoute(`/admin/${section}/edit`, origin)).toMatchObject({
      editor: { action: "edit" },
    });
  for (const path of [
    "/admin/models/edit",
    "/admin/telemetry/new",
    "/admin/routing/x/edit",
    "/settings/appearance/new",
    "/admin/announcements/a/edit?token=secret",
    "/projects/p/settings/credentials/%2Fsecret/edit",
    "/admin/providers/a/edit/extra",
  ])
    expect(parseRoute(path, origin).kind, path).toBe("invalid");
});
it("retains a credential destination in an explicit project scope without accepting partial or misplaced targets", () => {
  const editor = {
    action: "new" as const,
    target: { kind: "mcp" as const, id: "docs:a" },
  };
  expect(
    parseRoute(
      settingsEditorHref(projectSettingsHref("p", "credentials"), editor),
      origin,
    ),
  ).toEqual({
    kind: "project-settings",
    project: "p",
    section: "credentials",
    editor,
  });
  expect(
    parseRoute(
      settingsEditorHref(personalSettingsHref("credentials", "p"), editor),
      origin,
    ),
  ).toEqual({
    kind: "global",
    surface: "settings",
    section: "credentials",
    project: "p",
    editor,
  });
  expect(
    parseRoute(
      settingsEditorHref(globalHref("admin", "p", "credentials"), editor),
      origin,
    ),
  ).toEqual({
    kind: "global",
    surface: "admin",
    view: "credentials",
    project: "p",
    editor,
  });
  for (const path of [
    "/settings/credentials/new?kind=mcp&connection=a",
    "/settings/credentials/new?project=p&kind=mcp",
    "/admin/credentials/new?project=p&connection=a",
    "/admin/credentials/new?project=p&kind=unknown&connection=a",
    "/admin/credentials/a/edit?project=p&kind=mcp&connection=a",
    "/projects/p/settings/credentials/new?kind=mcp&connection=%2Fsecret",
    "/projects/p/settings/providers/new?kind=mcp&connection=a",
  ])
    expect(parseRoute(path, origin).kind, path).toBe("invalid");
});
