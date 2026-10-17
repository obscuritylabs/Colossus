import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "colossus"],
  ["dark", "neutral"],
  ["dark", "hacker"],
]) {
  test(`browser settings remain usable in ${theme} ${palette} at large text`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 700, height: 990 });
    await page.addInitScript(
      ({ theme, palette }) => {
        localStorage.setItem(
          "colossus.desktop.appearance.v1",
          JSON.stringify({
            colorTheme: theme,
            darkPalette: palette,
            textSize: "large",
          }),
        );
      },
      { theme, palette },
    );
    await page.goto("/?fixture=operations-studio");
    await page.getByRole("button", { name: "Open work navigation" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Browser", exact: true })
      .click();
    await expect(
      page.getByRole("checkbox", { name: "Browser access" }),
    ).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute(
      "data-text-size",
      "large",
    );
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme!);
    await expect(page.locator("html")).toHaveAttribute(
      "data-palette",
      theme === "light" ? "colossus" : palette!,
    );
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    const audit = await new AxeBuilder({ page })
      .include(".managed-settings-shell")
      .analyze();
    expect(audit.violations).toEqual([]);
    await page.screenshot({
      path: `output/playwright/browser-settings-${theme}-${palette}.png`,
      fullPage: true,
    });
  });
}
