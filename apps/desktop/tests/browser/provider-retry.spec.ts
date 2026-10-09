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
  await page.goto("/?fixture=provider-retry");
  const status = page.locator(".provider-retry-status");
  await expect(status).toBeVisible();
  expect(
    await status.evaluate(
      (element) => element.scrollWidth <= element.clientWidth,
    ),
  ).toBe(true);
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
