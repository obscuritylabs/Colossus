import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function installInventory(page: Page, fail = false) {
  await page.evaluate((fail) => {
    const host = window as unknown as Record<string, any>;
    host.capabilityRequests = [];
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        if (command === "list_setup_packages") return [];
        if (command !== "get_capability_inventory")
          throw new Error(`Unexpected command ${command}`);
        host.capabilityRequests.push(args.targetId);
        if (fail)
          throw {
            code: "transport",
            message: "private-token-that-must-not-be-rendered",
            retryable: true,
            violations: [],
          };
        return {
          targetId: args.targetId,
          sections: [
            {
              kind: "tools",
              available: true,
              message: "",
              entries: [
                {
                  name: "filesystem.read",
                  description: "Read an authorized workspace file.",
                  detail: "filesystem",
                  status: "Active",
                },
                {
                  name: "web.search",
                  description: "Search through the selected search service.",
                  detail: "network",
                  status: "Active",
                },
              ],
            },
            {
              kind: "mcp",
              available: true,
              message: "",
              entries: [
                {
                  name: "Engineering documentation",
                  description:
                    "Allowed tools: documentation_search, fetch_documentation",
                  detail: "streamable_http",
                  status: "Configured",
                },
              ],
            },
            {
              kind: "search",
              available: true,
              message: "",
              entries: [
                {
                  name: "Workspace search",
                  description: "",
                  detail: "searxng",
                  status: "Configured",
                },
              ],
            },
            {
              kind: "plugins",
              available: true,
              message: "",
              entries: [
                {
                  name: "colossus",
                  description: "Built-in workspace skills.",
                  detail: "colossus/coding, colossus/security-review",
                  status: "Enabled",
                },
              ],
            },
          ],
        };
      },
    };
  }, fail);
}

async function openCapabilities(page: Page) {
  await expect(
    page.getByRole("heading", {
      name: "Harden desktop agent bootstrap",
      exact: true,
    }),
  ).toBeVisible();
  const navigation = page.getByRole("navigation", {
    name: "Workspace destinations",
  });
  if (!(await navigation.isVisible()))
    await page
      .getByRole("button", { name: "Open work navigation", exact: true })
      .click();
  await navigation
    .getByRole("button", { name: "Capabilities", exact: true })
    .click();
}

test("Capabilities reads the selected native runtime inventory and keeps artifacts in the thread", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installInventory(page);
  await expect(
    page.getByRole("button", { name: "Library", exact: true }),
  ).toHaveCount(0);
  await openCapabilities(page);
  await expect(
    page.getByRole("heading", { name: "Capabilities", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("table", { name: "Tools", exact: true }),
  ).toContainText("filesystem.read");
  await expect(
    page.getByRole("table", { name: "MCP servers", exact: true }),
  ).toContainText("documentation_search");
  await expect(
    page.getByRole("table", { name: "Search services", exact: true }),
  ).toContainText("searxng");
  await expect(
    page.getByRole("table", { name: "Plugins", exact: true }),
  ).toContainText("colossus/security-review");
  await expect(
    page.getByText("Available orchestration", { exact: true }),
  ).toHaveCount(0);
  const tools = page.getByRole("searchbox", {
    name: "Search tools",
    exact: true,
  });
  await tools.focus();
  await tools.fill("web.search");
  await expect(
    page.getByRole("table", { name: "Tools", exact: true }),
  ).not.toContainText("filesystem.read");
  await tools.fill("");
  await page
    .getByRole("button", { name: "Refresh inventory", exact: true })
    .click();
  await expect(
    page.getByRole("table", { name: "Tools", exact: true }),
  ).toContainText("filesystem.read");
  expect(await page.evaluate(() => (window as any).capabilityRequests)).toEqual(
    ["fixture-managed-local", "fixture-managed-local"],
  );
  await page
    .getByRole("navigation", { name: "Workspace destinations" })
    .getByRole("button", { name: "Work", exact: true })
    .click();
  await page.getByRole("button", { name: "Resources", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Resources", exact: true }),
  ).toBeVisible();
});

test("inventory failures remain explicit and can be refreshed without exposing native errors", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  await installInventory(page, true);
  await openCapabilities(page);
  await expect(page.getByRole("alert")).toContainText("could not be loaded");
  await expect(page.locator("body")).not.toContainText(
    "private-token-that-must-not-be-rendered",
  );
  await installInventory(page);
  await page
    .getByRole("button", { name: "Refresh inventory", exact: true })
    .click();
  await expect(
    page.getByRole("table", { name: "Tools", exact: true }),
  ).toContainText("filesystem.read");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

for (const appearance of [
  { name: "light", colorTheme: "light", darkPalette: "colossus" },
  { name: "colossus", colorTheme: "dark", darkPalette: "colossus" },
  { name: "neutral", colorTheme: "dark", darkPalette: "neutral" },
  { name: "black", colorTheme: "dark", darkPalette: "black" },
  { name: "hacker", colorTheme: "dark", darkPalette: "hacker" },
]) {
  test(`capability inventory remains accessible at compact width with Large text in ${appearance.name}`, async ({
    page,
  }) => {
    await page.addInitScript(
      (appearance) =>
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({ ...appearance, textSize: "large" }),
        ),
      appearance,
    );
    await page.goto("/?fixture=operations-studio");
    await installInventory(page);
    await openCapabilities(page);
    await expect(
      page.getByRole("table", { name: "Tools", exact: true }),
    ).toContainText("filesystem.read");
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(880);
    const navigation = await page
      .getByRole("button", { name: "Open Workspace navigation", exact: true })
      .boundingBox();
    const title = await page.locator(".surface-title-copy").boundingBox();
    expect(navigation!.x + navigation!.width).toBeLessThan(title!.x);
    const results = await new AxeBuilder({ page })
      .include(".operations-surface")
      .analyze();
    expect(
      results.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/capabilities-${appearance.name}-large.png`,
    });
  });
}
