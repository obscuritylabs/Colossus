import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { installWorkflowFixture } from "./support/workflow-fixture";

test.use({ timezoneId: "America/New_York" });
async function open(page: Page, modern = true, completed = false) {
  await installWorkflowFixture(page, { modern });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
  if (completed)
    await page.evaluate(() => {
      (window as unknown as { workflowRunStatus: string }).workflowRunStatus =
        "completed";
    });
  if (
    !(await page
      .getByRole("button", { name: "Schedules", exact: true })
      .isVisible())
  )
    await page
      .getByRole("button", { name: "Open work navigation", exact: true })
      .click();
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page
    .getByRole("button", { name: /hourly-health workspace-health/ })
    .click();
  await expect(
    page.getByRole("button", { name: "Back to schedules" }),
  ).toBeVisible();
}
async function capture(page: Page, name: string) {
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS !== "1") return;
  const folder = resolve("../../.local/schedule-details/after");
  mkdirSync(folder, { recursive: true });
  await page.screenshot({
    path: resolve(folder, name),
    animations: "disabled",
  });
}
test("schedule detail is a dedicated page with output on the right and keyboard return", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await open(page, true, true);
  await expect(
    page.getByRole("table", { name: "Schedules", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toHaveCount(0);
  const output = page.getByRole("complementary", {
    name: "Selected run output",
  });
  await expect(output).toContainText('"ok":true');
  const detail = await page
    .getByRole("region", { name: "Selected schedule" })
    .boundingBox();
  const box = await output.boundingBox();
  expect(box!.x).toBeGreaterThan(detail!.x + detail!.width);
  expect(Math.abs(box!.y - detail!.y)).toBeLessThan(2);
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await capture(page, "01-schedule-detail-dark.png");
  expect(
    (await new AxeBuilder({ page }).include(".workflow-surface").analyze())
      .violations,
  ).toEqual([]);
  await page.getByRole("button", { name: "Back to schedules" }).click();
  await expect(
    page.getByRole("table", { name: "Schedules", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true }),
  ).toBeFocused();
});
test("task deletion requires confirmation, uses the reviewed revision and returns to inventory", async ({
  page,
}) => {
  await installWorkflowFixture(page, { modern: true, empty: true });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page.getByText("More options", { exact: true }).click();
  await page
    .getByRole("button", { name: "Schedule a task", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Task name", exact: true })
    .fill("Workspace health check");
  await page
    .getByRole("textbox", { name: "Instructions", exact: true })
    .fill("Inspect workspace health and return a report.");
  await page.getByRole("button", { name: "Review task", exact: true }).click();
  await page
    .getByRole("button", { name: "Create task schedule", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Workspace health check",
      exact: true,
      level: 2,
    }),
  ).toBeFocused();
  await expect(
    page.getByRole("table", { name: "Schedules", exact: true }),
  ).toHaveCount(0);
  await capture(page, "02-task-detail.png");
  const remove = page.getByRole("button", { name: "Delete task", exact: true });
  await remove.click();
  await expect(page.getByRole("dialog")).toContainText(
    "run history is retained",
  );
  await capture(page, "03-delete-task-review.png");
  await page.keyboard.press("Escape");
  await expect(remove).toBeFocused();
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { workflowCalls: { command: string }[] }
      ).workflowCalls.filter(
        (call) => call.command === "delete_workflow_schedule",
      ),
    ),
  ).toHaveLength(0);
  await remove.click();
  await page.getByRole("button", { name: "Confirm deletion" }).click();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true }),
  ).toBeFocused();
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toBeVisible();
  const requests = await page.evaluate(() =>
    (
      window as unknown as {
        workflowCalls: {
          command: string;
          args: { request: { etag: string; schedule_id: string } };
        }[];
      }
    ).workflowCalls.filter(
      (call) => call.command === "delete_workflow_schedule",
    ),
  );
  expect(requests).toHaveLength(1);
  expect(requests[0]!.args.request.etag).toMatch(/^[a-f0-9]{64}$/);
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Start with an example" }),
  ).toBeVisible();
});
test("stale deletion stays visible for reconciliation and never retries", async ({
  page,
}) => {
  await open(page);
  await page
    .getByRole("button", { name: "Delete schedule", exact: true })
    .click();
  await page.evaluate(() => {
    (window as unknown as { workflowConflict: boolean }).workflowConflict =
      true;
  });
  await page.getByRole("button", { name: "Confirm deletion" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "never retried automatically",
  );
  await expect(
    page.getByRole("button", { name: "Back to schedules" }),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { workflowCalls: { command: string }[] }
      ).workflowCalls.filter(
        (call) => call.command === "delete_workflow_schedule",
      ),
    ),
  ).toHaveLength(1);
});
test("an unconfirmed deletion reconciles the catalog without repeating the mutation", async ({
  page,
}) => {
  await open(page);
  await page
    .getByRole("button", { name: "Delete schedule", exact: true })
    .click();
  await page.evaluate(() => {
    (
      window as unknown as { workflowDeleteUncertain: boolean }
    ).workflowDeleteUncertain = true;
  });
  await page.getByRole("button", { name: "Confirm deletion" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Deletion could not be confirmed",
  );
  await page
    .getByRole("button", { name: "Refresh schedules", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true, level: 2 }),
  ).toBeFocused();
  await expect(
    page.getByRole("table", { name: "Schedules", exact: true }),
  ).not.toContainText("hourly-health");
  expect(
    await page.evaluate(() =>
      (
        window as unknown as { workflowCalls: { command: string }[] }
      ).workflowCalls.filter(
        (call) => call.command === "delete_workflow_schedule",
      ),
    ),
  ).toHaveLength(1);
});

