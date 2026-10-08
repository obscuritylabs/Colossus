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
async function open(page: Page, options = { empty: true, modern: true }) {
  await installWorkflowFixture(page, options);
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true }),
  ).toBeVisible();
}
async function typography(page: Page, selector: string) {
  return page.locator(selector).evaluate((element) => {
    const style = getComputedStyle(element);
    return { size: parseFloat(style.fontSize), family: style.fontFamily };
  });
}
test("schedule examples match settings typography across text sizes, palettes, and compact widths", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Global", exact: true }).click();
  await page.getByRole("button", { name: "Providers", exact: true }).click();
  const heading = await typography(page, ".catalog-heading h3");
  const supporting = await typography(page, ".catalog-heading p");
  await open(page);
  await expect(page.getByRole("article")).toHaveCount(6);
  expect(await typography(page, ".workflow-page-header h2")).toEqual(heading);
  expect(
    await typography(page, ".workflow-page-header > div > p:last-child"),
  ).toEqual(supporting);
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
    await page.getByText("More options", { exact: true }).click();
    await page
      .getByRole("button", { name: "Schedule a task", exact: true })
      .click();
    const input = page.getByRole("textbox", { name: "Task name", exact: true });
    expect(
      await input.evaluate((element) => getComputedStyle(element).fontWeight),
    ).toBe("400");
    await page.keyboard.press("Escape");
    await page.getByText("More options", { exact: true }).click();
  }
  for (const theme of ["light", "dark"]) {
    for (const palette of ["colossus", "neutral", "hacker"]) {
      await page.evaluate(
        ({ theme, palette }) => {
          document.documentElement.dataset.theme = theme;
          document.documentElement.dataset.palette = palette;
        },
        { theme, palette },
      );
      expect(
        (
          await new AxeBuilder({ page })
            .include(".workflow-surface")
            .withTags(["wcag2a", "wcag2aa"])
            .analyze()
        ).violations,
      ).toEqual([]);
      await capture(page, `examples-${theme}-${palette}.png`);
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page
      .locator(".workflow-surface")
      .evaluate((element) => element.scrollWidth <= element.clientWidth + 1),
  ).toBe(true);
  await capture(page, "examples-compact-large.png");
});

test("an example starts a fresh agent chat with the complete scheduling prompt", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page);
  await page
    .getByRole("button", { name: "Create Workspace health check with agent" })
    .focus();
  await page.keyboard.press("Enter");
  const messages = page.locator(
    'article[data-role="user"] .shared-message-body',
  );
  await expect(messages).toHaveCount(1);
  await expect(messages).toContainText("@colossus/schedule-task");
  await expect(messages).toContainText("Name: Workspace health check");
  await expect(messages).toContainText(
    "Do not edit files, run commands, commit, or publish anything.",
  );
  await expect(messages).toContainText("Repeat: Daily");
  await expect(messages).toContainText("Time zone: America/New_York");
  await expect(messages).toContainText(
    "Allowed task tools: git.status, git.diff",
  );
  await expect(messages).toContainText("workflow.task.schedule");
  await expect(
    page.getByRole("textbox", { name: "Prompt", exact: true }),
  ).toHaveValue("");
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { workflowCalls: { command: string }[] }
      ).workflowCalls.filter(
        (call) => call.command === "create_workflow_schedule",
      ),
    ),
  ).toHaveLength(0);
});

test("custom creation starts an agent chat without picking an unrelated example", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page);
  await page
    .getByRole("button", { name: "Create with agent", exact: true })
    .click();
  const message = page.locator(
    'article[data-role="user"] .shared-message-body',
  );
  await expect(message).toContainText(
    "Ask what I want the agent to do and when it should repeat.",
  );
  await expect(message).not.toContainText("Cybersecurity market briefing");
});

test("saved schedules show inventory and examples open in a separate view", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page, { empty: false, modern: true });
  const table = page.getByRole("table", { name: "Schedules", exact: true });
  await expect(table).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toHaveCount(0);
  await capture(page, "schedule-inventory.png");
  await page
    .getByRole("textbox", { name: "Search loaded schedules", exact: true })
    .fill("hourly-health");
  await expect(table.getByRole("button")).toHaveCount(1);
  await page.getByRole("button", { name: "Browse examples" }).click();
  await expect(table).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to schedules" }).click();
  await expect(table).toBeVisible();
});

test("examples respect unavailable task scheduling capabilities", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1100 });
  await open(page, { empty: true, modern: false });
  await expect(
    page.getByRole("button", {
      name: "Create Workspace health check with agent",
    }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Create with agent", exact: true }),
  ).toBeDisabled();
});
