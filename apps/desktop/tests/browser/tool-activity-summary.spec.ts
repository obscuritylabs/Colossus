import { expect, test } from "@playwright/test";

for (const [colorTheme, darkPalette, width] of [
  ["light", "colossus", 960],
  ["dark", "neutral", 760],
  ["dark", "hacker", 760],
] as const) {
  test(`canonical tool summary stays compact and expandable with Large text in ${colorTheme}/${darkPalette}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(
      ({ colorTheme, darkPalette }) => {
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({ colorTheme, darkPalette, textSize: "large" }),
        );
      },
      { colorTheme, darkPalette },
    );
    await page.goto("/?fixture=activity-comparison");
    const row = page.locator(".compact-tool-activity").filter({
      has: page.getByText("Mapped repository structure", { exact: true }),
    });
    await expect(row).toHaveCount(1);
    const thread = page.locator(".run-activity-thread").filter({ has: row });
    if (
      !(await thread.evaluate(
        (element) => (element as HTMLDetailsElement).open,
      ))
    ) {
      await thread.locator(":scope > summary").click();
    }
    const heading = row.locator(":scope > summary");
    await expect(heading).toContainText("Mapped repository structure");
    await expect(heading).toContainText("repo.map_structure");
    await expect(heading.locator(".tabler-icon-folder-open")).toBeVisible();
    await expect(row).toHaveJSProperty("open", false);
    await heading.click();
    await expect(row).toHaveJSProperty("open", true);
    await expect(row.locator(".tool-activity-preview")).toContainText(
      "file_count",
    );
    await heading.focus();
    await expect(heading).toBeFocused();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath("canonical-tool-summary.png"),
      fullPage: true,
    });
  });
}
