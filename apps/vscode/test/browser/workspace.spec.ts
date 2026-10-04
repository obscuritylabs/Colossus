import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView, type InspectionView } from "../../src/model.js";
const run = {
  id: "run-1",
  sessionId: "s1",
  title: "Build navigation",
  role: "primary",
  mode: "plan",
  status: "completed",
  createdAt: "2026-10-01T12:00:00Z",
  updatedAt: "2026-10-01T12:01:00Z",
  startedAt: "2026-10-01T12:00:00Z",
  finishedAt: "2026-10-01T12:01:00Z",
  sequence: "9007199254741009",
  pendingInteractions: 0,
};
const plan = {
  id: "p1",
  sourceRunId: "run-1",
  sessionId: "s1",
  title: "Build navigation",
  revision: "9007199254741009",
  status: "draft",
  goalId: "",
};
async function open(page: Page, path: string) {
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      actions: unknown[];
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (value: unknown) => host.actions.push(value),
      getState: () => undefined,
      setState() {},
    });
  });
  await page.goto(path);
}
const actions = (page: Page) =>
  page.evaluate(() => (window as unknown as { actions: unknown[] }).actions);

test("workspace browses sessions and canonical plans separately from chat, and reads older history explicitly", async ({
  page,
}) => {
  await open(page, "/explorer");
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    {
      ...initialView("Colossus"),
      connected: true,
      sessions: [{ id: "s1", title: "Build navigation", status: "completed" }],
      runs: [run],
      plans: [plan],
      historyHasMore: true,
    },
  );
  await page
    .getByRole("button", { name: "Build navigation completed", exact: true })
    .click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({ type: "selectSession", id: "s1" });
  await page
    .getByRole("button", { name: "Inspect session: Build navigation" })
    .click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({ type: "inspectRun", id: "run-1" });
  await page.getByRole("button", { name: "Plans", exact: true }).click();
  await page.getByRole("button", { name: /Revision 9007199254741009/ }).click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({ type: "inspectPlan", id: "p1" });
  await page.getByRole("button", { name: "Load older history" }).click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({ type: "loadMoreSessions" });
  await page
    .getByRole("searchbox", { name: "Search workspace data" })
    .fill("missing");
  await expect(page.getByText("No matching plans.")).toBeVisible();
  await page.setViewportSize({ width: 240, height: 850 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("inspector shows saved identities, exact revisions, released output and activity without navigable model markup", async ({
  page,
}) => {
  await open(page, "/inspector");
  const view: InspectionView = {
    run,
    plan,
    output:
      "## Plan\n\n[Link](https://invalid.example)\n\n<img src=x onerror=alert(1)>",
    model: "primary",
    provider: "offline",
    activities: [
      {
        id: "a1",
        title: "Plan saved",
        summary: "Saved revision",
        kind: "plan",
        lane: "agent",
        status: "completed",
        startedAt: run.createdAt,
        completedAt: run.finishedAt,
        result: "<script>steal()</script>",
      },
    ],
    activityState: "Projection is up to date.",
    activityHasMore: true,
    observedAt: run.finishedAt,
  };
  await page.evaluate(
    (view) =>
      window.postMessage(
        {
          type: "inspection",
          view,
          connected: true,
          loading: false,
          error: "",
        },
        "*",
      ),
    view,
  );
  await expect(page.getByText("9007199254741009", { exact: true })).toHaveCount(
    2,
  );
  await page.getByRole("button", { name: "Refresh state" }).click();
  await expect
    .poll(() => actions(page))
    .toContainEqual({ type: "refreshInspection" });
  await page.getByRole("button", { name: "Output", exact: true }).click();
  await expect(page.locator("main a, main img, main script")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Session activity", exact: true })
    .click();
  await page.getByText("Released result", { exact: true }).click();
  await expect(page.locator("pre")).toHaveText("<script>steal()</script>");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.evaluate(() => document.body.classList.add("vscode-light"));
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(
    await page.evaluate(() =>
      getComputedStyle(document.documentElement)
        .getPropertyValue("--main")
        .trim(),
    ),
  ).toBe("#f7f9fc");
});

test("composer grows with the draft and retains busy drafts without automatically sending", async ({
  page,
}) => {
  await open(page, "/");
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    { ...initialView("Colossus"), connected: true, busy: true, watching: true },
  );
  const prompt = page.getByRole("textbox", { name: "Task for Colossus" });
  const before = (await prompt.boundingBox())!.height;
  await prompt.fill(
    Array.from({ length: 12 }, (_, i) => `Next task line ${i}`).join("\n"),
  );
  expect((await prompt.boundingBox())!.height).toBeGreaterThan(before);
  await prompt.press("Control+Enter");
  expect(await actions(page)).not.toContainEqual(
    expect.objectContaining({ type: "send" }),
  );
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    { ...initialView("Colossus"), connected: true },
  );
  expect(await prompt.inputValue()).toContain("Next task line 11");
  expect(await actions(page)).not.toContainEqual(
    expect.objectContaining({ type: "send" }),
  );
});
