import { expect, test, type Page } from "@playwright/test";
import type {
  AdminSettings,
  AuthConfig,
  Member,
  PolicyBaseline,
  Project,
  User,
} from "../../src/control-api";
import { appearanceCases, selectAppearance } from "./appearance";
import { browserFixture } from "./fixture";

const projectId = "browser-project";
const projectPath = "/api/projects/" + projectId;
const timestamp = "2026-10-23T10:00:00Z";
// These workflows span several full pages; individual UI assertions retain their normal deadlines.
test.setTimeout(90_000);

/** Closed synthetic administration API; never uses the user's controller. */
async function administrationFixture(
  page: Page,
  authConfig: AuthConfig = { local_enabled: true, oidc: null },
) {
  const base = await browserFixture(page, {
    administer: true,
    globalAdmin: true,
  });
  let project: Project = {
    id: projectId,
    name: "Browser test project",
    description: "Settings acceptance",
    parent_project_id: null,
    archived: false,
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
  };
  let user: User = {
    id: "matrix-user",
    display_name: "Matrix operator",
    email: "operator@example.test",
    active: true,
    is_admin: false,
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
    identities: [
      { kind: "local", label: "Local", username: "matrix-operator" },
    ],
  };
  let settings: AdminSettings = {
    revision: 1,
    auth: {
      local_enabled: authConfig.local_enabled,
      oidc_label: authConfig.oidc?.label ?? null,
    },
    classification: {
      enabled: false,
      text: "",
      tone: "neutral",
      position: "top",
    },
  };
  let policy: PolicyBaseline = {
    revision: 1,
    required_sandbox_profile: null,
    allowed_approval_modes: [],
    allowed_tools: null,
  };
  let members: Member[] = [];
  const writes: {
    path: string;
    method: string;
    body: Record<string, unknown>;
  }[] = [];
  await page.route("**/api/**", async (route) => {
    const request = route.request(),
      url = new URL(request.url()),
      path = url.pathname;
    if (request.method() === "GET") {
      if (path === "/api/auth/config")
        return route.fulfill({ json: authConfig });
      if (path === "/api/me")
        return route.fulfill({
          json: {
            user: {
              ...user,
              id: "browser-user",
              display_name: "Browser Test Operator",
              active: true,
              is_admin: true,
            },
            projects: [project],
            memberships: [
              {
                project_id: projectId,
                user_id: "browser-user",
                subject: "browser-user",
                role: "project_admin",
                permissions: ["read", "execute", "control", "administer"],
              },
            ],
          },
        });
      if (path === "/api/admin/users")
        return route.fulfill({ json: { users: [user], next_cursor: null } });
      if (path === "/api/admin/users/" + user.id)
        return route.fulfill({ json: { user } });
      if (path === "/api/admin/users/" + user.id + "/projects")
        return route.fulfill({ json: { projects: [], next_cursor: null } });
      if (path === "/api/admin/projects")
        return route.fulfill({
          json: { projects: [project], next_cursor: null },
        });
      if (path === "/api/admin/settings")
        return route.fulfill({ json: settings });
      if (path === "/api/settings")
        return route.fulfill({
          json: { classification: settings.classification },
        });
      if (path === projectPath + "/members")
        return route.fulfill({ json: { members, next_cursor: null } });
      if (path === projectPath + "/member-candidates")
        return route.fulfill({ json: { users: [user] } });
      if (path === projectPath + "/policy")
        return route.fulfill({ json: policy });
      return route.fallback();
    }
    if (
      ![
        "/api/admin/users",
        "/api/admin/users/" + user.id,
        "/api/admin/settings",
        projectPath,
        projectPath + "/members",
        projectPath + "/members/" + user.id,
        projectPath + "/policy",
      ].includes(path)
    )
      return route.fallback();
    expect(request.headers()["x-colossus-csrf"]).toBe("1");
    const body = request.method() === "DELETE" ? {} : request.postDataJSON();
    writes.push({ path, method: request.method(), body });
    if (path === "/api/admin/users") {
      expect(request.method()).toBe("POST");
      user = {
        ...user,
        display_name: body.display_name,
        email: body.email,
        is_admin: body.is_admin,
        identities: [
          ...(body.username
            ? [
                {
                  kind: "local" as const,
                  label: "Local",
                  username: body.username,
                },
              ]
            : []),
          ...(body.oidc_subject
            ? [
                {
                  kind: "oidc" as const,
                  label: authConfig.oidc!.label,
                  issuer: "https://identity.example.test",
                  subject: body.oidc_subject,
                },
              ]
            : []),
        ],
      };
      return route.fulfill({ json: { user } });
    }
    if (path === "/api/admin/users/" + user.id) {
      expect(request.method()).toBe("PATCH");
      expect(body.revision).toBe(user.revision);
      user = { ...user, ...body, revision: user.revision + 1 };
      return route.fulfill({ json: { user } });
    }
    if (path === "/api/admin/settings") {
      expect(request.method()).toBe("PATCH");
      expect(body.revision).toBe(settings.revision);
      expect(body).not.toHaveProperty("auth");
      settings = {
        ...settings,
        classification: body.classification,
        revision: settings.revision + 1,
      };
      return route.fulfill({ json: settings });
    }
    if (path === projectPath) {
      expect(request.method()).toBe("PATCH");
      expect(body.revision).toBe(project.revision);
      project = { ...project, ...body, revision: project.revision + 1 };
      return route.fulfill({ json: { project } });
    }
    if (path === projectPath + "/policy") {
      expect(request.method()).toBe("PATCH");
      expect(body.revision).toBe(policy.revision);
      policy = { ...body, revision: policy.revision + 1 };
      return route.fulfill({ json: policy });
    }
    if (request.method() === "DELETE") {
      expect(url.searchParams.get("revision")).toBe(
        String(members[0]!.revision),
      );
      members = [];
      return route.fulfill({ json: {} });
    }
    if (request.method() === "PATCH")
      expect(body.revision).toBe(members[0]!.revision);
    members = [
      {
        user_id: user.id,
        project_id: projectId,
        display_name: user.display_name,
        role: body.role,
        permissions: ["read"],
        revision: (members[0]?.revision ?? 0) + 1,
      },
    ];
    return route.fulfill({ json: { member: members[0] } });
  });
  return { ...base, writes };
}
async function choose(page: Page, name: string, option: string) {
  await page.getByRole("combobox", { name, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

for (const kind of ["local", "oidc", "both"] as const) {
  test(
    "user boundary acceptance preserves full metadata with " +
      kind +
      " identities",
    async ({ page }) => {
      const fixture = await administrationFixture(page, {
        local_enabled: kind !== "oidc",
        oidc:
          kind !== "local"
            ? { label: "Organization", login_url: "/api/auth/oidc/login" }
            : null,
      });
      const name = "n".repeat(256);
      const subject = " " + "é".repeat(127) + " ";
      const username = "u".repeat(128);
      const password = "🙂".repeat(15);
      await page.goto("/admin/users/new");
      const nameField = page.getByRole("textbox", {
        name: "Display name",
        exact: true,
      });
      await nameField.fill(name);
      await expect(nameField).toHaveValue(name);
      if (kind !== "oidc") {
        const usernameField = page.getByRole("textbox", {
          name: "Local username",
          exact: true,
        });
        await usernameField.fill(username);
        await expect(usernameField).toHaveValue(username);
        await page.getByLabel("Local password", { exact: true }).fill(password);
      } else
        await expect(
          page.getByLabel("Local password", { exact: true }),
        ).toHaveCount(0);
      if (kind !== "local") {
        const subjectField = page.getByRole("textbox", {
          name: "OIDC subject",
          exact: true,
        });
        await subjectField.fill(subject);
        await expect(subjectField).toHaveValue(subject);
      } else
        await expect(
          page.getByRole("textbox", { name: "OIDC subject", exact: true }),
        ).toHaveCount(0);
      await page
        .getByRole("button", { name: "Save user", exact: true })
        .click();
      await expect(page).toHaveURL(/\/admin\/users$/u);
      expect(fixture.writes).toHaveLength(1);
      const body = fixture.writes[0]!.body;
      expect(body.display_name).toBe(name);
      if (kind === "oidc") {
        expect(body).not.toHaveProperty("username");
        expect(body).not.toHaveProperty("password");
      } else expect(body).toMatchObject({ username, password });
      if (kind === "local") expect(body).not.toHaveProperty("oidc_subject");
      else expect(body.oidc_subject).toBe(subject);
      fixture.verify();
    },
  );
}

test("project boundary acceptance preserves the full name and description", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  const name = "n".repeat(256),
    description = "d".repeat(4096);
  await page.goto("/projects/" + projectId + "/settings");
  const form = page.getByRole("form", { name: "Edit project", exact: true });
  const nameField = form.getByRole("textbox", { name: "Name", exact: true });
  const descriptionField = form.getByRole("textbox", {
    name: "Description",
    exact: true,
  });
  await nameField.fill(name);
  await descriptionField.fill(description);
  await expect(nameField).toHaveValue(name);
  await expect(descriptionField).toHaveValue(description);
  await form.getByRole("button", { name: "Save project", exact: true }).click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]!.body).toMatchObject({
    revision: 1,
    name,
    description,
  });
  await page.reload();
  await expect(nameField).toHaveValue(name);
  await expect(descriptionField).toHaveValue(description);
  fixture.verify();
});

