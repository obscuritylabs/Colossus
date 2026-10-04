import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { installWorkflowFixture } from "./support/workflow-fixture";

const captures = resolve("../../.local/issue149-ui");
async function capture(page: Page, name: string) {
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS !== "1") return;
  mkdirSync(captures, { recursive: true });
  await page.screenshot({
    path: resolve(captures, name),
    fullPage: true,
    animations: "disabled",
  });
}
async function open(
  page: Page,
  options: { empty?: boolean; supported?: boolean } = {},
) {
  await installWorkflowFixture(page, options);
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
  ).toBeVisible();
  if (
    !(await page
      .getByRole("button", { name: "Schedules", exact: true })
      .isVisible())
  ) {
    await page
      .getByRole("button", { name: "Open work navigation", exact: true })
      .click();
  }
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Schedules", exact: true }),
  ).toBeVisible();
}
test("inspects canonical detail, independent waiting runs, pause, and legacy metadata", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await open(page);
  await expect(
    page.getByRole("button", { name: /hourly-health workspace-health/u }),
  ).toBeVisible();
  await capture(page, "01-schedules-list-light.png");
  await page
    .getByRole("button", { name: /hourly-health workspace-health/u })
    .click();
  await page.getByRole("button", { name: "Inspect last workflow run" }).click();
  await expect(
    page.getByRole("region", { name: "Independent workflow run" }),
  ).toContainText("waiting");
  await page
    .getByRole("region", { name: "Independent workflow run" })
    .scrollIntoViewIfNeeded();
  await capture(page, "02-schedule-detail-run-light.png");
  await page.getByRole("button", { name: "Review pause" }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "Already queued or running workflows continue",
  );
  await capture(page, "03-pause-review.png");
  await page.getByRole("button", { name: "Pause future ticks" }).click();
  await expect(
    page.getByRole("button", { name: "Review enable" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: /legacy-report workspace-health/u })
    .click();
  await expect(
    page.getByRole("region", { name: "Selected schedule" }),
  ).toContainText("ownership cannot be claimed");
  await expect(page.getByRole("button", { name: "Review enable" })).toHaveCount(
    0,
  );
});
test("imports an existing definition, freezes creation review, and reconciles an uncertain create", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 1000 });
  await open(page, { empty: true });
  await capture(page, "04-empty-workspace.png");
  await page.getByRole("button", { name: "Workflows", exact: true }).click();
  await page
    .getByRole("button", { name: "Import workflow", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Existing definition YAML" })
    .fill(
      "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata: {name: workspace-health, version: 1.0.0, description: Workspace health}\ninputs: {type: object, required: [message], properties: {message: {type: string}}, additionalProperties: false}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n",
    );
  await page.getByRole("button", { name: "Validate and review" }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "workspace-health:1.0.0",
  );
  await capture(page, "05-workflow-registration-review.png");
  await page
    .getByRole("button", { name: "Register workflow", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page
    .getByRole("button", { name: "Schedule a workflow", exact: true })
    .click();
  await choose(page, "Registered workflow", "workspace-health · 1.0.0");
  await page.getByRole("textbox", { name: "Schedule ID" }).fill("daily-health");
  await page.getByRole("button", { name: "Edit JSON", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Inputs (JSON object)", exact: true })
    .fill('{"message":"Check Workspace health"}');
  await choose(page, "Cadence", "Every 24 hours");
  await choose(page, "Time zone", "UTC — exact occurrence");
  await page
    .getByLabel("First occurrence", { exact: true })
    .fill("2026-10-05T09:30");
  await capture(page, "06-create-schedule-form.png");
  await page
    .getByRole("button", { name: "Review schedule", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "2026-10-05T09:30:00.000Z",
  );
  await capture(page, "07-create-schedule-review.png");
  const accessibility = await new AxeBuilder({ page })
    .include(".workflow-dialog")
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await page.evaluate(() => {
    (window as unknown as { workflowUncertain: boolean }).workflowUncertain =
      true;
  });
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Create schedule", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "Creation is unconfirmed",
  );
  await capture(page, "08-uncertain-allocation.png");
  await page.getByRole("button", { name: "Check stored schedule" }).click();
  await expect(
    page.getByRole("heading", { name: "daily-health" }),
  ).toBeVisible();
  const calls = await page.evaluate(() =>
    (
      window as unknown as {
        workflowCalls: { command: string; args: Record<string, unknown> }[];
      }
    ).workflowCalls.filter(
      (call) => call.command === "create_workflow_schedule",
    ),
  );
  expect(calls).toHaveLength(1);
  expect(calls[0]!.args.selectionEpoch).toBe(41);
  expect(calls[0]!.args.request).toMatchObject({
    starts_at: "2026-10-05T09:30:00.000Z",
    expected_hash: "a".repeat(64),
    enabled: false,
  });
});
test("rejects stale control reviews without retrying the mutation", async ({
  page,
}) => {
  await open(page);
  await page
    .getByRole("button", { name: /hourly-health workspace-health/u })
    .click();
  await page.getByRole("button", { name: "Review pause" }).click();
  await page.evaluate(() => {
    (window as unknown as { workflowConflict: boolean }).workflowConflict =
      true;
  });
  await page.getByRole("button", { name: "Pause future ticks" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Refresh the schedule and review again",
  );
  const calls = await page.evaluate(() =>
    (
      window as unknown as { workflowCalls: { command: string }[] }
    ).workflowCalls.filter(
      (call) => call.command === "set_workflow_schedule_enabled",
    ),
  );
  expect(calls).toHaveLength(1);
});
test("shows unavailable capability guidance", async ({ page }) => {
  await open(page, { supported: false });
  await expect(
    page.getByRole("button", { name: "Schedule a workflow" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("region", { name: "Workspace schedules" }),
  ).toContainText("administrator must enable workflow resources");
  await capture(page, "09-capability-unavailable.png");
});
test("reflows and remains accessible in dark theme at a narrow viewport", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 880, height: 900 });
  await open(page);
  await page
    .getByRole("button", { name: /hourly-health workspace-health/u })
    .click();
  const results = await new AxeBuilder({ page })
    .include(".workflow-surface")
    .analyze();
  expect(results.violations).toEqual([]);
  await capture(page, "10-schedule-detail-dark-narrow.png");
  const width = await page.locator(".workflow-surface").evaluate((element) => ({
    scroll: element.scrollWidth,
    client: element.clientWidth,
  }));
  expect(width.scroll).toBeLessThanOrEqual(width.client + 1);
  await page.getByRole("button", { name: "Review pause" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "Review pause" }),
  ).toBeFocused();
});

async function choose(page: Page, name: string, option: string) {
  await page.getByRole("combobox", { name, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test("validates DST gaps and repeated wall times at a compact width", async ({
  browser,
}) => {
  const context = await browser.newContext({
    timezoneId: "America/New_York",
    viewport: { width: 390, height: 844 },
  });
  const page = await context.newPage();
  try {
    await open(page);
    await page
      .getByRole("button", { name: "Schedule a workflow", exact: true })
      .click();
    await choose(page, "Registered workflow", "workspace-health · 1.0.0");
    await page.getByRole("textbox", { name: "Schedule ID" }).fill("dst-health");
    await page.getByRole("button", { name: "Edit JSON", exact: true }).click();
    await page
      .getByRole("textbox", { name: "Inputs (JSON object)" })
      .fill('{"message":"Check health"}');
    await page
      .getByLabel("First occurrence", { exact: true })
      .fill("2026-03-08T02:30");
    await page
      .getByRole("button", { name: "Review schedule", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText("does not exist");
    await page
      .getByLabel("First occurrence", { exact: true })
      .fill("2026-11-01T01:30");
    await page
      .getByRole("button", { name: "Review schedule", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText("occurs twice");
    await page.getByRole("alert").scrollIntoViewIfNeeded();
    await capture(page, "11-dst-validation-compact.png");
    await choose(page, "Time zone", "UTC — exact occurrence");
    await page
      .getByRole("button", { name: "Review schedule", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText(
      "2026-11-01T01:30:00.000Z",
    );
    const width = await page.getByRole("dialog").evaluate((element) => ({
      scroll: element.scrollWidth,
      client: element.clientWidth,
    }));
    expect(width.scroll).toBeLessThanOrEqual(width.client);
    await capture(page, "12-create-review-compact.png");
    const results = await new AxeBuilder({ page })
      .include(".workflow-dialog")
      .analyze();
    expect(results.violations).toEqual([]);
  } finally {
    await context.close();
  }
});

test("shows input schema failures inline without allocating a schedule", async ({
  page,
}) => {
  await open(page);
  await page
    .getByRole("button", { name: "Schedule a workflow", exact: true })
    .click();
  await choose(page, "Registered workflow", "workspace-health · 1.0.0");
  await page
    .getByRole("textbox", { name: "Schedule ID" })
    .fill("invalid-health");
  await page.getByRole("button", { name: "Edit JSON", exact: true }).click();
  await choose(page, "Time zone", "UTC — exact occurrence");
  await page
    .getByLabel("First occurrence", { exact: true })
    .fill("2026-10-05T09:30");
  await page
    .getByRole("button", { name: "Review schedule", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Create schedule", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "registered workflow schema",
  );
  await expect(page.getByRole("alert")).toBeFocused();
  await capture(page, "13-input-schema-error.png");
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /invalid-health workspace-health/u }),
  ).toHaveCount(0);
});

test("opens authorized schedule and independent run inspection from an agent activity card", async ({
  page,
}) => {
  await installWorkflowFixture(page);
  await page.goto("/?fixture=operations-studio&scheduleActivity=1");
  await page.locator(".run-activity-summary").click();
  const card = page.getByRole("article", { name: "Schedule activity" });
  await expect(card).toContainText("Schedule: hourly-health");
  await expect(card).toContainText("Enabled · Every hour");
  await card.scrollIntoViewIfNeeded();
  await capture(page, "14-agent-schedule-activity.png");
  await card.getByRole("button", { name: "Inspect last workflow run" }).click();
  await expect(
    page.getByRole("region", { name: "Independent workflow run" }),
  ).toContainText("waiting");
});
