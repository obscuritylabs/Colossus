import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "colossus"],
  ["dark", "neutral"],
  ["dark", "hacker"],
]) {
  test(`computer settings in ${theme} ${palette} preserve disabled defaults and workspace inheritance`, async ({
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
      .getByRole("button", { name: "Computer use", exact: true })
      .click();
    const access = page.getByRole("switch", { name: "Computer access" });
    await expect(access).not.toBeChecked();
    await expect(
      page.getByText("@computer-use", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(/including actions outside the workspace/),
    ).toBeVisible();
    await expect(
      page.getByRole("spinbutton", { name: "Session timeout (ms)" }),
    ).toHaveValue("300000");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(
      (
        await new AxeBuilder({ page })
          .include(".managed-settings-shell")
          .analyze()
      ).violations,
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/computer-settings-${theme}-${palette}.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.getByRole("button", { name: "Workspace", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Computer use", exact: true })
      .click();
    await access.focus();
    await page.keyboard.press("Space");
    await expect(access).toBeChecked();
    const group = page.getByRole("group", {
      name: "Computer access",
      exact: true,
    });
    await group.getByRole("button", { name: "Inherit" }).focus();
    await page.keyboard.press("Enter");
    await expect(access).not.toBeChecked();
    await expect(access).toBeFocused();
    expect(
      (
        await new AxeBuilder({ page })
          .include(".managed-settings-shell")
          .analyze()
      ).violations,
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/computer-settings-workspace-${theme}-${palette}.png`,
      fullPage: true,
    });
  });
}
