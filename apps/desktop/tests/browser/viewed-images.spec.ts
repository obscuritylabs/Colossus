import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const fixture = "/?fixture=activity-comparison&imageViews=1";

test("viewed images expand to thumbnails, open by keyboard, and survive reload", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(fixture);
  const row = page.locator(".activity-kind-image");
  const summary = row.locator("summary");
  await expect(summary).toContainText("Viewed 5 images");
  await expect(row.getByRole("list", { name: "Viewed images" })).toHaveCount(0);
  await summary.focus();
  await page.keyboard.press("Enter");
  const thumbnails = row.locator("img");
  await expect(thumbnails).toHaveCount(5);
  await expect
    .poll(() =>
      thumbnails.evaluateAll((images) =>
        images.every(
          (image) =>
            (image as HTMLImageElement).complete &&
            (image as HTMLImageElement).naturalWidth > 0,
        ),
      ),
    )
    .toBe(true);
  const button = row.getByRole("button", {
    name: "Open image 1: sample-screen-1.png",
    exact: true,
  });
  await button.focus();
  await page.keyboard.press("Enter");
  const preview = page.getByRole("dialog", {
    name: "Image preview: sample-screen-1.png",
  });
  await expect(preview.getByRole("img")).toBeVisible();
  const accessibility = await new AxeBuilder({ page })
    .include(".viewed-image-dialog")
    .analyze();
  expect(
    accessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);
  await expect(button).toBeFocused();
  await page.reload();
  await expect(summary).toContainText("Viewed 5 images");
  await summary.click();
  await expect(thumbnails).toHaveCount(5);
  await page.screenshot({
    path: "output/playwright/viewed-images-desktop.png",
    fullPage: true,
  });
});

test("image preview failures keep filenames and a clear unavailable state", async ({
  page,
}) => {
  await page.goto(`${fixture}&imagePreviewError=1`);
  const row = page.locator(".activity-kind-image");
  await row.locator("summary").click();
  await expect(
    row.getByText("Preview unavailable", { exact: true }),
  ).toHaveCount(5);
  await row
    .getByRole("button", {
      name: "Open image 1: sample-screen-1.png",
      exact: true,
    })
    .click();
  const preview = page.getByRole("dialog");
  await expect(preview.getByRole("status")).toHaveText(
    "Image preview unavailable.",
  );
  await preview.getByRole("button", { name: "Close image preview" }).click();
  await expect(preview).toHaveCount(0);
});

test("session Activity renders viewed images in the Result inspector", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`${fixture}&view=activity`);
  const inspector = page.getByRole("complementary", {
    name: "Activity inspector",
  });
  await expect(inspector).toContainText("Viewed 5 images");
  await inspector.getByRole("button", { name: "Result", exact: true }).click();
  await expect(inspector.locator("img")).toHaveCount(5);
  await inspector
    .getByRole("button", {
      name: "Open image 2: sample-screen-2.png",
      exact: true,
    })
    .click();
  const preview = page.getByRole("dialog", {
    name: "Image preview: sample-screen-2.png",
  });
  await expect(preview.getByRole("img")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(preview).toHaveCount(0);
});

for (const [theme, palette, width] of [
  ["light", "colossus", 880],
  ["dark", "neutral", 880],
  ["dark", "hacker", 880],
] as const) {
  test(`viewed images fit ${theme} ${palette} at ${width}px and Large text`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 800 });
    await page.goto(fixture);
    await page.evaluate(
      ({ theme, palette }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.dataset.palette = palette;
        document.documentElement.dataset.textSize = "large";
      },
      { theme, palette },
    );
    const row = page.locator(".activity-kind-image");
    await row.locator("summary").click();
    await expect(row.locator("img")).toHaveCount(5);
    const geometry = await row.evaluate((element) => ({
      width: element.getBoundingClientRect().width,
      right: element.getBoundingClientRect().right,
      pageWidth: document.documentElement.scrollWidth,
    }));
    expect(geometry.width).toBeGreaterThan(100);
    expect(geometry.right).toBeLessThanOrEqual(width);
    expect(geometry.pageWidth).toBe(width);
    await row.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: `output/playwright/viewed-images-${theme}-${palette}-${width}.png`,
    });
  });
}
