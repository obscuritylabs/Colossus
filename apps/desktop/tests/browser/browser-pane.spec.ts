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
    .getByRole("button", { name: "Close tool pane", exact: true })
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
    .getByRole("button", { name: "Close tool pane", exact: true })
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
  await page.getByRole("button", { name: "Expand tool pane" }).click();
  await expect(
    page.getByRole("region", { name: "Work conversation" }),
  ).not.toBeVisible();
  await page.getByRole("button", { name: "Restore tool pane" }).click();
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
      page.getByRole("button", { name: "Close tool pane", exact: true }),
    ).toBeInViewport();
  }
  const report = await new AxeBuilder({ page })
    .include(".browser-pane")
    .analyze();
  expect(report.violations).toEqual([]);
});

test("browser uses two compact rows and keeps pane actions visible with many tabs", async ({
  page,
}) => {
  const pane = page.getByRole("region", { name: "Browser", exact: true });
  const addTab = page.getByRole("button", {
    name: "New browser tab",
    exact: true,
  });
  for (let index = 0; index < 8; index += 1) {
    await addTab.click();
    await expect(
      page.getByRole("button", { name: "Close tab: New tab", exact: true }),
    ).toHaveCount(index + 1);
    await expect(page.locator(".browser-tab.is-active")).toBeInViewport();
  }
  for (const width of [1440, 880]) {
    await page.setViewportSize({ width, height: 640 });
    await expect(addTab).toBeInViewport();
    await expect(
      page.getByRole("button", { name: "Close tool pane", exact: true }),
    ).toBeInViewport();
    await expect(
      page.getByRole("textbox", { name: "Web address" }),
    ).toBeInViewport();
    const viewport = await page
      .getByLabel("Web page", { exact: true })
      .boundingBox();
    const bounds = await pane.boundingBox();
    expect(viewport!.y - bounds!.y).toBeLessThanOrEqual(100);
  }
  await page
    .getByRole("button", { name: "Clear session", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Browse beside your work" }),
  ).toBeVisible();
});

test("a rejected address remains visible through browser context refresh", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("file:///blocked-preview");
  await address.press("Enter");
  const error = page
    .getByRole("alert")
    .filter({ hasText: "Enter an HTTP or HTTPS address" });
  await expect(error).toBeVisible();
  // Two periodic context reads must not erase the rejected action's message.
  await page.waitForTimeout(1600);
  await expect(error).toBeVisible();
});
