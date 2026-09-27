import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

test("Git shows the branch, grouped changes and history without losing a draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const composer = page.getByRole("form", {
    name: "Send a prompt",
    exact: true,
  });
  const indicator = composer.getByRole("button", {
    name: /Open Git: codex\/desktop-git/,
  });
  await expect(indicator).toBeVisible();
  await expect(indicator.getByText("Git", { exact: true })).toBeVisible();
  await expect(
    page
      .locator(".work-surface-header")
      .getByRole("button", { name: /Open Git:/ }),
  ).toHaveCount(0);
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
    .getByRole("button", { name: /WorkSurface.tsx/ })
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
  await expect(indicator).toBeFocused();
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("Keep my draft while I inspect Git.");
});

test("Git stays available on views without a message composer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const views = page.getByRole("navigation", { name: "Session views" });
  for (const name of ["Topology", "Activity"]) {
    await views.getByRole("button", { name, exact: true }).click();
    await expect(
      page.getByRole("form", { name: "Send a prompt", exact: true }),
    ).toHaveCount(0);
    await page
      .locator(".work-surface-header")
      .getByRole("button", { name: /Open Git:/ })
      .click();
    await expect(
      page.getByRole("region", { name: "Workspace Git", exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Close Git panel" }).click();
  }
  await views
    .getByRole("button", { name: "Conversation", exact: true })
    .click();
  await expect(
    page
      .getByRole("form", { name: "Send a prompt", exact: true })
      .getByRole("button", { name: /Open Git:/ }),
  ).toBeVisible();
});

test("Git has its own compact width and preserves a user resize", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const resize = page.getByRole("separator", { name: "Resize Git panel" });
  await expect(resize).toHaveAttribute("aria-valuenow", "360");
  await resize.press("ArrowLeft");
  const width = await resize.getAttribute("aria-valuenow");
  expect(Number(width)).toBeGreaterThan(360);
  await page.getByRole("button", { name: "Open Aside", exact: true }).click();
  const asideResize = page.getByRole("separator", {
    name: "Resize Aside conversation",
  });
  await asideResize.press("End");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await expect(resize).toHaveAttribute("aria-valuenow", width!);
  await resize.dblclick();
  await expect(resize).toHaveAttribute("aria-valuenow", "360");
});

test("file groups collapse, filter by directory, and keep details visible", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio&git=many");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const pane = page.getByRole("region", { name: "Workspace Git", exact: true });
  const group = pane.getByRole("region", { name: "Unstaged", exact: true });
  const toggle = group.getByRole("button", { name: /^Unstaged/ });
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  const filter = pane.getByRole("searchbox", { name: "Filter files" });
  await filter.fill("FEATURE-59");
  const file = group.getByRole("button", { name: /feature-59\/index.ts/ });
  await expect(file).toBeVisible();
  await file.click();
  const inspector = pane.getByRole("region", { name: "Changed file details" });
  await expect(inspector).toBeInViewport();
  await pane.getByRole("button", { name: "Clear filter files" }).click();
  await expect(inspector).toBeInViewport();
  await expect(inspector).toContainText("src/features/feature-59/index.ts");
  await filter.fill("not-a-matching-path");
  await expect(pane.getByText(/No files match/)).toBeVisible();
  await inspector.getByRole("button", { name: "Close file details" }).click();
  await expect(filter).toBeFocused();
});

test("file inspector explains conflicts, deletions and renamed paths", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio&git=many");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const pane = page.getByRole("region", { name: "Workspace Git", exact: true });
  const inspector = pane.getByRole("region", { name: "Changed file details" });
  await pane.getByRole("button", { name: /conflict.tsx, Conflicts/ }).click();
  await expect(inspector).toContainText("Needs resolution");
  await pane
    .getByRole("searchbox", { name: "Filter files" })
    .fill("removed.ts");
  await pane.getByRole("button", { name: /removed.ts, Unstaged/ }).click();
  await expect(inspector).toContainText("This file has been deleted");
  await expect(
    inspector.getByRole("button", { name: "Open current file" }),
  ).toHaveCount(0);
  await pane
    .getByRole("searchbox", { name: "Filter files" })
    .fill("new-name.ts");
  await pane.getByRole("button", { name: /new-name.ts, Staged/ }).click();
  await expect(inspector).toContainText("Renamed from src/old-name.ts");
});

