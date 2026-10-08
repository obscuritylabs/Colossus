import AxeBuilder from "@axe-core/playwright";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { installWorkflowFixture } from "./support/workflow-fixture";

test("workflow library opens logic before schedules and freezes a manual run through uncertain allocation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await installWorkflowFixture(page, { modern: true });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Workflows", exact: true }).click();
  await page.getByRole("button", { name: /workspace-health Version/u }).click();
  await page
    .getByRole("button", { name: "View workflow logic", exact: true })
    .click();
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "result, Output", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Close graph", exact: true }).click();
  await page.getByRole("button", { name: "Run workflow", exact: true }).click();
  await page
    .getByRole("textbox", { name: "message *", exact: true })
    .fill("Inspect the Workspace");
  await page.getByRole("button", { name: "Edit JSON", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Inputs (JSON object)", exact: true }),
  ).toHaveValue(/Inspect the Workspace/u);
  await page.getByRole("button", { name: "Review run", exact: true }).click();
  await page.evaluate(() => {
    (
      window as unknown as { workflowRunUncertain: boolean }
    ).workflowRunUncertain = true;
  });
  await page
    .getByRole("button", { name: "Start workflow", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText(
    "allocation is unconfirmed",
  );
  await page
    .getByRole("button", { name: "Confirm same run request", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("region", { name: "Independent workflow run" }),
  ).toContainText('"ok": true');
  await expect(
    page.getByRole("region", { name: "Independent workflow run" }),
  ).toContainText("18446744073709551615");
  const requests = await page.evaluate(() =>
    (
      window as unknown as {
        workflowCalls: { command: string; args: unknown }[];
      }
    ).workflowCalls.filter((call) => call.command === "start_workflow_run"),
  );
  expect(requests).toHaveLength(2);
  expect(requests[0]?.args).toEqual(requests[1]?.args);
});

test("compact workflow library is keyboard reachable and task timing retains local timezone semantics", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await installWorkflowFixture(page, { modern: true });
  await page.goto("/?fixture=operations-studio");
  await page
    .getByRole("button", { name: "Open work navigation", exact: true })
    .click();
  await page.getByRole("button", { name: "Workflows", exact: true }).click();
  await page.getByRole("button", { name: /workspace-health Version/u }).click();
  await expect(
    page.getByRole("button", { name: "Run workflow", exact: true }),
  ).toBeVisible();
  const audit = await new AxeBuilder({ page })
    .include(".workflow-surface")
    .analyze();
  expect(audit.violations).toEqual([]);
  await page
    .getByRole("button", { name: "Open Workspace navigation", exact: true })
    .click();
  await page.getByRole("button", { name: "Schedules", exact: true }).click();
  await page.getByText("More options", { exact: true }).click();
  await page
    .getByRole("button", { name: "Schedule a task", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Task name", exact: true })
    .fill("Monday briefing");
  await page
    .getByRole("textbox", { name: "Instructions", exact: true })
    .fill("Brief me on consequential cybersecurity developments.");
  await page.getByRole("combobox", { name: "Repeat", exact: true }).click();
  await page.getByRole("option", { name: "Weekly", exact: true }).click();
  await page.getByRole("combobox", { name: "Time zone", exact: true }).click();
  await page
    .getByRole("option", { name: "America/New_York", exact: true })
    .click();
  await page.getByLabel("First date", { exact: true }).fill("2026-10-04");
  await page.getByRole("button", { name: "Review task", exact: true }).click();
  await expect(page.getByRole("dialog")).toContainText(
    "Mon at 09:00 · America/New_York",
  );
  await expect(page.getByRole("dialog")).toContainText(
    "2026-10-05T13:00:00.000Z",
  );
  const dialogAudit = await new AxeBuilder({ page })
    .include("dialog")
    .analyze();
  expect(dialogAudit.violations).toEqual([]);
  expect(
    await page
      .getByRole("dialog")
      .evaluate((element) => element.scrollWidth <= element.clientWidth + 1),
  ).toBe(true);
});

test("empty workflow library has a single starting view and launches a new authoring chat", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await installWorkflowFixture(page, { empty: true, modern: true });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Workflows", exact: true }).click();
  await expect(
    page.getByRole("heading", {
      name: "Turn a repeatable task into a workflow",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Explore a workflow" }),
  ).toHaveCount(0);
  expect(
    (await new AxeBuilder({ page }).include(".workflow-surface").analyze())
      .violations,
  ).toEqual([]);
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS === "1") {
    const folder = resolve("../../.local/schedules-polish/after");
    mkdirSync(folder, { recursive: true });
    await page.screenshot({
      path: resolve(folder, "workflows-empty.png"),
      animations: "disabled",
    });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => {
    document.documentElement.dataset.textSize = "large";
  });
  expect(
    await page
      .locator(".workflow-surface")
      .evaluate((element) => element.scrollWidth <= element.clientWidth + 1),
  ).toBe(true);
  await page
    .getByRole("button", { name: "Create with agent", exact: true })
    .click();
  const message = page.locator(
    'article[data-role="user"] .shared-message-body',
  );
  await expect(message).toHaveCount(1);
  await expect(message).toContainText("Help me create a reusable workflow");
  await expect(message).toContainText("strict input schema");
});
