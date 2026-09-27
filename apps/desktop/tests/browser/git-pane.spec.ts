import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("Git shows the branch, grouped changes and history without losing a draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const indicator = page.getByRole("button", {
    name: /Open Git: codex\/desktop-git/,
  });
  await expect(indicator).toBeVisible();
  await page
    .getByRole("textbox", { name: "Prompt", exact: true })
    .fill("Keep my draft while I inspect Git.");
  await indicator.click();
  const pane = page.getByRole("region", { name: "Workspace Git", exact: true });
  await expect(
    pane.getByRole("region", { name: "Staged", exact: true }),
  ).toBeVisible();
  await expect(
    pane.getByRole("region", { name: "Unstaged", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("separator", { name: "Resize Git panel" }),
  ).toBeVisible();
  await pane
    .getByRole("region", { name: "Staged", exact: true })
    .getByRole("button")
    .click();
  await expect(
    pane.getByRole("region", { name: "Changed file details" }),
  ).toContainText("src/components/WorkSurface.tsx");
  await pane.getByRole("tab", { name: "History", exact: true }).click();
  await pane.getByRole("button", { name: /Improve desktop setup/ }).click();
  await expect(
    pane.getByRole("region", { name: "Commit details" }),
  ).toContainText("Affected files");
  await page.getByRole("button", { name: "Close Git panel" }).click();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("Keep my draft while I inspect Git.");
});

for (const [scenario, expected] of [
  ["none", "No Git repository"],
  ["clean", "Working tree clean"],
  ["error", "Git metadata is not readable"],
  ["detached", "Detached"],
]) {
  test(`Git handles ${scenario}`, async ({ page }) => {
    await page.goto(`/?fixture=operations-studio&git=${scenario}`);
    await page.getByRole("button", { name: /Open Git:/ }).click();
    await expect(
      page.getByRole("region", { name: "Workspace Git", exact: true }),
    ).toContainText(expected);
  });
}

test("slow refresh keeps the composer usable and shows progress", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio&git=slow");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await expect(
    page.getByRole("region", { name: "Workspace Git", exact: true }),
  ).toContainText("Refreshing");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Typing while Git refreshes");
  await expect(page.getByRole("tab", { name: "Changes (3)" })).toBeVisible();
  await expect(prompt).toHaveValue("Typing while Git refreshes");
});

test("Git is accessible at desktop and compact widths", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await page.screenshot({
    path: "output/playwright/git-panel.png",
    fullPage: true,
  });
  for (const width of [1440, 880]) {
    await page.setViewportSize({ width, height: 950 });
    await expect(
      page.getByRole("button", { name: "Refresh Git", exact: true }),
    ).toBeInViewport();
    await expect(
      page.getByRole("button", { name: "Close Git panel" }),
    ).toBeInViewport();
    const results = await new AxeBuilder({ page })
      .include(".git-pane")
      .analyze();
    expect(results.violations).toEqual([]);
  }
  await page.getByRole("tab", { name: /Changes/ }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(
    page.getByRole("tab", { name: "History", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("region", { name: "Workspace Git", exact: true }),
  ).not.toBeVisible();
  await expect(page.getByRole("button", { name: /Open Git:/ })).toBeFocused();
});

test("retrying an expired history cursor reloads the first page", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio&git=expired");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await page.getByRole("tab", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Load older commits" }).click();
  await expect(page.getByRole("alert")).toContainText("history page expired");
  await page.getByRole("button", { name: "Retry history" }).click();
  await expect(page.getByRole("alert")).not.toBeVisible();
  await expect(
    page.getByRole("button", { name: /Improve desktop setup/ }),
  ).toHaveCount(1);
});

test("workspace switching discards an old pending Git refresh", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  await page.goto("/?fixture=operations-studio&git=switch");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await expect(
    page.getByRole("region", { name: "Workspace Git", exact: true }),
  ).toContainText("Refreshing");
  await page.getByRole("button", { name: "Research Lab", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Open Git: research/ }),
  ).toBeVisible();
  await page.clock.runFor(1500);
  await expect(
    page.getByRole("button", { name: /Open Git: research/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Open Git: codex/ }),
  ).toHaveCount(0);
});