test("classification boundary acceptance preserves 160 Unicode characters", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  const text = "🙂".repeat(160);
  await page.goto("/admin/settings");
  await page
    .getByRole("checkbox", {
      name: "Display classification banner",
      exact: true,
    })
    .check();
  const field = page.getByRole("textbox", {
    name: "Classification text",
    exact: true,
  });
  await field.fill(text);
  await expect(field).toHaveValue(text);
  await page
    .getByRole("button", { name: "Save environment settings", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]!.body.classification).toMatchObject({
    enabled: true,
    text,
  });
  await page.reload();
  await expect(field).toHaveValue(text);
  fixture.verify();
});

test("global user creation and disabling use dedicated URLs and reviewed revisions", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/users/new");
  await page
    .getByRole("textbox", { name: "Display name", exact: true })
    .fill("New settings operator");
  await page
    .getByRole("textbox", { name: "Email · optional", exact: true })
    .fill("new@example.test");
  await page
    .getByRole("textbox", { name: "Local username", exact: true })
    .fill("new-settings-user");
  await page
    .getByLabel("Local password", { exact: true })
    .fill("Synthetic-only-password-2026");
  await page
    .getByRole("button", { name: "Save user", exact: true })
    .press("Enter");
  await expect(page).toHaveURL(/\/admin\/users$/u);
  expect(fixture.writes[0]).toMatchObject({
    path: "/api/admin/users",
    method: "POST",
    body: {
      display_name: "New settings operator",
      username: "new-settings-user",
      is_admin: false,
    },
  });
  await expect(
    page.getByRole("link", { name: "New settings operator", exact: true }),
  ).toBeVisible();
  await page.goto("/admin/users/matrix-user/edit");
  await expect(
    page.getByRole("textbox", { name: "Display name", exact: true }),
  ).toHaveValue("New settings operator");
  await page.getByRole("checkbox", { name: /^Active user/u }).uncheck();
  await page.getByRole("button", { name: "Save user", exact: true }).click();
  await expect(page).toHaveURL(/\/admin\/users\/matrix-user$/u);
  expect(fixture.writes[1]).toMatchObject({
    method: "PATCH",
    body: { revision: 1, active: false, is_admin: false },
  });
  await expect(page.getByText("Disabled", { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByText("Disabled", { exact: true })).toBeVisible();
  await expect(page.locator('input[type="password"]')).toHaveCount(0);
  fixture.verify();
});

test("authentication summary is read-only and classification markings survive reload", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/settings");
  await expect(
    page.getByRole("heading", { name: "Authentication", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Authentication providers are configured by the deployment operator.",
      { exact: false },
    ),
  ).toBeVisible();
  await page
    .getByRole("checkbox", {
      name: "Display classification banner",
      exact: true,
    })
    .check();
  await page
    .getByRole("textbox", { name: "Classification text", exact: true })
    .fill("SETTINGS ACCEPTANCE");
  await choose(page, "Tone", "Warning");
  await choose(page, "Placement", "Top and bottom");
  await page
    .getByRole("button", { name: "Save environment settings", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]).toMatchObject({
    body: {
      revision: 1,
      classification: {
        enabled: true,
        text: "SETTINGS ACCEPTANCE",
        tone: "warning",
        position: "top_and_bottom",
      },
    },
  });
  await page.reload();
  await expect(
    page.getByRole("textbox", { name: "Classification text", exact: true }),
  ).toHaveValue("SETTINGS ACCEPTANCE");
  await expect(
    page.getByRole("combobox", { name: "Placement", exact: true }),
  ).toContainText("Top and bottom");
  await expect(
    page.locator(".control-plane-frame > .control-classification"),
  ).toHaveCount(2);
  fixture.verify();
});

test("project details, membership roles and monitoring baseline persist", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/projects/" + projectId + "/settings");
  const editor = page.getByRole("form", { name: "Edit project", exact: true });
  await editor
    .getByRole("textbox", { name: "Name", exact: true })
    .fill("Reviewed project");
  await editor
    .getByRole("textbox", { name: "Description", exact: true })
    .fill("Saved by a project administrator");
  await editor
    .getByRole("button", { name: "Save project", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(1);
  expect(fixture.writes[0]).toMatchObject({
    path: projectPath,
    body: { revision: 1, name: "Reviewed project", parent_project_id: null },
  });
  await page.reload();
  await expect(
    editor.getByRole("textbox", { name: "Name", exact: true }),
  ).toHaveValue("Reviewed project");
  await page
    .getByRole("textbox", { name: "Required sandbox profile", exact: true })
    .fill("reviewed-profile");
  await page
    .getByRole("checkbox", { name: "Ask for approval", exact: true })
    .check();
  await page
    .getByRole("checkbox", { name: "Monitor a tool ceiling", exact: true })
    .check();
  await page
    .getByRole("textbox", { name: "Allowed tools · one per line", exact: true })
    .fill("mcp.docs.read\nfilesystem.search");
  await page
    .getByRole("button", { name: "Save baseline", exact: true })
    .click();
  await expect.poll(() => fixture.writes.length).toBe(2);
  expect(fixture.writes[1]).toMatchObject({
    path: projectPath + "/policy",
    body: {
      revision: 1,
      required_sandbox_profile: "reviewed-profile",
      allowed_approval_modes: ["ask"],
      allowed_tools: ["mcp.docs.read", "filesystem.search"],
    },
  });
  await page.goto("/projects/" + projectId + "/access");
  await page
    .getByRole("textbox", { name: "Find a user", exact: true })
    .fill("Matrix");
  await expect(
    page.getByRole("combobox", { name: "User", exact: true }),
  ).toBeEnabled();
  await choose(page, "User", "Matrix operator");
  await choose(page, "Role", "Operator");
  await page.getByRole("button", { name: "Add member", exact: true }).click();
  await expect.poll(() => fixture.writes.length).toBe(3);
  expect(fixture.writes[2]).toMatchObject({
    path: projectPath + "/members",
    body: { user_id: "matrix-user", role: "operator" },
  });
  await choose(page, "Role for Matrix operator", "Approver");
  await expect.poll(() => fixture.writes.length).toBe(4);
  expect(fixture.writes[3]).toMatchObject({
    method: "PATCH",
    body: { revision: 1, role: "approver" },
  });
  await page.reload();
  await expect(
    page.getByRole("combobox", {
      name: "Role for Matrix operator",
      exact: true,
    }),
  ).toContainText("Approver");
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect.poll(() => fixture.writes.length).toBe(5);
  await expect(
    page.getByText("No explicit project memberships.", { exact: true }),
  ).toBeVisible();
  fixture.verify();
});

test("personal Account shows the current identity and opens private connections", async ({
  page,
}) => {
  const fixture = await administrationFixture(page);
  await page.goto("/settings/account?project=" + projectId);
  const account = page.getByRole("region", {
    name: "Signed-in account",
    exact: true,
  });
  await expect(account).toContainText("Browser Test Operator");
  await expect(account.getByRole("textbox")).toHaveCount(0);
  await account
    .getByRole("button", { name: "Manage your connections", exact: true })
    .click();
  await expect(page).toHaveURL(
    /\/projects\/browser-project\/chat\/connections$/u,
  );
  await expect(
    page.getByRole("heading", { name: "Your connections", exact: true }),
  ).toBeVisible();
  expect(fixture.writes).toHaveLength(0);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(
    "administration forms fit Large " +
      appearance.label +
      " desktop and mobile layouts",
    async ({ page }) => {
      const fixture = await administrationFixture(page);
      await selectAppearance(page, appearance);
      for (const [id, path, label, action] of [
        ["user", "/admin/users/new", "Display name", "Save user"],
        [
          "classification",
          "/admin/settings",
          "Classification text",
          "Save environment settings",
        ],
        [
          "general",
          "/projects/" + projectId + "/settings",
          "Name",
          "Save baseline",
        ],
        [
          "members",
          "/projects/" + projectId + "/access",
          "Find a user",
          "Add member",
        ],
      ]) {
        await page.goto(path!);
        const field = page.getByRole("textbox", { name: label!, exact: true });
        await expect(field).toBeVisible();
        for (const width of [1440, 390]) {
          await page.setViewportSize({ width, height: 1000 });
          await field.scrollIntoViewIfNeeded();
          expect(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= innerWidth,
            ),
          ).toBe(true);
          await page.screenshot({
            path:
              "output/playwright/settings-" +
              id +
              "-" +
              appearance.id +
              "-large-" +
              width +
              ".png",
            fullPage: true,
          });
          const footer = page.getByRole("button", {
            name: action!,
            exact: true,
          });
          await footer.scrollIntoViewIfNeeded();
          await expect(footer).toBeInViewport();
          await page.screenshot({
            path:
              "output/playwright/settings-" +
              id +
              "-" +
              appearance.id +
              "-large-" +
              width +
              "-footer.png",
            fullPage: true,
          });
        }
      }
      fixture.verify();
    },
  );
}
