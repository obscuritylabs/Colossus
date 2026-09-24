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
      expect(style, "Settings navigation matches the main sidebar").toEqual(
        navigationStyle,
      );
    }
    const pageStyle = () =>
      page.locator(".managed-settings-body").evaluate((body) => {
        const heading = body.querySelector(".managed-section-heading h3")!;
        return {
          padding: getComputedStyle(body).padding,
          headingFontSize: getComputedStyle(heading).fontSize,
        };
      });
    const workspaceStyle = await pageStyle();
    const turns = page.getByRole("spinbutton", { name: "Maximum turns" });
    await turns.fill("75");
    await sidebar.getByRole("button", { name: "Global", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(
      sidebar.getByRole("button", { name: "Global", exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    const globalGeometry = await geometry();
    await expect(sidebarBack).toBeInViewport();
    // Switching scope must not shift the title, content canvas, or category start.
    expect(globalGeometry.header).toEqual(workspaceGeometry.header);
    expect(globalGeometry.sidebar).toEqual(workspaceGeometry.sidebar);
    expect(globalGeometry.content).toEqual(workspaceGeometry.content);
    expect(globalGeometry.categories.y).toEqual(workspaceGeometry.categories.y);
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
      "Search",
      "Telemetry",
      "Defaults",
      "Desktop",
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
