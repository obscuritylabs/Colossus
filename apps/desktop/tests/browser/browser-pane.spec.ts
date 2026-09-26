import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
});

test("browser loading leaves the composer usable and exposes stop", async ({
  page,
}) => {
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("https://example.com/docs");
  await address.press("Enter");
  await expect(
    page.getByRole("button", { name: "Stop loading", exact: true }),
  ).toBeEnabled();
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep this draft while I inspect the website.");
  await page.getByRole("button", { name: "Stop loading", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Reload page", exact: true }),
  ).toBeEnabled();
  await expect(prompt).toHaveValue(
    "Keep this draft while I inspect the website.",
  );
});

test("tabs survive closing the pane and switching app surfaces", async ({
  page,
}) => {
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("https://example.com/docs");
  await address.press("Enter");
  await expect(address).toHaveValue("https://example.com/docs");
  await page
    .getByRole("button", { name: "Close browser pane", exact: true })
    .click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  await expect(address).toHaveValue("https://example.com/docs");
  await page
    .getByRole("button", { name: "New browser tab", exact: true })
    .click();
  await expect(address).toHaveValue("");
  await page
    .getByRole("button", { name: "Close tab: New tab", exact: true })
    .click();
  await expect(address).toHaveValue("https://example.com/docs");
  await page
    .getByRole("button", { name: "Clear session", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Browse beside your work" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open in system browser" }),
  ).toBeDisabled();
});

test("a failed page has retry controls and does not cover the app", async ({
  page,
}) => {
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("https://failure.test/");
  await address.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Unable to display this page" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeEnabled();
  await page
    .getByRole("textbox", { name: "Prompt", exact: true })
    .fill("The conversation still works.");
  await page
    .getByRole("button", { name: "Close browser pane", exact: true })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("The conversation still works.");
});

test("browser pane expands, resizes, and has accessible controls", async ({
  page,
}) => {
  const pane = page.getByRole("region", { name: "Browser", exact: true });
  await expect
    .poll(async () => (await pane.boundingBox())?.width ?? 0)
    .toBeGreaterThan(450);
  await expect(
    page.getByRole("separator", { name: "Resize browser pane" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Expand browser pane" }).click();
  await expect(
    page.getByRole("region", { name: "Work conversation" }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Restore browser pane" }).click();
  await expect(
    page.getByRole("region", { name: "Work conversation" }),
  ).toBeVisible();
  for (const width of [1440, 880]) {
    await page.setViewportSize({ width, height: 950 });
    await expect(pane).toBeVisible();
    await expect(
      page.getByRole("textbox", { name: "Web address" }),
    ).toBeInViewport();
    await expect(
      page.getByRole("button", { name: "Close browser pane", exact: true }),
    ).toBeInViewport();
  }
  const report = await new AxeBuilder({ page })
    .include(".browser-pane")
    .analyze();
  expect(report.violations).toEqual([]);
});