test("legacy and older runtimes do not offer deletion, compact detail has no overflow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await installWorkflowFixture(page, { modern: false });
  await page.goto("/?fixture=operations-studio");
  await page
    .getByRole("button", { name: "Open work navigation", exact: true })
    .click();
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page
    .getByRole("button", { name: /legacy-report workspace-health/ })
    .click();
  await expect(
    page.getByRole("button", { name: /Delete (task|schedule)/ }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("complementary", { name: "Selected run output" }),
  ).toContainText("unavailable");
  await page.evaluate(() => {
    document.documentElement.dataset.textSize = "large";
  });
  expect(
    await page
      .locator(".workflow-surface")
      .evaluate((el) => el.scrollWidth <= el.clientWidth + 1),
  ).toBe(true);
  await capture(page, "04-compact-task-detail.png");
  expect(
    (await new AxeBuilder({ page }).include(".workflow-surface").analyze())
      .violations,
  ).toEqual([]);
});
test("workflow runs place exact released output in a right-side panel", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await installWorkflowFixture(page, { modern: true });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Workflows", exact: true }).click();
  await page.getByRole("button", { name: /workspace-health Version/ }).click();
  await page.getByRole("button", { name: "Run workflow", exact: true }).click();
  await page
    .getByRole("textbox", { name: "message *", exact: true })
    .fill("Inspect the Workspace");
  await page.getByRole("button", { name: "Review run", exact: true }).click();
  await page
    .getByRole("button", { name: "Start workflow", exact: true })
    .click();
  const output = page.getByRole("complementary", {
    name: "Selected run output",
  });
  await expect(output).toContainText("18446744073709551615");
  const history = await page
    .getByRole("region", { name: "Workflow run history" })
    .boundingBox();
  const box = await output.boundingBox();
  expect(box!.x).toBeGreaterThan(history!.x + history!.width);
  await capture(page, "05-workflow-output-right.png");
  expect(
    (await new AxeBuilder({ page }).include(".workflow-surface").analyze())
      .violations,
  ).toEqual([]);
  await page.getByRole("button", { name: "Close output" }).click();
  await expect(output).toHaveCount(0);
});

test("Run now executes a paused task with pinned inputs and keeps its schedule unchanged", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await installWorkflowFixture(page, { empty: true, modern: true });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page.getByText("More options", { exact: true }).click();
  await page
    .getByRole("button", { name: "Schedule a task", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Task name", exact: true })
    .fill("Manual health test");
  await page
    .getByRole("textbox", { name: "Instructions", exact: true })
    .fill("Inspect workspace health and return a report.");
  await page.getByText("Advanced", { exact: true }).click();
  await page
    .getByRole("checkbox", { name: "Enable immediately", exact: true })
    .uncheck();
  await page.getByRole("button", { name: "Review task", exact: true }).click();
  await page
    .getByRole("button", { name: "Create task schedule", exact: true })
    .click();
  const timing = await page.locator(".automation-overview").innerText();
  await page.evaluate(() => {
    (
      window as unknown as { workflowRunUncertain: boolean }
    ).workflowRunUncertain = true;
  });
  await page.getByRole("button", { name: "Run now", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: "run is unconfirmed" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Confirm same run request", exact: true })
    .click();
  await expect(page.getByRole("region", { name: "Task result" })).toContainText(
    "Workspace is healthy",
  );
  expect(await page.locator(".automation-overview").innerText()).toBe(timing);
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          workflowCalls: {
            command: string;
            args: { request: Record<string, unknown> };
          }[];
        }
      ).workflowCalls,
  );
  const starts = calls.filter((call) => call.command === "start_workflow_run");
  expect(starts).toHaveLength(2);
  expect(starts[0]!.args).toEqual(starts[1]!.args);
  expect(starts[0]!.args.request.inputs).toEqual({});
  expect(starts[0]!.args.request.expected_hash).toBe("a".repeat(64));
  expect(starts[0]!.args.request.workflow_id).toMatch(/^desktop-task-/u);
  expect(
    calls.filter((call) => call.command === "set_workflow_schedule_enabled"),
  ).toHaveLength(0);
  await capture(page, "05-task-run-now.png");
});
