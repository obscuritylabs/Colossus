import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const width of [1280, 700]) {
  test(`MCP diagnostics remain readable with many tools at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ colorScheme: width === 1280 ? "dark" : "light" });
    await page.goto("/?fixture=operations-studio");
    if (width < 980) {
      await page.getByRole("button", { name: "Open work navigation" }).click();
    }
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByRole("button", { name: "Review and apply r4", exact: true })
      .click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "MCP", exact: true })
      .click();
    const server = page
      .locator(".managed-mcp-resource")
      .filter({ hasText: "github-local" });
    await server.getByRole("button", { name: "Test", exact: true }).click();
    const diagnostic = server.locator(".mcp-diagnostic-detail");
    await expect(
      diagnostic.getByText("Connection healthy", { exact: true }),
    ).toBeVisible();
    await expect(
      diagnostic.getByText("45 allowlisted tools discovered."),
    ).toBeVisible();
    await expect(
      diagnostic.getByText("The MCP subprocess manages its own TLS trust."),
    ).toBeVisible();
    const tools = diagnostic.getByRole("list", {
      name: "github-local discovered tools",
    });
    const report = diagnostic.getByLabel(
      "github-local connection diagnostic report",
    );
    await expect(tools).toBeHidden();
    await expect(report).toBeHidden();
    expect((await diagnostic.boundingBox())!.height).toBeLessThan(430);
    await page.screenshot({
      path: `output/playwright/mcp-diagnostics-summary-${width}.png`,
    });

    await diagnostic.getByText("Discovered tools", { exact: true }).click();
    await expect(tools.getByRole("listitem")).toHaveCount(45);
    const toolBounds = (await tools.boundingBox())!;
    expect(toolBounds.height).toBeLessThanOrEqual(240);
    await tools.focus();
    await page.keyboard.press("End");
    await expect(
      tools.getByText("update_pull_request", { exact: true }),
    ).toBeInViewport();

    await diagnostic
      .getByText("Connection diagnostics", { exact: true })
      .click();
    await expect(report).toBeVisible();
    const reportBounds = (await report.boundingBox())!;
    expect(reportBounds.width).toBeGreaterThan(
      (await diagnostic.boundingBox())!.width * 0.8,
    );
    const expandedToolBounds = (await tools.boundingBox())!;
    expect(reportBounds.y).toBeGreaterThan(
      expandedToolBounds.y + expandedToolBounds.height,
    );
    expect(reportBounds.height).toBeLessThanOrEqual(320);
    for (const selector of [
      ".settings-main",
      ".mcp-diagnostic-detail",
      ".mcp-health-tools",
    ]) {
      const surface =
        selector === ".settings-main"
          ? page.locator(selector)
          : server.locator(selector);
      const overflow = await surface.evaluate(
        (element) => element.scrollWidth - element.clientWidth,
      );
      expect(
        overflow,
        `${selector} has no horizontal overflow`,
      ).toBeLessThanOrEqual(1);
    }
    const accessibility = await new AxeBuilder({ page })
      .include(".mcp-diagnostic-detail")
      .analyze();
    expect(
      accessibility.violations.filter((item) =>
        ["critical", "serious"].includes(item.impact ?? ""),
      ),
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/mcp-diagnostics-${width}.png`,
    });
  });
}
