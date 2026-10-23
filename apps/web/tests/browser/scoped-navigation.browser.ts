import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Me } from "../../src/control-api";
import { appearanceCases } from "./appearance";
import { browserFixture } from "./fixture";

const project = "browser-project";
const chatUrl = `/projects/${project}/chat/browser-thread`;
const projectSections = [
  ["Members", `/projects/${project}/access`],
  ["Providers", `/projects/${project}/settings/providers`],
  ["Models", `/projects/${project}/settings/models`],
  ["MCP servers", `/projects/${project}/settings/mcp`],
  ["Model routing", `/projects/${project}/settings/routing`],
  ["Search", `/projects/${project}/settings/search`],
  ["Telemetry", `/projects/${project}/settings/telemetry`],
  ["Secrets", `/projects/${project}/settings/credentials`],
  ["Variables", `/projects/${project}/settings/variables`],
  ["General", `/projects/${project}/settings`],
] as const;
const globalSections = [
  ["Users", "/admin/users"],
  ["Projects", "/admin/projects"],
  ["Providers", "/admin/providers"],
  ["Models", "/admin/models"],
  ["MCP servers", "/admin/mcp"],
  ["Model routing", "/admin/routing"],
  ["Search", "/admin/search"],
  ["Telemetry", "/admin/telemetry"],
  ["Secrets", "/admin/credentials"],
  ["Variables", "/admin/variables"],
  ["Announcements", "/admin/announcements"],
  ["Appearance", "/admin/appearance"],
  ["Authentication & settings", "/admin/settings"],
] as const;
type Role =
  "operator" | "project administrator" | "Control Plane administrator";
