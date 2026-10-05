import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { installWorkflowFixture } from "./support/workflow-fixture";

test.use({ timezoneId: "America/New_York" });
async function capture(page: Page, name: string) {
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS !== "1") return;
  const folder = resolve("../../.local/schedules-polish/after");
  mkdirSync(folder, { recursive: true });
  await page.screenshot({
    path: resolve(folder, name),
    animations: "disabled",
  });
}
async function open(
  page: Page,
  options: { empty?: boolean; modern?: boolean } = {
    empty: true,
    modern: true,
  },
) {
  await installWorkflowFixture(page, options);
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Managed Local worker is running", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toBeVisible();
}
async function typography(page: Page, selector: string) {
  return page.locator(selector).evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      size: parseFloat(style.fontSize),
      family: style.fontFamily,
      weight: style.fontWeight,
    };
  });
}
test("schedule typography matches Providers and responds to text preferences and palettes", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  await expect(page.locator(".catalog-heading h3")).toHaveText("Providers");
  const heading = await typography(page, ".catalog-heading h3");
  const supporting = await typography(page, ".catalog-heading p");
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await capture(page, "01-providers-reference.png");
  await open(page);
  await expect(page.getByRole("article")).toHaveCount(6);
  expect(await typography(page, ".workflow-page-header h2")).toMatchObject({
    size: heading.size,
    family: heading.family,
  });
  expect(
    await typography(page, ".workflow-page-header > div > p:last-child"),
  ).toMatchObject({ size: supporting.size, family: supporting.family });
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await capture(page, "02-schedules-examples-dark.png");
  for (const [preference, scale] of [
    ["compact", 15 / 16],
    ["comfortable", 1],
    ["large", 18 / 16],
  ] as const) {
    await page.evaluate((preference) => {
      document.documentElement.dataset.textSize = preference;
    }, preference);
    expect(
      (await typography(page, ".workflow-page-header h2")).size,
    ).toBeCloseTo(heading.size * scale);
    expect(
      (await typography(page, ".workflow-page-header > div > p:last-child"))
        .size,
    ).toBeCloseTo(supporting.size * scale);
    await page
      .getByRole("button", { name: "Use Workspace health check example" })
      .click();
    const input = page.getByRole("textbox", { name: "Task name", exact: true });
    expect(
      await input.evaluate((element) => getComputedStyle(element).fontWeight),
    ).toBe("400");
    expect(
      await input.evaluate((element) =>
        parseFloat(getComputedStyle(element).fontSize),
      ),
    ).toBeCloseTo(14 * scale);
    await page.keyboard.press("Escape");
  }
  for (const theme of ["light", "dark"]) {
    for (const palette of ["colossus", "neutral", "hacker"]) {
      await page.evaluate(
        ({ palette, theme }) => {
          document.documentElement.dataset.palette = palette;
          document.documentElement.dataset.theme = theme;
        },
        { palette, theme },
      );
      const audit = await new AxeBuilder({ page })
        .include(".workflow-surface")
        .withTags(["wcag2a", "wcag2aa"])
        .analyze();
      expect(audit.violations).toEqual([]);
      await capture(
        page,
        `03-schedules-large-${theme === "dark" ? "" : "light-"}${palette}.png`,
      );
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  const overflow = await page
    .locator(".workflow-surface")
    .evaluate((element) => element.scrollWidth > element.clientWidth);
  expect(overflow).toBe(false);
  await capture(page, "04-schedules-compact-large.png");
});

test("examples fill editable task drafts and agent prompts without saving automatically", async ({
  page,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page);
  await page
    .getByRole("button", { name: "Use Workspace health check example" })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Task name", exact: true }),
  ).toHaveValue("Workspace health check");
  await expect(
    page.getByRole("textbox", { name: "Instructions", exact: true }),
  ).toHaveValue(/Do not edit files/u);
  await expect(
    page.getByRole("combobox", { name: "Repeat", exact: true }),
  ).toContainText("Daily");
  await capture(page, "05-example-task-draft.png");
  await page.getByText("Advanced", { exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: /Allowed tools/u }),
  ).toHaveValue("git.status, git.diff");
  await capture(page, "05b-example-task-advanced.png");
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", { name: "Use Release notes draft example" })
    .click();
  await expect(
    page.getByRole("combobox", { name: "Repeat", exact: true }),
  ).toContainText("Weekly");
  await expect(
    page.getByRole("checkbox", { name: "Fri", exact: true }),
  ).toBeChecked();
  await expect(page.getByLabel("Time", { exact: true })).toHaveValue("16:00");
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", {
      name: "Agent prompt for Cybersecurity market briefing",
    })
    .click();
  const prompt = page.getByRole("textbox", {
    name: "Agent prompt",
    exact: true,
  });
  await expect(prompt).toHaveValue(/@colossus\/schedule-task/u);
  const value = await prompt.inputValue();
  expect(value).toContain("workflow.task.schedule");
  expect(value).toContain("America/New_York");
  await page.keyboard.press("Tab");
  await expect(prompt).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Close", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Copy agent prompt", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toContainText("Prompt copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(value);
  await capture(page, "06-agent-scheduling-prompt.png");
  const audit = await new AxeBuilder({ page })
    .include(".workflow-dialog")
    .analyze();
  expect(audit.violations).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", {
      name: "Agent prompt for Cybersecurity market briefing",
    }),
  ).toBeFocused();
  await expect(
    page.getByRole("heading", { name: "No schedules in this Workspace" }),
  ).toBeVisible();
});

test("schedule inventory uses the settings table and filters loaded schedules", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page, { modern: true });
  const table = page.getByRole("table", { name: "Schedules", exact: true });
  await expect(table).toBeVisible();
  await page
    .getByRole("textbox", { name: "Search loaded schedules", exact: true })
    .fill("hourly-health");
  await expect(table.getByRole("button")).toHaveCount(1);
  await table.getByRole("button", { name: /hourly-health/u }).click();
  await expect(
    page.getByRole("region", { name: "Selected schedule" }),
  ).toContainText("hourly-health");
  await capture(page, "07-schedule-inventory.png");
  await page.getByRole("button", { name: "Back to schedules" }).click();
  await page
    .getByRole("textbox", { name: "Search loaded schedules", exact: true })
    .fill("no-match");
  await expect(table).toContainText("No loaded schedules match");
});

test("examples respect unavailable task scheduling capabilities", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page, { empty: true, modern: false });
  await expect(
    page.getByRole("button", { name: "Use Workspace health check example" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", {
      name: "Agent prompt for Workspace health check",
    }),
  ).toBeEnabled();
});
