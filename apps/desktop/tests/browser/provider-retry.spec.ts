import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("provider recovery counts down in place and leaves Stop available", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.clock.setFixedTime(new Date("2026-10-03T00:00:00Z"));
  await page.goto("/?fixture=provider-retry");
  const status = page.locator(".provider-retry-status");
  await expect(status).toHaveCount(1);
  await expect(status).toContainText("Reconnecting to provider");
  await expect(status).toContainText("Retry 2 of 5");
  await expect(status).toContainText("Next attempt in 4s");
  const stop = page.getByRole("button", { name: "Stop response", exact: true });
  await expect(stop).toBeEnabled();
  await page.clock.setFixedTime(new Date("2026-10-03T00:00:01Z"));
  await expect(status).toContainText("Next attempt in 3s");
  await expect(status).toHaveAttribute("aria-live", "polite");
  const accessibility = await new AxeBuilder({ page })
    .include(".provider-retry-status")
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await page.screenshot({
    path: testInfo.outputPath("provider-retry-desktop.png"),
    fullPage: true,
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(
    await status
      .locator("svg")
      .evaluate((icon) => getComputedStyle(icon).animationName),
  ).toBe("none");
  await stop.click();
  await expect(status).toHaveCount(0);
});

test("provider recovery fits a narrow window without horizontal overflow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 600, height: 800 });
  await page.goto("/?fixture=provider-retry&retry=partial");
  const status = page.locator(".provider-retry-status");
  await expect(status).toBeVisible();
  await expect(page.getByText("Earlier output stays visible.")).toBeVisible();
  expect(
    await status.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
});

test("a confirmed failure can continue the saved conversation and keep the draft", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 600, height: 800 });
  await page.goto("/?fixture=provider-retry&retry=exhausted");
  const retry = page.getByRole("button", { name: "Retry", exact: true });
  const draft = page.getByRole("textbox", { name: "Prompt", exact: true });
  await draft.fill("Keep this unsent draft");
  await expect(retry).toBeEnabled();
  await expect(retry).toHaveAttribute(
    "title",
    "Continue from saved history with the selected conversation model",
  );
  await expect(page.getByText("Provider returned HTTP 500.")).toBeVisible();
  await expect(page.getByText("Earlier output stays visible.")).toBeVisible();
  for (const [theme, palette] of [
    ["light", "colossus"],
    ["dark", "colossus"],
    ["dark", "neutral"],
    ["dark", "black"],
    ["dark", "hacker"],
    ["dark", "forced"],
  ]) {
    await page.evaluate(
      ({ theme, palette }) => {
        document.documentElement.dataset.textSize = "large";
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette =
          palette === "forced" ? "colossus" : palette;
      },
      { theme, palette },
    );
    await page.emulateMedia({
      forcedColors: palette === "forced" ? "active" : "none",
    });
    expect(
      await retry.evaluate((element) => {
        const { left, right } = element.getBoundingClientRect();
        return (
          left >= 0 &&
          right <= innerWidth &&
          element.scrollWidth <= element.clientWidth
        );
      }),
    ).toBe(true);
    if (theme === "light" || palette === "hacker") {
      expect(
        (
          await new AxeBuilder({ page })
            .include(".response-utilities")
            .analyze()
        ).violations,
      ).toEqual([]);
      await retry.scrollIntoViewIfNeeded();
      await page.screenshot({
        path: testInfo.outputPath(
          `provider-retry-exhausted-${theme}-${palette}.png`,
        ),
      });
    }
  }
  await retry.focus();
  await expect(retry).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByText("Continue", { exact: true })).toHaveCount(1);
  await expect(retry).toHaveCount(0);
  await expect(page.getByText("Earlier output stays visible.")).toBeVisible();
  await expect(draft).toHaveValue("Keep this unsent draft");
});

test("an uncertain provider outcome has no Retry action", async ({ page }) => {
  await page.goto("/?fixture=provider-retry&retry=unknown");
  await expect(page.getByText(/external outcome is unknown/i)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);
});

test("provider recovery uses the dark palette with readable contrast", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=provider-retry");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await page.getByRole("combobox", { name: /Color theme/u }).click();
  await page
    .getByRole("listbox")
    .getByRole("option", { name: "Dark", exact: true })
    .click();
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(page.locator(".provider-retry-status")).toBeVisible();
  const accessibility = await new AxeBuilder({ page })
    .include(".provider-retry-status")
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await page.screenshot({
    path: testInfo.outputPath("provider-retry-desktop-dark.png"),
    fullPage: true,
  });
});
