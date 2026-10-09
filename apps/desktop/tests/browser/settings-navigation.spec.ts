import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.use({ colorScheme: "dark" });

for (const width of [1586, 880, 700]) {
  test(`dedicated settings preserves scope geometry and edits at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 990 });
    await page.goto("/?fixture=operations-studio");
    if (width < 980) {
      await page.getByRole("button", { name: "Open work navigation" }).click();
    }
    const navigationStyle = await page
      .getByRole("button", { name: "Settings", exact: true })
      .evaluate((button) => {
        const style = getComputedStyle(button);
        return {
          fontSize: style.fontSize,
          fontWeight: style.fontWeight,
          padding: style.padding,
          gap: style.gap,
          borderRadius: style.borderRadius,
          minHeight: style.minHeight,
          iconWidth: button.querySelector("svg")!.getBoundingClientRect().width,
        };
      });
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Settings", exact: true }),
    ).toBeVisible();
    await expect(page.locator(".settings-brand > strong")).toBeVisible();
    await expect(page.locator("#work-navigation")).toHaveCount(0);
    const sidebar = page.getByRole("complementary", {
      name: "Settings navigation",
    });
    const categories = page.getByRole("navigation", {
      name: "Settings sections",
    });
    const sidebarBack = sidebar.getByRole("button", {
      name: "Back to work",
      exact: true,
    });
    await expect(sidebarBack).toBeInViewport();
    const globalScopeBounds = await sidebar
      .getByRole("button", { name: "Global", exact: true })
      .boundingBox();
    expect(globalScopeBounds!.height).toBeLessThanOrEqual(52);
    const geometry = () =>
      page.locator(".managed-settings-shell").evaluate((shell) => {
        const rect = (selector: string) => {
          const { x, y, width, height } = shell
            .querySelector(selector)!
            .getBoundingClientRect();
          return { x, y, width, height };
        };
        return {
          header: rect(".managed-settings-header"),
          sidebar: rect(".settings-sidebar"),
          content: rect(".settings-main"),
          categories: rect(".managed-settings-tabs"),
        };
      });
    const workspaceGeometry = await geometry();
    expect(workspaceGeometry.header.height).toBeLessThanOrEqual(60);
    const sectionStyles = await categories
      .getByRole("button")
      .evaluateAll((buttons) =>
        buttons.map((button) => {
          const style = getComputedStyle(button);
          return {
            fontSize: style.fontSize,
            fontWeight: style.fontWeight,
            padding: style.padding,
            gap: style.gap,
            borderRadius: style.borderRadius,
            minHeight: style.minHeight,
            iconWidth: button.querySelector("svg")!.getBoundingClientRect()
              .width,
          };
        }),
      );
    for (const style of sectionStyles) {
      const { iconWidth, ...sectionStyle } = style;
      const { iconWidth: navigationIconWidth, ...sidebarStyle } =
        navigationStyle;
      expect(
        sectionStyle,
        "Settings navigation matches the main sidebar",
      ).toEqual(sidebarStyle);
      expect(iconWidth, "Settings icons match the main sidebar").toBeCloseTo(
        navigationIconWidth,
        4,
      );
    }
    const pageStyle = async () => {
      const body = page.locator(".managed-settings-body");
      const heading = body.getByRole("heading", { level: 3 }).first();
      await expect(heading).toBeVisible();
      return {
        padding: await body.evaluate(
          (element) => getComputedStyle(element).padding,
        ),
        headingFontSize: await heading.evaluate(
          (element) => getComputedStyle(element).fontSize,
        ),
      };
    };
    const workspaceStyle = await pageStyle();
    const turns = page.getByRole("spinbutton", { name: "Maximum turns" });
    await turns.fill("75");
    await sidebar.getByRole("button", { name: "Global", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(
      sidebar.getByRole("button", { name: "Global", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    const globalHeading = categories.getByRole("heading", {
      name: "Global settings",
    });
    await expect(globalHeading).toBeVisible();
    await expect(globalHeading).toHaveCSS("border-top-width", "1px");
    const globalGeometry = await geometry();
    await expect(sidebarBack).toBeInViewport();
    // Keep the editor stable, but only reserve workspace controls in Workspace scope.
    expect(globalGeometry.header).toEqual(workspaceGeometry.header);
    expect(globalGeometry.sidebar).toEqual(workspaceGeometry.sidebar);
    expect(globalGeometry.content).toEqual(workspaceGeometry.content);
    const scopeSwitch = await sidebar
      .locator(".managed-scope-switch")
      .boundingBox();
    expect(
      globalGeometry.categories.y - scopeSwitch!.y - scopeSwitch!.height,
    ).toBeLessThanOrEqual(8);
    expect(globalGeometry.categories.y).toBeLessThan(
      workspaceGeometry.categories.y,
    );
    await expect(
      sidebar.getByRole("combobox", { name: "Workspace", exact: true }),
    ).toHaveCount(0);
    await categories
      .getByRole("button", { name: "Models", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Models", exact: true }),
    ).toBeVisible();
    const modelActions = await page
      .getByRole("button", { name: "Edit primary", exact: true })
      .boundingBox();
    const contentBounds = await page.locator(".settings-main").boundingBox();
    expect(modelActions!.x + modelActions!.width).toBeLessThanOrEqual(
      contentBounds!.x + contentBounds!.width,
    );
    for (const tab of [
      "Providers",
      "Models",
      "Credentials",
      "MCP",
      "Plugins",
      "Search",
      "Telemetry",
      "Defaults",
      "Appearance",
      "Connections",
      "Setup",
      "Git",
      "Browser",
      "Terminal",
      "Certificates",
      "Updates & diagnostics",
    ]) {
      await categories.getByRole("button", { name: tab, exact: true }).click();
      expect(
        await pageStyle(),
        `${tab} uses shared page spacing and type`,
      ).toEqual(workspaceStyle);
      const overflow = await page
        .locator(".settings-main")
        .evaluate((element) => element.scrollWidth - element.clientWidth);
      expect(
        overflow,
        `${tab} controls fit inside the editor`,
      ).toBeLessThanOrEqual(1);
    }
    await sidebar
      .getByRole("button", { name: "Workspace", exact: true })
      .click();
    await expect(turns).toHaveValue("75");
    await expect(
      page.getByRole("button", { name: "Apply Workspace changes" }),
    ).toBeEnabled();
    await page.getByRole("button", { name: "Discard", exact: true }).click();
    await expect(turns).toHaveValue("50");

    const workspace = sidebar.getByRole("combobox", {
      name: "Workspace",
      exact: true,
    });
    await workspace.click();
    await page
      .getByRole("option", { name: "Research Lab", exact: true })
      .click();
    await expect(page.locator(".settings-page-context")).toContainText(
      "Workspace / Research Lab",
    );
    await workspace.click();
    await page.getByRole("option", { name: "Colossus", exact: true }).click();
    await page
      .getByRole("searchbox", { name: "Search settings" })
      .fill("Maximum turns");
    await expect(page.locator(".settings-sidebar")).toBeVisible();
    await page.getByRole("button", { name: "Clear settings search" }).click();
    await expect(turns).toBeVisible();
    await categories
      .getByRole("button", { name: "Access", exact: true })
      .click();
    await expect(
      page.getByRole("combobox", { name: "Execution boundary" }),
    ).toHaveAccessibleDescription(
      "Choose how tools and processes are isolated. Full access disables filesystem and network isolation and requires native confirmation.",
    );
    await categories
      .getByRole("button", { name: "Sandbox", exact: true })
      .click();
    await expect(
      page.getByText(
        "Filesystem and network rules apply to isolated execution. Select the execution boundary in Access.",
      ),
    ).toBeVisible();
    await categories
      .getByRole("button", { name: "Runtime", exact: true })
      .click();
    for (const tab of [
      "Runtime",
      "Providers",
      "MCP",
      "Plugins",
      "Access",
      "Sandbox",
      "Search",
      "Telemetry",
      "Research",
      "Advanced",
      "Effective YAML",
    ]) {
      await categories.getByRole("button", { name: tab, exact: true }).click();
      expect(
        await pageStyle(),
        `${tab} uses shared page spacing and type`,
      ).toEqual(workspaceStyle);
    }
    await categories
      .getByRole("button", { name: "Runtime", exact: true })
      .click();
    await page.locator(".settings-main").evaluate((element) => {
      element.scrollTop = 0;
    });

    const overflow = await page
      .locator(".settings-main")
      .evaluate((element) => element.scrollWidth - element.clientWidth);
    expect(overflow).toBeLessThanOrEqual(1);
    const accessibility = await new AxeBuilder({ page })
      .include(".managed-settings-shell")
      .analyze();
    expect(
      accessibility.violations.filter((violation) =>
        ["critical", "serious"].includes(violation.impact ?? ""),
      ),
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/settings-sidebar-${width}.png`,
    });
    await page.setViewportSize({ width, height: 560 });
    await expect(sidebarBack).toBeInViewport();
    const footerBounds = await sidebarBack.boundingBox();
    expect(560 - footerBounds!.y - footerBounds!.height).toBeLessThanOrEqual(
      24,
    );
    await page.locator(".settings-sidebar-content").evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    expect(await sidebarBack.boundingBox()).toEqual(footerBounds);
    await sidebarBack.focus();
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
    ).toBeVisible();
  });
}

