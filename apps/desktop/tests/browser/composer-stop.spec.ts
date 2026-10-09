import { expect, test } from "@playwright/test";

test("Stop pauses Next up, preserves the draft, and Resume sends queued messages", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=interaction-question");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  const stop = page.getByRole("button", { name: "Stop response", exact: true });
  await expect(stop).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Add message to Next up" }),
  ).toHaveCount(0);
  await prompt.fill("Inspect the Windows path next");
  await page.getByRole("button", { name: "Add message to Next up" }).click();
  const queue = page.getByRole("region", { name: "Next up", exact: true });
  await expect(queue).toContainText("Inspect the Windows path next");
  await prompt.fill("Keep this unsent draft");
  await stop.click();
  await expect(stop).toHaveCount(0);
  await expect(queue.getByRole("status")).toContainText("Paused");
  await expect(prompt).toHaveValue("Keep this unsent draft");
  await expect(
    queue.getByRole("button", { name: "Resume queue" }),
  ).toBeEnabled();
  // Adding another message while paused must not start either one.
  await page.getByRole("button", { name: "Add message to Next up" }).click();
  await expect(queue.locator("li")).toHaveCount(2);
  await expect(queue.getByRole("status")).toContainText("Paused");
  await prompt.fill("Another unsent draft");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Back to work", exact: true }).click();
  await expect(queue.getByRole("status")).toContainText("Paused");
  await queue.getByRole("button", { name: "Resume queue" }).click();
  await expect(queue).toHaveCount(0);
  await expect(
    page.getByText("Inspect the Windows path next", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Keep this unsent draft", { exact: true }),
  ).toBeVisible();
  await expect(prompt).toHaveValue("Another unsent draft");
});

test("Stop remains available for an oversized draft and reduced motion disables its glow animation", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?fixture=interaction-question");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("\u00e9".repeat(32769));
  const stop = page.getByRole("button", { name: "Stop response", exact: true });
  await expect(stop).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Add message to Next up" }),
  ).toBeDisabled();
  expect(
    await stop.evaluate((element) => getComputedStyle(element).animationName),
  ).toBe("none");
  await stop.click();
  await expect(prompt).toHaveValue("\u00e9".repeat(32769));
  await expect(stop).toHaveCount(0);
});
