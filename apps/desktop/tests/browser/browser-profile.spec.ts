import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

for (const [theme, palette] of [
  ["light", "colossus"],
  ["dark", "colossus"],
  ["dark", "neutral"],
  ["dark", "hacker"],
] as const) {
  test(`human profile settings in ${theme} ${palette} save and clear explicitly`, async ({
    page,
  }, testInfo) => {
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
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/?fixture=operations-studio");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Global", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Settings sections" })
      .getByRole("button", { name: "Browser", exact: true })
      .click();
    const form = page.getByRole("form", { name: "Workspace browser profile" });
    const remember = form.getByRole("switch", { name: "Remember sign-ins" });
    await expect(remember).not.toBeChecked();
    await remember.check();
    await form
      .getByRole("textbox", { name: "User-Agent" })
      .fill("Colossus-Profile-Fixture/1.0");
    await form.getByRole("button", { name: "Save browser settings" }).click();
    await expect(form.getByRole("status")).toContainText(
      "Browser settings saved",
    );
    await form.getByRole("button", { name: "Reset browser profile…" }).click();
    await form.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(remember).toBeChecked();
    await form.getByRole("button", { name: "Reset browser profile…" }).click();
    await form
      .getByRole("button", { name: "Clear and reset", exact: true })
      .click();
    await expect(remember).not.toBeChecked();
    await expect(form.getByRole("textbox", { name: "User-Agent" })).toHaveValue(
      "",
    );
    await form.screenshot({
      path: testInfo.outputPath("browser-profile-wide.png"),
    });
    await page.setViewportSize({ width: 700, height: 990 });
    await form.scrollIntoViewIfNeeded();
    await form.screenshot({
      path: testInfo.outputPath("browser-profile-narrow.png"),
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(
      (
        await new AxeBuilder({ page })
          .include(".managed-settings-shell")
          .analyze()
      ).violations,
    ).toEqual([]);
  });
}
