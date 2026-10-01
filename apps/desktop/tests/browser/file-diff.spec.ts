import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { openWorkspaceWithPausedClock } from "./support/paused-workspace";

async function openDiff(page: Page) {
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await page
    .getByRole("button", {
      name: "src/components/WorkSurface.tsx, Staged: modified",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "View staged diff", exact: true })
    .click();
}

test("staged, unstaged and commit snapshots keep separate tabs", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await openDiff(page);
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.getByRole("button", { name: /Open Git:/ }).click();
  await page
    .getByRole("button", {
      name: "src/components/WorkSurface.tsx, Unstaged: modified",
      exact: true,
    })
    .click();
  await page
    .getByRole("button", { name: "View unstaged diff", exact: true })
    .click();
  const diff = page.getByRole("region", { name: "File diff", exact: true });
  await expect(diff).toContainText("'Working'");
  const tabs = page.getByRole("navigation", { name: "Open files" });
  await tabs
    .getByRole("button", { name: "WorkSurface.tsx · staged", exact: true })
    .click();
  await expect(diff).toContainText("'Staged'");
  await page
    .getByRole("button", { name: "Close tool pane", exact: true })
    .click();
  await page.getByRole("button", { name: /Open Git:/ }).click();
  const git = page.getByRole("region", { name: "Workspace Git", exact: true });
  await git.getByRole("tab", { name: "History", exact: true }).click();
  await git.getByRole("button", { name: /Improve desktop setup/ }).click();
  await git
    .getByRole("button", {
      name: "View commit diff for src/components/WorkSurface.tsx",
      exact: true,
    })
    .click();
  await expect(diff).toContainText("Compared with the first parent");
  await expect(
    tabs.getByRole("button", { name: /^Close WorkSurface.tsx/ }),
  ).toHaveCount(3);
});

