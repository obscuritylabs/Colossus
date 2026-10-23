import { expect, test, type Page } from "@playwright/test";
import type { Thread } from "../../src/api";
import type { Project, User } from "../../src/control-api";
import { appearanceCases } from "./appearance";
import { browserFixture } from "./fixture";

const timestamp = "2026-10-22T10:00:00Z";
const projectId = "browser-project";
const selfId = "browser-user";
const foreignId = "another-user";
const user: User = {
  id: selfId,
  display_name: "Browser Test Operator",
  email: "operator@example.test",
  active: true,
  is_admin: true,
  revision: 1,
  created_at: timestamp,
  updated_at: timestamp,
  identities: [{ kind: "local", label: "Local account", username: "operator" }],
};
const foreignUser: User = {
  ...user,
  id: foreignId,
  display_name: "Morgan Ellis",
  email: "morgan@example.test",
  is_admin: false,
};
const projects: Project[] = [
  {
    id: projectId,
    name: "Browser test project",
    description: "Synthetic browser acceptance data",
    parent_project_id: null,
    archived: false,
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
  },
  {
    id: "design-project",
    name: "Design review project",
    description: "A separately scoped membership",
    parent_project_id: null,
    archived: true,
    revision: 1,
    created_at: timestamp,
    updated_at: timestamp,
  },
];
const chats: Thread[] = Array.from({ length: 12 }, (_, index) => ({
  thread_id: `profile-chat-${String(index + 1).padStart(2, "0")}`,
  project_id: projectId,
  owner_user_id: selfId,
  node_id: "browser-node",
  host_id: "browser-host",
  workspace_id: "profile-workspace",
  title: `Profile conversation ${String(index + 1).padStart(2, "0")}`,
  revision: 1,
  created_at: timestamp,
  updated_at: timestamp,
  archived: index === 11,
  source: "cloud",
  sync_status: "current",
  can_continue: true,
}));

async function profileFixture(
  page: Page,
  options: {
    globalAdmin?: boolean;
    failProfileRead?: boolean;
    chatContinuation?: boolean;
  } = {},
) {
  const fixture = await browserFixture(page, {
    globalAdmin: options.globalAdmin ?? true,
    administer: options.globalAdmin === false,
  });
  const reads: string[] = [];
  const contentReads: string[] = [];
  const expectedConsoleErrors: string[] = [];
  let failProfileRead = Boolean(options.failProfileRead);
  await page.route("**/api/me", (route) =>
    route.fulfill({
      json: {
        user: { ...user, is_admin: options.globalAdmin ?? true },
        projects,
        memberships: projects.map((project) => ({
          project_id: project.id,
          user_id: selfId,
          subject: selfId,
          role: "operator",
          permissions: [
            "read",
            "execute",
            "control",
            ...(options.globalAdmin === false ? ["administer"] : []),
          ],
        })),
      },
    }),
  );
  await page.route("**/api/projects/design-project/**", (route) => {
    const collection = /\/(nodes|tasks|hosts)$/u.exec(
      new URL(route.request().url()).pathname,
    )?.[1];
    if (route.request().method() !== "GET" || !collection)
      return route.fallback();
    return route.fulfill({ json: { [collection]: [], next_cursor: null } });
  });
  page.on("console", (message) => {
    if (
      message.type() === "error" &&
      /Failed to load resource:.*503/u.test(message.text())
    )
      expectedConsoleErrors.push(message.text());
  });
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (/\/threads\/[^/]+(?:\/(?:messages|events))?$/u.test(url.pathname))
      contentReads.push(url.pathname);
  });
  await page.route("**/api/admin/users**", async (route) => {
    const request = route.request();
    if (request.method() !== "GET") return route.fallback();
    const url = new URL(request.url());
    const path = url.pathname;
    reads.push(`${path}${url.search}`);
    if (path === "/api/admin/users")
      return route.fulfill({
        json: { users: [user, foreignUser], next_cursor: null },
      });
    const match = /^\/api\/admin\/users\/([^/]+)(?:\/(.*))?$/u.exec(path);
    if (!match) return route.fallback();
    const selected = match[1] === foreignId ? foreignUser : user;
    if (!match[2]) {
      if (failProfileRead)
        return route.fulfill({ status: 503, json: { error: "storage" } });
      return route.fulfill({ json: { user: selected } });
    }
    if (match[2] === "projects")
      return route.fulfill({
        json: {
          projects: projects.map((project) => ({
            project,
            membership: {
              project_id: project.id,
              user_id: selected.id,
              subject: selected.id,
              role: "operator",
              permissions: ["read", "execute", "control"],
              revision: 1,
            },
          })),
          next_cursor: null,
        },
      });
    const resource = /^projects\/([^/]+)\/(workspaces|chats)$/u.exec(match[2]!);
    if (!resource) return route.fallback();
    const visible = selected.id === selfId;
    const visibility = visible ? "visible" : "personal_resources_private";
    if (resource[2] === "workspaces")
      return route.fulfill({
        json: {
          workspaces:
            visible && resource[1] === projectId
              ? [
                  {
                    workspace_id: "profile-workspace",
                    project_id: projectId,
                    host_id: "browser-host",
                    node_id: "browser-node",
                    label: "Operator development workspace",
                    sharing: "personal",
                    revision: 1,
                  },
                ]
              : [],
          next_cursor: null,
          visibility,
        },
      });
    const hasChats = visible && resource[1] === projectId;
    const after = url.searchParams.get("after");
    if (after) expect(after).toBe("profile-chats-after-12");
    return route.fulfill({
      json: {
        chats: hasChats
          ? after
            ? [
                {
                  ...chats[0]!,
                  thread_id: "profile-chat-13",
                  title: "Profile conversation 13",
                },
              ]
            : chats
          : [],
        next_cursor:
          hasChats && options.chatContinuation && !after
            ? "profile-chats-after-12"
            : null,
        visibility,
      },
    });
  });
  return {
    reads,
    contentReads,
    recoverProfile: () => {
      failProfileRead = false;
    },
    verify: () => {
      expect(contentReads).toEqual([]);
      fixture.verify(expectedConsoleErrors);
    },
  };
}

