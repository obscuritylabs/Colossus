import { expect, type Page } from "@playwright/test";

export async function openWorkspaceWithPausedClock(page: Page, url: string) {
  await page.clock.install();
  await page.clock.pauseAt(new Date());
  await page.goto(url);
  // React can defer a resolved Suspense fallback. Advance only until the
  // workspace mounts so its slow fixture reads remain pending for assertions.
  await expect
    .poll(
      async () => {
        await page.clock.runFor(50);
        return page
          .getByRole("textbox", { name: "Prompt", exact: true })
          .isVisible();
      },
      { timeout: 10_000, intervals: [50] },
    )
    .toBe(true);
}