async function navigationFixture(page: Page, role: Role, noProject = false) {
  const fixture = await browserFixture(page, {
    managedChat: true,
    administer: role === "project administrator",
    globalAdmin: role === "Control Plane administrator",
  });
  const adminReads: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (request.method() === "GET" && path.startsWith("/api/admin/"))
      adminReads.push(path);
  });
  await page.route("**/api/admin/users**", async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    return route.fulfill({ json: { users: [], next_cursor: null } });
  });
  if (noProject) {
    const identity: Me = {
      user: {
        id: "browser-user",
        display_name: "Browser Test Operator",
        email: null,
        active: true,
        is_admin: role === "Control Plane administrator",
        revision: 1,
        created_at: "2026-10-22T10:00:00Z",
        updated_at: "2026-10-22T10:00:00Z",
      },
      projects: [],
      memberships: [],
    };
    await page.route("**/api/me", (route) => route.fulfill({ json: identity }));
  }
  return { ...fixture, adminReads };
}
function application(page: Page) {
  return page.getByRole("navigation", { name: "Application", exact: true });
}
async function openApplication(page: Page) {
  const toggle = page.getByRole("button", {
    name: "Toggle app sidebar",
    exact: true,
  });
  await expect(toggle).toHaveAttribute("aria-expanded", /^(true|false)$/u);
  if ((await toggle.getAttribute("aria-expanded")) === "false")
    await toggle.click();
  await expect(application(page)).toBeVisible();
}
async function expectDestinations(
  nav: Locator,
  items: readonly (readonly [string, string])[],
) {
  for (const [name, href] of items) {
    const link = nav.getByRole("link", { name, exact: true });
    await expect(link).toBeVisible();
    await expect(link).toHaveAttribute("href", href);
  }
}
for (const role of [
  "operator",
  "project administrator",
  "Control Plane administrator",
] as const) {
  test(`${role} sees compact authorized administration destinations and personal settings`, async ({
    page,
  }) => {
    const fixture = await navigationFixture(page, role);
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(chatUrl);
    await openApplication(page);
    const workspace = application(page).getByRole("region", {
      name: "Workspace",
      exact: true,
    });
    for (const label of ["Home", "Chat", "Hosted", "Fleet", "Projects"])
      await expect(
        workspace.getByRole("link", { name: label, exact: true }),
      ).toBeVisible();
    await expect(
      workspace.getByRole("link", { name: "Projects", exact: true }),
    ).toHaveAttribute("href", "/projects");
    const administration = application(page).getByRole("region", {
      name: "Administration",
      exact: true,
    });
    if (role === "operator") await expect(administration).toHaveCount(0);
    else {
      await expect(
        administration.getByRole("link", {
          name: "Project Admin",
          exact: true,
        }),
      ).toHaveAttribute("href", `/projects/${project}/access`);
      await expect(administration.getByRole("link")).toHaveCount(
        role === "Control Plane administrator" ? 2 : 1,
      );
      await page.goto(`/projects/${project}/settings/providers`);
      const secondary = page.getByRole("navigation", {
        name: "Project Admin sections",
        exact: true,
      });
      await expectDestinations(secondary, projectSections);
      await expect(
        page.getByRole("navigation", {
          name: "Settings sections",
          exact: true,
        }),
      ).toHaveCount(0);
      await expect(
        secondary.getByRole("link", { name: "Providers", exact: true }),
      ).toHaveAttribute("aria-current", "page");
      await expect(
        page.getByRole("complementary", {
          name: "Project Admin navigation",
          exact: true,
        }),
      ).toContainText("Browser test project");
    }
    if (role === "Control Plane administrator") {
      await application(page)
        .getByRole("link", { name: "Control Plane Admin", exact: true })
        .press("Enter");
      await expect(page).toHaveURL("/admin/users");
      const secondary = page.getByRole("navigation", {
        name: "Control Plane Admin sections",
        exact: true,
      });
      await expectDestinations(secondary, globalSections);
      await expect(
        secondary.getByRole("link", { name: "Users", exact: true }),
      ).toHaveAttribute("aria-current", "page");
      await expect(
        application(page).getByRole("link", {
          name: "Control Plane Admin",
          exact: true,
        }),
      ).toHaveAttribute("aria-current", "page");
      await expect(
        page.getByRole("navigation", { name: "Breadcrumb", exact: true }),
      ).toContainText("Users");
    } else
      await expect(
        application(page).getByRole("link", {
          name: "Control Plane Admin",
          exact: true,
        }),
      ).toHaveCount(0);
    const settings = application(page)
      .getByRole("region", { name: "Personal", exact: true })
      .getByRole("link", { name: "Settings", exact: true });
    await expect(settings).toHaveAttribute(
      "href",
      `/settings/appearance?project=${project}`,
    );
    await settings.press("Enter");
    await expect(page).toHaveURL(`/settings/appearance?project=${project}`);
    await expect(page.locator("#appearance-text-size")).toBeVisible();
    if (role !== "Control Plane administrator")
      expect(fixture.adminReads).toEqual([]);
    fixture.verify();
  });
}
test("Control Plane Users remains reachable without a selected project", async ({
  page,
}) => {
  const fixture = await navigationFixture(
    page,
    "Control Plane administrator",
    true,
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/users");
  await expect(
    application(page).getByRole("link", { name: "Project Admin", exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByRole("navigation", {
        name: "Control Plane Admin sections",
        exact: true,
      })
      .getByRole("link", { name: "Users", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("table", { name: "Control Plane users", exact: true }),
  ).toBeVisible();
  await application(page)
    .getByRole("link", { name: "Settings", exact: true })
    .press("Enter");
  await expect(page).toHaveURL("/settings/appearance");
  fixture.verify();
});
test("a project administrator cannot reach global Users through a copied URL", async ({
  page,
}) => {
  const fixture = await navigationFixture(page, "project administrator");
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/users");
  await expect(
    page.getByText(
      "Your signed-in identity does not have Control Plane administration access.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    application(page).getByRole("link", {
      name: "Control Plane Admin",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("table", { name: "Control Plane users", exact: true }),
  ).toHaveCount(0);
  expect(fixture.adminReads).toEqual([]);
  fixture.verify();
});
for (const appearance of appearanceCases) {
  test(`compact administration rail and keyboard secondary sheets fit ${appearance.label} Large layouts`, async ({
    page,
  }) => {
    const fixture = await navigationFixture(
      page,
      "Control Plane administrator",
    );
    await page.addInitScript(
      (appearance) =>
        localStorage.setItem(
          "colossus.web.appearance.v1",
          JSON.stringify({
            colorTheme: appearance.theme,
            darkPalette: appearance.palette,
            textSize: "large",
          }),
        ),
      appearance,
    );
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto("/admin/users");
    const toggle = page.getByRole("button", {
      name: "Toggle app sidebar",
      exact: true,
    });
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(application(page).getByRole("link")).toHaveCount(8);
    await expect
      .poll(
        async () =>
          (await page.locator(".control-app-sidebar").boundingBox())!.width,
      )
      .toBeLessThanOrEqual(49);
    const admin = application(page).getByRole("link", {
      name: "Control Plane Admin",
      exact: true,
    });
    await admin.focus();
    await admin.press("Enter");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(
      page.getByRole("navigation", {
        name: "Control Plane Admin sections",
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({
      path: `output/playwright/scoped-navigation-${appearance.id}-large-desktop.png`,
    });
    await page.setViewportSize({ width: 390, height: 900 });
    await expect(
      page.locator(".control-app-sidebar[data-sidebar='sidebar']"),
    ).toHaveCount(0);
    const secondaryToggle = page.getByRole("button", {
      name: "Toggle administration navigation",
      exact: true,
    });
    await secondaryToggle.focus();
    await secondaryToggle.press("Enter");
    const secondaryDialog = page.getByRole("dialog", {
      name: "Control Plane Admin navigation",
      exact: true,
    });
    await expect(secondaryDialog).toBeVisible();
    const users = secondaryDialog.getByRole("link", {
      name: "Users",
      exact: true,
    });
    await users.focus();
    await expect(users).toBeFocused();
    expect((await users.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await secondaryDialog
      .getByRole("link", { name: "Authentication & settings", exact: true })
      .scrollIntoViewIfNeeded();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: `output/playwright/scoped-navigation-${appearance.id}-large-mobile-secondary.png`,
    });
    await page.keyboard.press("Escape");
    await expect(secondaryDialog).toBeHidden();
    await expect(secondaryToggle).toBeFocused();
    await openApplication(page);
    const primaryDialog = page.getByRole("dialog", {
      name: "Navigation",
      exact: true,
    });
    await expect(
      primaryDialog
        .getByRole("navigation", { name: "Application", exact: true })
        .getByRole("link"),
    ).toHaveCount(8);
    await expect(
      primaryDialog
        .getByRole("region", { name: "Administration", exact: true })
        .getByRole("link"),
    ).toHaveCount(2);
    await page.keyboard.press("Escape");
    await expect(primaryDialog).toBeHidden();
    await expect(toggle).toBeFocused();
    fixture.verify();
  });
}