function profileUrl(id = selfId, section = "") {
  return `/admin/users/${id}${section ? `/${section}` : ""}`;
}

async function openTab(page: Page, name: string) {
  const href = profileUrl(selfId, name.toLowerCase());
  const destination = page.locator(`a[href="${href}"]`).first();
  await expect(destination).toBeVisible();
  await destination.focus();
  await destination.press("Enter");
}

test("a user has a keyboard accessible profile with linkable resource tabs and browser history", async ({
  page,
}, testInfo) => {
  const fixture = await profileFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/users");
  const userLink = page.getByRole("link", {
    name: user.display_name,
    exact: true,
  });
  await expect(userLink).toHaveAttribute("href", profileUrl());
  await userLink.focus();
  await userLink.press("Enter");
  await expect(page).toHaveURL(profileUrl());
  await expect(
    page.getByRole("heading", { name: user.display_name, level: 1 }),
  ).toBeVisible();
  await expect(
    page.getByText(user.email!, { exact: true }).first(),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Edit user", exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("admin-user-overview-desktop.png"),
  });
  const edit = page.getByRole("button", { name: "Edit user", exact: true });
  await edit.focus();
  await edit.press("Enter");
  await expect(page).toHaveURL(`${profileUrl()}/edit`);
  await page.reload();
  await expect(
    page.getByRole("textbox", { name: "Display name", exact: true }),
  ).toHaveValue(user.display_name);
  await expect(
    page.getByRole("checkbox", { name: /^Active user/u }),
  ).toBeChecked();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Display name", exact: true }),
  ).toHaveCount(0);
  await expect(page).toHaveURL(profileUrl());
  await expect(edit).toBeVisible();

  await openTab(page, "Projects");
  await expect(page).toHaveURL(profileUrl(selfId, "projects"));
  await expect(
    page.getByRole("table", { name: "User projects", exact: true }),
  ).toContainText(projects[0]!.name);
  await openTab(page, "Workspaces");
  await expect(page).toHaveURL(profileUrl(selfId, "workspaces"));
  await expect(
    page.getByRole("table", { name: "User workspaces", exact: true }),
  ).toContainText("Operator development workspace");
  await openTab(page, "Chats");
  await expect(page).toHaveURL(profileUrl(selfId, "chats"));
  await expect(
    page.getByRole("table", { name: "User chats", exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("table", { name: "User chats", exact: true }),
  ).toContainText("Profile conversation 01");
  await page.goBack();
  await expect(page).toHaveURL(profileUrl(selfId, "workspaces"));
  await expect(
    page.getByRole("table", { name: "User workspaces", exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Back to users", exact: true }).click();
  await expect(page).toHaveURL("/admin/users");
  fixture.verify();
});

test("personal chat metadata supports loaded search, sorting, pagination and project scope", async ({
  page,
}, testInfo) => {
  const fixture = await profileFixture(page, { chatContinuation: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(profileUrl(selfId, "chats"));
  const table = page.getByRole("table", { name: "User chats", exact: true });
  await expect(table).toContainText("Profile conversation 01");
  const initialChatReads = fixture.reads.filter((path) =>
    path.includes(`/projects/${projectId}/chats`),
  ).length;
  await page
    .getByRole("button", { name: "Refresh user details", exact: true })
    .click();
  await expect
    .poll(
      () =>
        fixture.reads.filter((path) =>
          path.includes(`/projects/${projectId}/chats`),
        ).length,
    )
    .toBeGreaterThan(initialChatReads);
  await expect(table).toContainText("Profile conversation 01");
  await expect(table).not.toContainText("Profile conversation 12");
  await page.screenshot({
    path: testInfo.outputPath("admin-user-chats-desktop.png"),
  });
  const next = page.getByRole("button", { name: "Next page", exact: true });
  await next.focus();
  await next.press("Enter");
  await expect(table).toContainText("Profile conversation 12");
  await expect(page.getByText("Page 2 of 2", { exact: true })).toBeVisible();
  const search = page.getByRole("searchbox", {
    name: "Search loaded chats",
    exact: true,
  });
  await search.fill("conversation 03");
  await expect(table).toContainText("Profile conversation 03");
  await expect(table).not.toContainText("Profile conversation 12");
  await expect(page.getByText("Page 1 of 1", { exact: true })).toBeVisible();
  await search.fill("");
  const sort = table.getByRole("button", { name: "Sort by Chat", exact: true });
  const chatHeader = table.getByRole("columnheader").filter({
    has: page.getByRole("button", { name: "Sort by Chat", exact: true }),
  });
  await sort.focus();
  for (let attempt = 0; attempt < 3; attempt++) {
    if ((await chatHeader.getAttribute("aria-sort")) === "descending") break;
    await sort.press("Enter");
  }
  await expect(table.locator("tbody tr").first()).toContainText(
    "Profile conversation 12",
  );
  await expect(
    table.getByRole("columnheader").filter({
      has: page.getByRole("button", { name: "Sort by Chat", exact: true }),
    }),
  ).toHaveAttribute("aria-sort", "descending");

  await search.fill("Profile conversation 13");
  await expect(table).toContainText("No personal chats found in this project.");
  await page
    .getByRole("button", { name: "Load more chats", exact: true })
    .click();
  await expect(table).toContainText("Profile conversation 13");
  await expect(
    page.getByRole("button", { name: "Load more chats", exact: true }),
  ).toHaveCount(0);
  expect(
    fixture.reads.some((path) => path.includes("after=profile-chats-after-12")),
  ).toBe(true);
  await search.fill("");

  await page
    .getByRole("combobox", { name: "Project for user resources", exact: true })
    .click();
  await page
    .getByRole("option", {
      name: `${projects[1]!.name} (Archived)`,
      exact: true,
    })
    .click();
  await expect(page).toHaveURL(
    `${profileUrl(selfId, "chats")}?project=design-project`,
  );
  await expect(table).toContainText("No personal chats found in this project.");
  await page.reload();
  await expect(table).toContainText("No personal chats found in this project.");
  await expect
    .poll(() =>
      fixture.reads.some((path) =>
        path.startsWith(
          `/api/admin/users/${selfId}/projects/design-project/chats`,
        ),
      ),
    )
    .toBe(true);
  fixture.verify();
});

test("another user's personal resources remain private while their identity and memberships are visible", async ({
  page,
}) => {
  const fixture = await profileFixture(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(profileUrl(foreignId, "projects"));
  await expect(
    page.getByRole("heading", { name: foreignUser.display_name, level: 1 }),
  ).toBeVisible();
  await expect(
    page.getByRole("table", { name: "User projects", exact: true }),
  ).toContainText(projects[0]!.name);
  for (const section of ["workspaces", "chats"]) {
    await page.goto(profileUrl(foreignId, section));
    await expect(page.getByText(/personal.*private/iu).first()).toBeVisible();
    await expect(
      page.getByText("Operator development workspace", { exact: true }),
    ).toHaveCount(0);
    await expect(page.getByText(/Profile conversation/u)).toHaveCount(0);
    await expect(
      page.getByText("Retained personal chat question", { exact: true }),
    ).toHaveCount(0);
  }
  fixture.verify();
});

test("a failed profile read can be retried without leaving the user route", async ({
  page,
}) => {
  const fixture = await profileFixture(page, { failProfileRead: true });
  await page.goto(profileUrl());
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("temporarily unavailable");
  const retry = alert.getByRole("button", { name: "Retry", exact: true });
  fixture.recoverProfile();
  await retry.focus();
  await retry.press("Enter");
  await expect(
    page.getByRole("heading", { name: user.display_name, level: 1 }),
  ).toBeVisible();
  await expect(alert).toHaveCount(0);
  await expect(page).toHaveURL(profileUrl());
  fixture.verify();
});

test("a project administrator cannot read a profile through a copied URL", async ({
  page,
}) => {
  const fixture = await profileFixture(page, { globalAdmin: false });
  await page.goto(profileUrl());
  await expect(
    page.getByText(
      "Your signed-in identity does not have Control Plane administration access.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: user.display_name, level: 1 }),
  ).toHaveCount(0);
  expect(fixture.reads).toEqual([]);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`user details and resource tables fit narrow ${appearance.label} Large text`, async ({
    page,
  }, testInfo) => {
    const fixture = await profileFixture(page);
    await page.addInitScript((appearance) => {
      localStorage.setItem(
        "colossus.web.appearance.v1",
        JSON.stringify({
          colorTheme: appearance.theme,
          darkPalette: appearance.palette,
          textSize: "large",
        }),
      );
    }, appearance);
    await page.setViewportSize({ width: 390, height: 900 });
    for (const section of ["", "projects", "workspaces", "chats"]) {
      await page.goto(profileUrl(selfId, section));
      await expect(
        page.getByRole("heading", { name: user.display_name, level: 1 }),
      ).toBeVisible();
      if (section) {
        const table = page.getByRole("table", {
          name: `User ${section}`,
          exact: true,
        });
        await expect(table).toContainText(
          section === "projects"
            ? projects[0]!.name
            : section === "workspaces"
              ? "Operator development workspace"
              : "Profile conversation 01",
        );
        expect(
          await table.evaluate(
            (element) => element.scrollWidth <= element.clientWidth,
          ),
        ).toBe(true);
      }
      await expect(page.locator("html")).toHaveAttribute(
        "data-text-size",
        "large",
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: testInfo.outputPath(
          `admin-user-${section || "overview"}-${appearance.id}-large-mobile.png`,
        ),
        fullPage: true,
      });
    }
    const sort = page.getByRole("combobox", { name: "Sort rows", exact: true });
    await sort.focus();
    await sort.press("Enter");
    await page
      .getByRole("option", { name: "Chat · descending", exact: true })
      .click();
    await expect(
      page
        .getByRole("table", { name: "User chats", exact: true })
        .locator("tbody tr")
        .first(),
    ).toContainText("Profile conversation 12");
    if (appearance.id === "black") {
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.goto(profileUrl(selfId, "chats"));
      await expect(
        page.getByRole("table", { name: "User chats", exact: true }),
      ).toContainText("Profile conversation 01");
      await page.screenshot({
        path: testInfo.outputPath("admin-user-chats-black-desktop.png"),
      });
    }
    fixture.verify();
  });
}
