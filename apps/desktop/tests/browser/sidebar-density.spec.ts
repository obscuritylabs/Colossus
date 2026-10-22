import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

const longTitle =
  "Review the shared workspace layout and keep this complete chat title visible";

test("Desktop chat rows keep one title line and keyboard actions at every text size", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/?fixture=operations-studio");
  const initialAction = page.getByRole("button", {
    name: "Thread actions for Audit ipc boundary",
    exact: true,
  });
  await expect(initialAction).toBeEnabled();
  await initialAction.press("ArrowDown");
  await page
    .getByRole("menuitem", { name: "Rename Audit ipc boundary", exact: true })
    .press("Enter");
  await page
    .getByRole("textbox", { name: "Thread name", exact: true })
    .fill(longTitle);
  await page
    .getByRole("button", { name: "Save thread name", exact: true })
    .click();
  const sidebar = page.locator(".work-sidebar");
  const row = sidebar.getByRole("button", {
    name: new RegExp(`^${longTitle} Execute`, "u"),
  });
  const action = sidebar.getByRole("button", {
    name: `Thread actions for ${longTitle}`,
    exact: true,
  });
  for (const textSize of ["compact", "comfortable", "large"]) {
    await page.evaluate((size) => {
      document.documentElement.dataset.textSize = size;
    }, textSize);
    await expect(row).toBeVisible();
    await expect(row).toHaveAttribute("title", new RegExp(longTitle, "u"));
    await expect(row).toHaveAttribute("title", /Execute/u);
    const title = row.locator(".shared-workspace-thread-copy > strong");
    const [geometry, rowBounds, actionBounds] = await Promise.all([
      title.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return {
          height: bounds.height,
          right: bounds.right,
          lineHeight: Number.parseFloat(style.lineHeight),
          clientWidth: element.clientWidth,
          scrollWidth: element.scrollWidth,
          textOverflow: style.textOverflow,
        };
      }),
      row.boundingBox(),
      action.boundingBox(),
    ]);
    expect(rowBounds!.height).toBeLessThanOrEqual(44);
    expect(geometry.height).toBeLessThanOrEqual(geometry.lineHeight + 1);
    expect(geometry.scrollWidth).toBeGreaterThan(geometry.clientWidth);
    expect(geometry.textOverflow).toBe("ellipsis");
    expect(geometry.right).toBeLessThanOrEqual(actionBounds!.x);
    await row.focus();
    await page.keyboard.press("Tab");
    await expect(action).toBeFocused();
  }
  await action.press("ArrowDown");
  const rename = page.getByRole("menuitem", {
    name: `Rename ${longTitle}`,
    exact: true,
  });
  await expect(rename).toBeFocused();
  const accessibility = await new AxeBuilder({ page })
    .include(".thread-actions-content")
    .analyze();
  expect(
    accessibility.violations.filter((violation) =>
      ["critical", "serious"].includes(violation.impact ?? ""),
    ),
  ).toEqual([]);
  await rename.press("Escape");
  await expect(action).toBeFocused();
  await sidebar.screenshot({
    path: "output/playwright/sidebar-compact-desktop-large.png",
  });
});