test("Desktop sections keep controls focused and are discoverable through search", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 760 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  const navigation = page.getByRole("navigation", {
    name: "Settings sections",
  });
  await expect(
    navigation.getByRole("heading", { name: "Desktop", exact: true }),
  ).toBeVisible();
  await navigation
    .getByRole("button", { name: "Appearance", exact: true })
    .click();
  await expect(
    page.getByRole("combobox", { name: /Color theme/u }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Import PEM", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Check for updates", exact: true }),
  ).toHaveCount(0);

  await navigation
    .getByRole("button", { name: "Connections", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Choose workspace", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add external runtime", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: /Color theme/u }),
  ).toHaveCount(0);

  await expect(
    page.getByRole("button", { name: "Import setup file", exact: true }),
  ).toHaveCount(0);
  await navigation
    .getByRole("button", { name: "Providers", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Export global setup", exact: true }),
  ).toHaveCount(0);
  await navigation.getByRole("button", { name: "Setup", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Import setup file", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Export global setup", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Choose workspace", exact: true }),
  ).toHaveCount(0);

  await navigation
    .getByRole("button", { name: "Terminal", exact: true })
    .click();
  await expect(
    page.getByRole("switch", { name: "Enable local terminal" }),
  ).toBeChecked();
  await expect(
    page.getByRole("combobox", { name: "Default session" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Shell", exact: true }),
  ).toHaveCount(0);

  await navigation
    .getByRole("button", { name: "Certificates", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Import PEM", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Shell", exact: true }),
  ).toHaveCount(0);

  await navigation
    .getByRole("button", { name: "Updates & diagnostics", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Check for updates", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Export diagnostics", exact: true }),
  ).toBeVisible();

  // Desktop preferences must also be discoverable when searching from a workspace.
  await page.getByRole("button", { name: "Workspace", exact: true }).click();
  const search = page.getByRole("searchbox", { name: "Search settings" });
  await search.fill("text size");
  await page.getByRole("button", { name: /Appearance Desktop/u }).click();
  await expect(search).toHaveValue("");
  await expect(page.locator(".settings-page-context")).toHaveText(
    "Desktop / Appearance",
  );
  await expect(
    navigation.getByRole("button", { name: "Appearance", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("combobox", { name: /Text size/u }),
  ).toBeVisible();
  await page.screenshot({
    path: "output/playwright/desktop-sections-appearance.png",
  });
  await search.fill("import");
  await page.getByRole("button", { name: /Setup Desktop/u }).click();
  await expect(search).toHaveValue("");
  await expect(page.locator(".settings-page-context")).toHaveText(
    "Desktop / Setup",
  );
  await expect(
    navigation.getByRole("button", { name: "Setup", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.screenshot({
    path: "output/playwright/desktop-sections-setup.png",
  });
});
