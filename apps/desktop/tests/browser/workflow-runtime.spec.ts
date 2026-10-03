import { expect, test, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { processRuntimeHost } from "./support/process-runtime-host";

test.skip(
  process.env.COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE !== "1",
  "Run npm run test:approval-runtime for the production managed SDK and real sidecar tier",
);

test("schedule UI → production managed SDK → authenticated sidecar: registration, uncertain allocation and control", async ({
  page,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 1000 });
  const host = await processRuntimeHost("workflow-acceptance");
  try {
    const calls: { command: string; args: unknown }[] = [];
    await page.exposeFunction(
      "workflowBridge",
      async (command: string, args: unknown) => {
        calls.push({ command, args });
        return host.invoke(command, args);
      },
    );
    await page.addInitScript(() => {
      const host = window as unknown as {
        workflowBridge: (command: string, args: unknown) => Promise<unknown>;
        __TAURI_INTERNALS__: unknown;
        loseScheduleResponse: boolean;
      };
      host.loseScheduleResponse = true;
      host.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: unknown) => {
          const value = await host.workflowBridge(command, args);
          if (
            command === "create_workflow_schedule" &&
            host.loseScheduleResponse
          ) {
            host.loseScheduleResponse = false;
            throw {
              code: "outcome_unknown",
              message: "Injected response loss after actual allocation.",
              retryable: false,
              outcomeUnknown: true,
              violations: [],
            };
          }
          return value;
        },
      };
    });
    await page.goto("/?fixture=operations-studio");
    await expect(
      page.getByRole("heading", { name: "Harden desktop agent bootstrap" }),
    ).toBeVisible();
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
      .getByRole("button", { name: "Import workflow", exact: true })
      .click();
    const yaml =
      "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata:\n  name: native-health\n  version: 1.0.0\n  description: Real managed workflow acceptance\ninputs: {type: object, additionalProperties: false}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n";
    await page
      .getByRole("textbox", { name: "Existing definition YAML" })
      .fill(yaml);
    await page.getByRole("button", { name: "Validate and review" }).click();
    await expect(page.getByRole("dialog")).toContainText("native-health:1.0.0");
    await page
      .getByRole("button", { name: "Register workflow", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Create schedule", exact: true })
      .click();
    await choose(page, "Registered workflow", "native-health · 1.0.0");
    await page
      .getByRole("textbox", { name: "Schedule ID" })
      .fill("native-health");
    await choose(page, "Time zone", "UTC — exact occurrence");
    await page
      .getByLabel("First occurrence", { exact: true })
      .fill("2050-10-05T09:30");
    await page
      .getByRole("button", { name: "Review schedule", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Create schedule", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText(
      "Creation is unconfirmed",
    );
    await page.getByRole("button", { name: "Check stored schedule" }).click();
    await expect(
      page.getByRole("heading", { name: "native-health", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("region", { name: "Selected schedule" }),
    ).toContainText("app:approval-acceptance");
    expect(
      calls.filter((call) => call.command === "create_workflow_schedule"),
    ).toHaveLength(1);
    expect(await host.invoke("active_work")).toEqual({ active: false });
    await page.getByRole("button", { name: "Review enable" }).click();
    await page
      .getByRole("button", { name: "Enable schedule", exact: true })
      .click();
    await expect(
      page.getByRole("button", { name: "Review pause" }),
    ).toBeVisible();
    expect(await host.invoke("active_work")).toEqual({ active: false });
    await page.getByRole("button", { name: "Review pause" }).click();
    await page.getByRole("button", { name: "Pause future ticks" }).click();
    await expect(
      page.getByRole("button", { name: "Review enable" }),
    ).toBeVisible();
    if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS === "1") {
      const path = resolve("../../.local/issue149-ui");
      mkdirSync(path, { recursive: true });
      await page.locator(".workflow-surface").evaluate((element) => {
        element.scrollTop = 0;
      });
      await page.screenshot({
        path: resolve(path, "15-real-sidecar-schedule.png"),
        animations: "disabled",
      });
    }
  } finally {
    await host.close();
  }
});

async function choose(page: Page, name: string, option: string) {
  await page.getByRole("combobox", { name, exact: true }).click();
  await page.getByRole("option", { name: option, exact: true }).click();
}