test("the selected file tab stays visible when tabs overflow or the panel narrows", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page
    .getByRole("button", { name: "Open files panel", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Expand tool pane", exact: true })
    .click();
  const search = page.getByRole("searchbox", {
    name: "Find files by name or path",
  });
  const results = page.getByRole("navigation", { name: "File search results" });
  const tabs = page.getByRole("navigation", { name: "Open files" });
  for (const path of [
    "apps/desktop/src-tauri/src/workspace_files.rs",
    "docs/develop/security-architecture.md",
    "apps/desktop/src/components/ProductRail.tsx",
    "apps/desktop/src/components/WorkSidebar.tsx",
  ]) {
    await search.fill(path);
    await results.getByRole("button").click();
    await expect(
      tabs.getByRole("button", { name: path.split("/").at(-1)!, exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
  }
  const activeTab = tabs.getByRole("button", {
    name: "WorkSidebar.tsx",
    exact: true,
  });
  // Allow subpixel rounding at the scroll container edge.
  await expect(activeTab.locator("..")).toBeInViewport({ ratio: 0.99 });
  await expect(results.getByRole("button")).toBeFocused();
  await page.setViewportSize({ width: 880, height: 640 });
  await expect(activeTab.locator("..")).toBeInViewport({ ratio: 0.99 });
});

test("diffs expand, navigate changes and restore a preserved chat draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Preserve this draft");
  await openDiff(page);
  const diff = page.getByRole("region", { name: "File diff", exact: true });
  await expect(diff).toContainText("HEAD → index");
  await expect(diff).toContainText("'Staged'");
  await expect(prompt).toBeHidden();
  await diff.getByRole("button", { name: "Next change", exact: true }).click();
  await expect(
    diff.getByRole("region", { name: "Change 2", exact: true }),
  ).toBeFocused();
  await expect(
    diff.getByRole("button", { name: "Next change", exact: true }),
  ).toBeDisabled();
  await diff.getByRole("button", { name: "Side by side", exact: true }).click();
  await expect(diff.locator(".diff-split-row")).toHaveCount(6);
  await diff
    .getByRole("button", { name: "Reveal in explorer", exact: true })
    .click();
  const tree = page.getByRole("navigation", {
    name: "Workspace tree",
    exact: true,
  });
  await expect(
    tree.getByRole("button", { name: "WorkSurface.tsx", exact: true }),
  ).toBeFocused();
  await diff.getByRole("button", { name: "Current file", exact: true }).click();
  await expect(
    page
      .getByRole("navigation", { name: "Open files" })
      .getByRole("button", { name: "WorkSurface.tsx", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page
    .getByRole("button", { name: "Close WorkSurface.tsx", exact: true })
    .click();
  await expect(diff).toBeVisible();
  await page
    .getByRole("button", { name: "Restore tool pane", exact: true })
    .click();
  await expect(prompt).toHaveValue("Preserve this draft");
  const resize = page.getByRole("separator", { name: "Resize files panel" });
  const width = Number(await resize.getAttribute("aria-valuenow"));
  await resize.press("ArrowRight");
  await expect(resize).not.toHaveAttribute("aria-valuenow", String(width));
  await page
    .getByRole("button", { name: "Expand tool pane", exact: true })
    .click();
  await diff.getByRole("button", { name: "Side by side", exact: true }).click();
  await page.screenshot({ path: "output/playwright/file-diff-dark.png" });
});

test("file search finds unopened descendants, bounds broad results, and reveals paths", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio&files=many");
  await page
    .getByRole("button", { name: "Open files panel", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Expand tool pane", exact: true })
    .click();
  const search = page.getByRole("searchbox", {
    name: "Find files by name or path",
  });
  await search.fill("FEATURE-4999/");
  const results = page.getByRole("navigation", { name: "File search results" });
  await expect(results.getByRole("button")).toHaveCount(1);
  await results.getByRole("button").press("Enter");
  await expect(
    page.getByRole("region", { name: "File preview", exact: true }),
  ).toContainText("src/features/feature-4999/view.tsx");
  await search.fill("feature-");
  await expect(results.getByRole("button")).toHaveCount(200);
  await expect(results).toContainText("Search limit reached");
  await search.fill("src-tauri/src/workspace_files.rs");
  await results.getByRole("button").click();
  await page
    .getByRole("button", { name: "Reveal in explorer", exact: true })
    .click();
  const tree = page.getByRole("navigation", { name: "Workspace tree" });
  const file = tree.getByRole("button", {
    name: "workspace_files.rs",
    exact: true,
  });
  await expect(file).toBeFocused();
  await page.getByRole("button", { name: "Collapse folders" }).click();
  await expect(file).toHaveCount(0);
  await tree
    .getByRole("button", { name: "apps", exact: true })
    .press("ArrowRight");
  await expect(
    tree.getByRole("button", { name: "desktop", exact: true }),
  ).toBeVisible();
  await tree
    .getByRole("button", { name: "apps", exact: true })
    .press("ArrowLeft");
  await expect(
    tree.getByRole("button", { name: "desktop", exact: true }),
  ).toHaveCount(0);
  await search.fill("no-such-file");
  await expect(results).toContainText("0 files found");
});

for (const [scenario, message] of [
  ["binary", "Binary or non-displayable text"],
  ["large", "Preview limit exceeded"],
  ["empty", "New empty file."],
  ["deleted", "File deleted. Showing the previous version."],
  ["limited", "This diff is truncated"],
  ["error", "Git changed during comparison"],
] as const) {
  test(`diff explains ${scenario} content`, async ({ page }) => {
    await page.goto(`/?fixture=operations-studio&diff=${scenario}`);
    await openDiff(page);
    await expect(
      page.getByRole("region", { name: "File preview", exact: true }),
    ).toContainText(message);
    if (scenario === "error") {
      await page.getByRole("button", { name: "Retry preview" }).click();
      await expect(page.getByRole("alert")).toContainText(message);
    }
  });
}

test("switching workspaces discards a pending diff and its tabs", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await openWorkspaceWithPausedClock(
    page,
    "/?fixture=operations-studio&diff=slow",
  );
  await openDiff(page);
  await expect(
    page.getByText("Opening WorkSurface.tsx…", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Research Lab", exact: true }).click();
  await expect(
    page.getByRole("navigation", { name: "Open files" }).getByRole("button"),
  ).toHaveCount(2);
  // Allow the old fixture request to complete; it must not appear in this workspace.
  await expect(async () => {
    await expect(
      page.getByRole("region", { name: "File diff", exact: true }),
    ).toHaveCount(0);
    await expect(
      page
        .getByRole("navigation", { name: "Open files" })
        .getByRole("button", { name: "README.md", exact: true }),
    ).toBeVisible();
  }).toPass({ timeout: 2000 });
  await page.clock.runFor(1500);
  await expect(
    page.getByRole("region", { name: "File diff", exact: true }),
  ).toHaveCount(0);
});

for (const theme of ["dark", "light"] as const) {
  test(`compact ${theme} diff has keyboard controls and no accessibility violations`, async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await page.goto("/?fixture=operations-studio");
    await openDiff(page);
    await page.getByRole("button", { name: "Hide files", exact: true }).click();
    const diff = page.getByRole("region", { name: "File diff", exact: true });
    await diff
      .getByRole("button", { name: "Side by side", exact: true })
      .click();
    await expect(
      diff.getByRole("button", { name: "Current file", exact: true }),
    ).toBeInViewport();
    const results = await new AxeBuilder({ page })
      .include(".enhanced-files")
      .analyze();
    expect(results.violations).toEqual([]);
    await page.screenshot({
      path: `output/playwright/file-diff-compact-${theme}.png`,
    });
  });
}
