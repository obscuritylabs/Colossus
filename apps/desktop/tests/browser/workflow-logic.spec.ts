import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { installWorkflowFixture } from "./support/workflow-fixture";

test.use({ timezoneId: "America/New_York" });

const leaf = (id: string, kind: "emit" | "approval" = "emit") => ({
  id,
  kind,
  summary:
    kind === "approval"
      ? "Operator approval required"
      : "Emit a workflow value",
  branches: [],
});
const logic = {
  steps: [
    {
      id: "baseline-checks",
      kind: "parallel",
      summary: "2 branches · at most 2 concurrent",
      branches: [
        { label: "Branch 1", steps: [leaf("schema-check")] },
        { label: "Branch 2", steps: [leaf("policy-check")] },
      ],
    },
    {
      id: "environment-route",
      kind: "condition",
      summary: '/inputs/environment == "production"',
      branches: [
        { label: "True", steps: [leaf("production-route")] },
        { label: "False", steps: [leaf("non-production-route")] },
      ],
    },
    {
      id: "inspect-components",
      kind: "foreach",
      summary: "/inputs/components · at most 8 items",
      branches: [{ label: "Each item", steps: [leaf("component-observed")] }],
    },
    leaf("release-approval", "approval"),
    leaf("result"),
  ],
  compensation: [],
};

async function open(
  page: Page,
  mode: "normal" | "missing" | "mismatch" = "normal",
) {
  await installWorkflowFixture(page);
  await page.addInitScript(
    ({ logic, mode }) => {
      const host = window as any;
      const invoke = host.__TAURI_INTERNALS__.invoke;
      host.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: unknown,
      ) => {
        const value = await invoke(command, args);
        if (command === "get_registered_workflow")
          return {
            ...value,
            logic: mode === "missing" ? null : logic,
            workflow_hash:
              mode === "mismatch" ? "e".repeat(64) : value.workflow_hash,
          };
        if (command === "get_scheduled_workflow_run")
          return {
            ...value,
            status: "waiting",
            step_states: logic.steps
              .flatMap((step) => [
                step,
                ...step.branches.flatMap((branch) => branch.steps),
              ])
              .filter(
                (step) => !["non-production-route", "result"].includes(step.id),
              )
              .map((step) => ({
                step_id: step.id,
                status:
                  step.id === "release-approval" ? "waiting" : "completed",
                completed_executions:
                  step.id === "component-observed"
                    ? 4
                    : step.id === "release-approval"
                      ? 0
                      : 1,
              })),
          };
        return value;
      };
    },
    { logic, mode },
  );
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
    .getByRole("button", { name: /hourly-health workspace-health/u })
    .click();
  await page
    .getByRole("button", { name: "View workflow logic", exact: true })
    .click();
}
async function capture(page: Page, name: string) {
  if (process.env.COLOSSUS_WORKFLOW_SCREENSHOTS !== "1") return;
  const path = resolve("../../.local/workflow-graph-ui");
  mkdirSync(path, { recursive: true });
  await page.screenshot({ path: resolve(path, name), animations: "disabled" });
}

test("draws real branch structure, observed states, loop counts, and keyboard navigation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1200 });
  await open(page);
  const dialog = page.getByRole("dialog");
  await expect(
    dialog.getByRole("button", {
      name: "release-approval, Approval, waiting",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    dialog.locator(".react-flow__edge-text").filter({ hasText: "True" }),
  ).toBeVisible();
  await expect(
    dialog.locator(".react-flow__edge-text").filter({ hasText: "Next item" }),
  ).toBeVisible();
  await capture(page, "01-complex-waiting.png");
  await dialog
    .getByRole("button", { name: "non-production-route, Output", exact: true })
    .click();
  await expect(
    dialog.getByRole("complementary", { name: "Workflow step details" }),
  ).toContainText("No recorded execution");
  await dialog.getByRole("combobox", { name: "Choose a step" }).click();
  await dialog
    .getByRole("option", { name: "component-observed · Output", exact: true })
    .click();
  await expect(
    dialog.getByRole("complementary", { name: "Workflow step details" }),
  ).toContainText("4 completed executions");
  await dialog
    .getByRole("button", { name: "Fit workflow", exact: true })
    .click();
  await capture(page, "02-loop-inspection.png");
  const report = await new AxeBuilder({ page })
    .include(".workflow-logic-dialog")
    .withTags(["wcag2a", "wcag2aa"])
    .analyze();
  expect(report.violations).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "View workflow logic", exact: true }),
  ).toBeFocused();
});

test("supports dark theme and a compact graph with an accessible step selector", async ({
  page,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.setViewportSize({ width: 390, height: 844 });
  await open(page);
  await expect(
    page.getByRole("button", {
      name: "release-approval, Approval, waiting",
      exact: true,
    }),
  ).toBeVisible();
  const start = page.locator('.react-flow__node[data-id="main:start"]');
  const toolbar = page.getByRole("group", { name: "Workflow graph view" });
  const startBounds = await start.boundingBox();
  const toolbarBounds = await toolbar.boundingBox();
  expect(startBounds!.y).toBeGreaterThan(
    toolbarBounds!.y + toolbarBounds!.height,
  );
  await capture(page, "03-compact-dark.png");
  expect(
    (
      await new AxeBuilder({ page })
        .include(".workflow-logic-dialog")
        .withTags(["wcag2a", "wcag2aa"])
        .analyze()
    ).violations,
  ).toEqual([]);
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("combobox", { name: "Choose a step" }).click();
  await dialog
    .getByRole("option", { name: "release-approval · Approval", exact: true })
    .click();
  await expect(
    dialog.getByRole("complementary", { name: "Workflow step details" }),
  ).toContainText("Recorded execution");
  await dialog.locator(".workflow-logic-execution").scrollIntoViewIfNeeded();
  await capture(page, "04-compact-step-details.png");
});

for (const mode of ["missing", "mismatch"] as const)
  test(`handles ${mode} logic without displaying an unpinned graph`, async ({
    page,
  }) => {
    await open(page, mode);
    await expect(page.getByRole("dialog")).toContainText(
      mode === "missing"
        ? "does not provide a displayable workflow definition"
        : "differs from the reviewed hash",
    );
    await expect(page.locator(".workflow-logic-canvas")).toHaveCount(0);
  });
