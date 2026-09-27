import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page
    .getByRole("button", { name: "Close thread details", exact: true })
    .click();
});

test("prompt grows for typed lines and pasted text, caps its height, and shrinks after clearing or queueing", async ({
  page,
}) => {
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  const initialHeight = (await prompt.boundingBox())!.height;
  await prompt.focus();
  for (const line of [
    "Review the setup flow.",
    "Check each provider.",
    "Keep the draft intact.",
    "Test model loading.",
    "Report the results.",
  ]) {
    await page.keyboard.type(line);
    await page.keyboard.press("Shift+Enter");
  }
  await expect(prompt).toBeFocused();
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(initialHeight + 30);
  expect(
    await prompt.evaluate(
      (element) => element.scrollHeight - element.clientHeight,
    ),
  ).toBeLessThanOrEqual(1);
  const longDraft = Array.from(
    { length: 80 },
    (_, index) =>
      `Requirement ${index + 1}: keep the composer comfortable for longer messages.`,
  ).join("\n");
  await prompt.fill(longDraft);
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(initialHeight + 100);
  await prompt.press("ControlOrMeta+End");
  await prompt.press("Shift+Enter");
  await page.keyboard.type("Keep this final line and its caret visible.");
  await expect(prompt).toBeFocused();
  await expect
    .poll(() =>
      prompt.evaluate(
        (element) =>
          element.scrollHeight -
          element.scrollTop -
          element.clientHeight -
          Number.parseFloat(getComputedStyle(element).paddingBottom),
      ),
    )
    .toBeLessThanOrEqual(2);
  for (const viewport of [
    { width: 1440, height: 950 },
    { width: 880, height: 640 },
    { width: 480, height: 720 },
  ]) {
    await page.setViewportSize(viewport);
    await expect
      .poll(async () => (await prompt.boundingBox())!.height)
      .toBeLessThanOrEqual(Math.min(320, viewport.height * 0.35) + 1);
    expect(
      await prompt.evaluate(
        (element) => element.scrollHeight > element.clientHeight,
      ),
    ).toBe(true);
    await expect(page.locator(".composer-header")).toBeInViewport({ ratio: 1 });
    await expect(page.locator(".composer-footer")).toBeInViewport({ ratio: 1 });
    await expect(
      page.getByRole("button", { name: "Add message to Next up", exact: true }),
    ).toBeInViewport({ ratio: 1 });
  }
  await prompt.clear();
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeCloseTo(initialHeight, 0);
  await prompt.fill(longDraft);
  await prompt.press("Enter");
  await expect(prompt).toHaveValue("");
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeCloseTo(initialHeight, 0);
});

test("prompt reflows for width and text-size changes and restores the size of an unsent draft", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  const wrappedDraft =
    "Review this change carefully and explain any problems with setup, navigation, or model selection. ".repeat(
      4,
    );
  await prompt.fill(wrappedDraft);
  const wideHeight = (await prompt.boundingBox())!.height;
  await page
    .getByRole("button", { name: "Open files panel", exact: true })
    .click();
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(wideHeight + 20);
  await page
    .getByRole("button", { name: "Close files panel", exact: true })
    .click();
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeCloseTo(wideHeight, 0);
  await page.setViewportSize({ width: 480, height: 950 });
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(wideHeight + 20);
  await page.setViewportSize({ width: 1440, height: 950 });
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeCloseTo(wideHeight, 0);
  await page.evaluate(() =>
    document.documentElement.setAttribute("data-text-size", "large"),
  );
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(wideHeight);
  expect(
    await prompt.evaluate(
      (element) => element.scrollHeight - element.clientHeight,
    ),
  ).toBeLessThanOrEqual(1);
  await page.getByRole("button", { name: /^Model settings:/ }).click();
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(prompt).toHaveValue(wrappedDraft);
  await expect
    .poll(async () => (await prompt.boundingBox())!.height)
    .toBeGreaterThan(wideHeight);
  expect(
    await prompt.evaluate(
      (element) => element.scrollHeight - element.clientHeight,
    ),
  ).toBeLessThanOrEqual(1);
  expect(errors).toEqual([]);
});