test("history filtering and commit details preserve navigation and focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const pane = page.getByRole("region", { name: "Workspace Git", exact: true });
  await pane.getByRole("tab", { name: "History", exact: true }).click();
  const filter = pane.getByRole("searchbox", { name: "Filter loaded commits" });
  await filter.fill("Desktop team");
  const commit = pane.getByRole("button", { name: /Add browser navigation/ });
  await expect(commit).toBeVisible();
  await expect(
    pane.getByRole("button", { name: /Improve desktop setup/ }),
  ).toHaveCount(0);
  await commit.click();
  const details = pane.getByRole("region", { name: "Commit details" });
  await expect(
    details.getByRole("button", { name: "Back to history" }),
  ).toBeFocused();
  await expect(
    details.getByRole("heading", {
      name: "Add browser navigation and tab controls",
    }),
  ).toBeVisible();
  await expect(details.getByText("Commit metadata")).toBeVisible();
  await expect(details).toContainText(
    "Renamed from src/components/GitPane.tsx",
  );
  await details.getByRole("button", { name: "Back to history" }).click();
  await expect(filter).toHaveValue("Desktop team");
  await expect(commit).toBeFocused();
  await filter.fill("no such commit");
  await expect(pane.getByText(/No loaded commits match/)).toBeVisible();
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
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  await page.goto("/?fixture=operations-studio&git=slow");
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await expect(
    page.getByRole("region", { name: "Workspace Git", exact: true }),
  ).toContainText("Refreshing");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Typing while Git refreshes");
  await page.clock.runFor(1500);
  await expect(page.getByRole("tab", { name: "Changes (3)" })).toBeVisible();
  await expect(prompt).toHaveValue("Typing while Git refreshes");
  await expect(
    page.getByRole("region", { name: "Workspace Git", exact: true }),
  ).toContainText("Auto-refresh on");
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
  await page.clock.runFor(1500);
  await expect(
    page.getByRole("button", { name: /Open Git: research/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Open Git: codex/ }),
  ).toHaveCount(0);
  await expect(page.getByRole("alert")).not.toBeVisible();
});

for (const colorTheme of ["dark", "light"]) {
  test(`Git stays usable in a short ${colorTheme} window with large text`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 880, height: 560 });
    await page.addInitScript((theme) => {
      localStorage.setItem(
        "colossus.desktop.appearance.v1",
        JSON.stringify({
          colorTheme: theme,
          textSize: "large",
          showSecurityWarnings: false,
        }),
      );
    }, colorTheme);
    await page.goto("/?fixture=operations-studio");
    await page.getByRole("button", { name: /Open Git:/ }).click();
    const pane = page.getByRole("region", {
      name: "Workspace Git",
      exact: true,
    });
    const close = pane.getByRole("button", { name: "Close Git panel" });
    await expect(close).toBeFocused();
    const filter = pane.getByRole("searchbox", { name: "Filter files" });
    await filter.fill("missing-path");
    await pane.getByRole("button", { name: "Clear filter files" }).click();
    await pane.getByRole("button", { name: /README.md, Unstaged/ }).click();
    await expect(
      pane.getByRole("button", { name: "Open current file" }),
    ).toBeInViewport();
    expect(
      (await new AxeBuilder({ page }).include(".git-pane").analyze())
        .violations,
    ).toEqual([]);
    await pane.getByRole("tab", { name: "History", exact: true }).click();
    expect(
      (await new AxeBuilder({ page }).include(".git-pane").analyze())
        .violations,
    ).toEqual([]);
    await pane
      .getByRole("button", { name: /Improve desktop setup/ })
      .press("Enter");
    await expect(
      pane.getByRole("button", { name: "Back to history" }),
    ).toBeFocused();
    await pane.getByText("Commit metadata", { exact: true }).click();
    expect(
      (await new AxeBuilder({ page }).include(".git-pane").analyze())
        .violations,
    ).toEqual([]);
    await page.screenshot({
      path: `output/playwright/git-short-${colorTheme}.png`,
    });
  });
}
