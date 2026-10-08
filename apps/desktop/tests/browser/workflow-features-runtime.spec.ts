import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { mkdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { processRuntimeHost } from "./support/process-runtime-host";

test.use({ timezoneId: "America/New_York" });
test.skip(
  process.env.COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE !== "1",
  "Uses the production authenticated SDK and an isolated real sidecar.",
);
async function capture(page: Page, name: string) {
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS !== "1") return;
  const directory = resolve("../../.local/workflows-schedules-ui");
  mkdirSync(directory, { recursive: true });
  await page.screenshot({
    path: resolve(directory, name),
    animations: "disabled",
  });
}
async function choose(page: Page, name: string, option: string) {
  await page.getByRole("combobox", { name, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}

test("Workflows and Schedules use real manual runs, calendar tasks, model preferences, and exact uncertain-create reconciliation", async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.setViewportSize({ width: 1600, height: 1100 });
  const host = await processRuntimeHost("workflow-acceptance");
  try {
    await page.exposeFunction(
      "workflowBridge",
      (command: string, args: unknown) => host.invoke(command, args),
    );
    await page.addInitScript(() => {
      const host = window as unknown as {
        workflowBridge: (command: string, args: unknown) => Promise<unknown>;
        __TAURI_INTERNALS__: unknown;
        loseTaskResponse: boolean;
        loseDeletionResponse: boolean;
      };
      host.loseTaskResponse = false;
      host.loseDeletionResponse = false;
      host.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: unknown) => {
          const result = await host.workflowBridge(command, args);
          if (
            command === "delete_workflow_schedule" &&
            host.loseDeletionResponse
          ) {
            host.loseDeletionResponse = false;
            throw {
              code: "outcome_unknown",
              message: "Deletion could not be confirmed.",
              retryable: false,
              outcomeUnknown: true,
              violations: [],
            };
          }
          if (command === "create_workflow_schedule" && host.loseTaskResponse) {
            host.loseTaskResponse = false;
            throw {
              code: "outcome_unknown",
              message: "Task allocation could not be confirmed.",
              retryable: false,
              outcomeUnknown: true,
              violations: [],
            };
          }
          return result;
        },
      };
    });
    await page.goto("/?fixture=operations-studio");
    await page.getByRole("button", { name: "Workflows", exact: true }).click();
    await page
      .getByRole("button", { name: "Import workflow", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Existing definition YAML" })
      .fill(
        readFileSync(
          resolve("../../examples/workflows/01-control-flow-lab.yaml"),
          "utf8",
        ),
      );
    await page
      .getByRole("button", { name: "Validate and review", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Register workflow", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page
      .getByRole("button", { name: /control-flow-lab Version/u })
      .click();
    await page
      .getByRole("button", { name: "View workflow logic", exact: true })
      .click();
    await expect(
      page.getByRole("dialog").locator(".react-flow__node-workflowStep"),
    ).toHaveCount(9);
    const schedules = (await host.invoke("list_workflow_schedules", {
      after: null,
    })) as { items: unknown[] };
    expect(schedules.items).toHaveLength(0);
    await capture(page, "01-workflow-logic-before-schedules.png");
    await page
      .getByRole("button", { name: "Close graph", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Run workflow", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Inputs (JSON object)", exact: true })
      .fill(
        JSON.stringify(
          {
            environment: "production",
            components: ["api", "worker", "desktop", "documentation"],
          },
          null,
          2,
        ),
      );
    await page.getByRole("button", { name: "Review run", exact: true }).click();
    await capture(page, "02-manual-run-review.png");
    await page
      .getByRole("button", { name: "Start workflow", exact: true })
      .click();
    await expect(
      page.getByRole("region", { name: "Independent workflow run" }),
    ).toContainText("completed", { timeout: 30_000 });
    await page
      .getByRole("button", { name: "Refresh history", exact: true })
      .click();
    await expect(
      page.getByRole("region", { name: "Workflow run history" }),
    ).toContainText("completed");
    await page.locator(".workflow-surface").evaluate((element) => {
      element.scrollTop = 0;
    });
    await page
      .getByRole("region", { name: "Workflow run history" })
      .getByRole("button")
      .filter({ hasText: "completed" })
      .click();
    await expect(
      page.getByRole("complementary", { name: "Selected run output" }),
    ).toContainText("Result");
    await capture(page, "03-workflows-library-and-history.png");
    await page
      .getByRole("button", { name: "View run in graph", exact: true })
      .click();
    await expect(
      page.getByRole("dialog").getByRole("button", {
        name: "component-observed, Output, completed",
        exact: true,
      }),
    ).toBeVisible();
    await page
      .getByRole("dialog")
      .getByRole("button", {
        name: "component-observed, Output, completed",
        exact: true,
      })
      .click();
    await capture(page, "04-complex-workflow-execution.png");
    await page
      .getByRole("button", { name: "Close graph", exact: true })
      .click();
    await page.getByRole("button", { name: "Schedules", exact: true }).click();
    await page.getByText("More options", { exact: true }).click();
    await page
      .getByRole("button", { name: "Schedule a task", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Task name", exact: true })
      .fill("Monday cybersecurity briefing");
    await page
      .getByRole("textbox", { name: "Instructions", exact: true })
      .fill(
        "Brief me on consequential U.S. federal cybersecurity market developments relevant to leading a cyber services firm. Prioritize procurement, policy, and agency priorities. Include source links and actionable implications.",
      );
    await choose(page, "Repeat", "Weekly");
    await choose(page, "Time zone", "America/New_York");
    await page.getByLabel("First date", { exact: true }).fill("2026-10-05");
    await capture(page, "05-schedule-a-task.png");
    await page.getByText("Advanced", { exact: true }).click();
    await choose(page, "Effort", "high");
    await page.getByRole("dialog").evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.setViewportSize({ width: 1600, height: 1600 });
    await capture(page, "06-task-advanced-options.png");
    await page.setViewportSize({ width: 1600, height: 1100 });
    await page
      .getByRole("button", { name: "Review task", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText(
      "Mon at 09:00 · America/New_York",
    );
    await expect(page.getByRole("dialog")).toContainText(
      "2026-10-05T13:00:00.000Z",
    );
    await capture(page, "07-task-schedule-review.png");
    await page.evaluate(() => {
      (window as unknown as { loseTaskResponse: boolean }).loseTaskResponse =
        true;
    });
    await page
      .getByRole("button", { name: "Create task schedule", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText(
      "Creation is unconfirmed",
      { timeout: 30_000 },
    );
    await page
      .getByRole("button", { name: "Check stored schedule", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(
      page.getByRole("region", { name: "Task instructions" }),
    ).toContainText("consequential U.S. federal cybersecurity");
    const page_ = (await host.invoke("list_workflow_schedules", {
      after: null,
    })) as { items: { record: { schedule_id: string } }[] };
    expect(page_.items).toHaveLength(1);
    const task = (await host.invoke("get_workflow_schedule", {
      scheduleId: page_.items[0]!.record.schedule_id,
    })) as {
      record: {
        workflow_name: string;
        workflow_version: string;
        workflow_hash: string;
        task: { options: { reasoning_effort: string } };
      };
    };
    expect(task.record.task.options.reasoning_effort).toBe("high");
    await page.getByRole("button", { name: "Run now", exact: true }).click();
    await expect(
      page.getByRole("region", { name: "Independent workflow run" }),
    ).toBeVisible();
    const manualHistory = (await host.invoke("list_workflow_runs", {
      workflowId: `${task.record.workflow_name}:${task.record.workflow_version}`,
      after: null,
    })) as { items: { run_id: string }[] };
    expect(manualHistory.items).toHaveLength(1);
    const queued = manualHistory.items[0]!;
    await expect
      .poll(
        async () => {
          const run = (await host.invoke("get_scheduled_workflow_run", {
            runId: queued.run_id,
          })) as { status: string };
          if (run.status === "failed")
            throw new Error(
              JSON.stringify({
                run,
                providerRequests: host.observations.length,
              }),
            );
          return run.status;
        },
        { timeout: 30_000 },
      )
      .toBe("completed");
    expect(
      host.observations.some(
        (observation) =>
          (observation as { reasoning_effort?: string }).reasoning_effort ===
          "high",
      ),
    ).toBe(true);
    const agentTask = {
      schedule_id: "agent-weekly-briefing",
      task: {
        name: "Agent weekly briefing",
        instructions: "Summarize procurement news.",
        tools: [],
        options: {},
      },
      calendar: { timezone: "America/New_York", time: "09:00", weekdays: [1] },
      starts_at: "2026-10-05T13:00:00Z",
      misfire_policy: "fire_once",
      enabled: true,
      idempotency_key: "agent-weekly-briefing-v1",
    };
    const agentRun = await host.runTool("workflow_task_schedule", agentTask);
    expect(agentRun.approvals).toBe(1);
    const agentSchedule = (await host.invoke("get_workflow_schedule", {
      scheduleId: agentTask.schedule_id,
    })) as {
      record: { task: { instructions: string } };
      origin: { owner: { id: string }; run_id: string; session_id: string };
    };
    expect(agentSchedule.record.task.instructions).toBe(
      agentTask.task.instructions,
    );
    expect(agentSchedule.origin.run_id).toBe(agentRun.run.run_id);
    expect(agentSchedule.origin.session_id).toBe(agentRun.run.session_id);
    expect(agentSchedule.origin.owner.id).toBe("app:approval-acceptance");
    const repeated = await host.runTool("workflow_task_schedule", agentTask);
    expect(repeated.approvals).toBe(1);
    const storedSchedules = (await host.invoke("list_workflow_schedules", {
      after: null,
    })) as { items: { record: { schedule_id: string } }[] };
    expect(
      storedSchedules.items.filter(
        (item) => item.record.schedule_id === agentTask.schedule_id,
      ),
    ).toHaveLength(1);
    const library = (await host.invoke("list_registered_workflows", {
      after: null,
    })) as { items: unknown[] };
    expect(library.items).toHaveLength(1);
    const storedAgent = (await host.invoke("get_workflow_schedule", {
      scheduleId: agentTask.schedule_id,
    })) as { etag: string };
    const deletion = await host.runTool("workflow_schedule_delete", {
      schedule_id: agentTask.schedule_id,
      etag: storedAgent.etag,
    });
    expect(deletion.approvals).toBe(1);
    const remaining = (await host.invoke("list_workflow_schedules", {
      after: null,
    })) as { items: { record: { schedule_id: string } }[] };
    expect(
      remaining.items.map((item) => item.record.schedule_id),
    ).not.toContain(agentTask.schedule_id);
    await page
      .getByRole("button", { name: "Refresh history", exact: true })
      .click();
    await page
      .getByRole("region", { name: "Task run history" })
      .getByRole("button")
      .filter({ hasText: queued.run_id })
      .click();
    await expect(
      page.getByRole("complementary", { name: "Selected run output" }),
    ).toContainText("completed");
    await expect(
      page.getByRole("region", { name: "Task result", exact: true }),
    ).toContainText("Acceptance turn completed.");
    await page.locator(".workflow-surface").evaluate((element) => {
      element.scrollTop = 0;
    });
    await capture(page, "08-schedules-task-detail.png");
    await page
      .getByRole("button", { name: "Delete task", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText(
      "run history is retained",
    );
    await capture(page, "11-delete-task-review.png");
    await page.evaluate(() => {
      (
        window as unknown as { loseDeletionResponse: boolean }
      ).loseDeletionResponse = true;
    });
    await page
      .getByRole("button", { name: "Confirm deletion", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "Deletion could not be confirmed",
      { timeout: 30_000 },
    );
    await page
      .getByRole("button", { name: "Refresh schedules", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Schedules", exact: true, level: 2 }),
    ).toBeFocused();
    const deletedCatalog = (await host.invoke("list_workflow_schedules", {
      after: null,
    })) as { items: unknown[] };
    expect(deletedCatalog.items).toHaveLength(0);
    const retainedRun = (await host.invoke("get_scheduled_workflow_run", {
      runId: queued.run_id,
    })) as { status: string };
    expect(retainedRun.status).toBe("completed");
    await page.getByText("More options", { exact: true }).click();
    await page
      .getByRole("button", { name: "Schedule a task", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Task name", exact: true })
      .fill("Daily market briefing");
    await page
      .getByRole("textbox", { name: "Instructions", exact: true })
      .fill(
        "Summarize consequential cybersecurity market developments with source links and actionable implications.",
      );
    await page.evaluate(() => {
      document.documentElement.dataset.theme = "dark";
    });
    await capture(page, "10-schedule-a-task-dark.png");
    const taskAudit = await new AxeBuilder({ page })
      .include(".workflow-dialog")
      .analyze();
    expect(taskAudit.violations).toEqual([]);
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.evaluate(() => {
      document.documentElement.dataset.theme = "light";
    });
    const audit = await new AxeBuilder({ page })
      .include(".workflow-surface")
      .analyze();
    expect(audit.violations).toEqual([]);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.locator(".workflow-surface").evaluate((element) => {
      element.scrollTop = 0;
    });
    await capture(page, "09-schedules-compact.png");
    expect(
      await page
        .locator(".workflow-surface")
        .evaluate((element) => element.scrollWidth <= element.clientWidth + 1),
    ).toBe(true);
  } finally {
    await host.close();
  }
});
