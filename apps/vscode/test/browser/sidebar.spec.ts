import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView, type WorkView } from "../../src/model.js";

async function open(page: Page, view: WorkView) {
  await page.addInitScript(() => {
    const host = window as unknown as {
      acquireVsCodeApi: () => unknown;
      actions: unknown[];
    };
    host.actions = [];
    host.acquireVsCodeApi = () => ({
      postMessage: (message: unknown) => host.actions.push(message),
      getState: () => undefined,
      setState: () => undefined,
    });
  });
  await page.goto("/");
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    view,
  );
}

test("sidebar works at narrow widths, retains a cancelled draft, and renders model content as text", async ({
  page,
}) => {
  const view = {
    ...initialView("project"),
    connected: true,
    status: "Ready",
    version: "0.11.6",
  };
  await open(page, view);
  await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .fill("Inspect this function");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({
      type: "send",
      text: "Inspect this function",
      mode: "plan",
    });
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    view,
  );
  await expect(
    page.getByRole("textbox", { name: "Task for Colossus" }),
  ).toHaveValue("Inspect this function");
  const model = '<img src="https://example.invalid/steal" onerror="alert(1)">';
  await page.evaluate(
    (view) => window.postMessage({ type: "state", view }, "*"),
    {
      ...view,
      messages: [
        { id: "u1", role: "user", text: "Inspect this function" },
        { id: "a1", role: "assistant", text: model },
      ],
    },
  );
  await expect(page.getByText(model, { exact: true })).toBeVisible();
  await expect(page.locator("#messages img")).toHaveCount(0);
  await page.getByRole("button", { name: "Open Colossus settings" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({ type: "openSettings" });
  await page.setViewportSize({ width: 240, height: 850 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "artifacts/sidebar-narrow.png",
    fullPage: true,
  });
});

test("long conversations scroll above a fixed composer and honor saved editor preferences", async ({
  page,
}) => {
  const view: WorkView = {
    ...initialView("Colossus"),
    connected: true,
    status: "Ready",
    sessionId: "s1",
    messages: [
      { id: "u1", role: "user", text: "Explain the architecture" },
      {
        id: "a1",
        role: "assistant",
        text:
          Array.from(
            { length: 30 },
            (_, i) =>
              `### Component ${i}\n\n**Runtime** uses \`CoreRuntime\` to run tasks.\n\n- Durable state\n- Approved tools\n\n`,
          ).join("") +
          "\n```sh\ncolossus worker\n```\n\n[Unsafe](javascript:alert(1))\n\n![Remote](https://example.invalid/tracker)\n",
      },
    ],
    tools: [
      {
        id: "tool1",
        name: "git.status",
        state: "completed",
        summary: "Workspace status",
      },
    ],
  };
  await open(page, view);
  const preferences = {
    sendShortcut: "enter",
    defaultMode: "execute",
    showToolActivity: false,
  };
  await page.evaluate(
    ({ view, preferences }) =>
      window.postMessage({ type: "state", view, preferences }, "*"),
    { view, preferences },
  );
  await expect(page.getByRole("radio", { name: "Execute" })).toBeChecked();
  await expect(page.locator("#activity")).toBeHidden();
  await expect(
    page.locator("#messages a, #messages img, #messages script"),
  ).toHaveCount(0);
  await expect(page.locator("#messages pre code")).toHaveText(
    "colossus worker",
  );
  const before = await page
    .getByRole("textbox", { name: "Task for Colossus" })
    .boundingBox();
  await page.locator("#conversation").evaluate((el) => {
    el.scrollTop = 0;
  });
  expect(
    await page
      .getByRole("textbox", { name: "Task for Colossus" })
      .boundingBox(),
  ).toEqual(before);
  expect(before!.y + before!.height).toBeLessThan(850);
  const prompt = page.getByRole("textbox", { name: "Task for Colossus" });
  await prompt.fill("Continue");
  await prompt.press("Shift+Enter");
  await expect(prompt).toHaveValue("Continue\n");
  await prompt.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({ type: "send", text: "Continue\n", mode: "execute" });
  await page.evaluate(
    ({ view, preferences }) =>
      window.postMessage(
        {
          type: "state",
          view: { ...view, sessionId: "", messages: [], tools: [] },
          preferences,
        },
        "*",
      ),
    { view, preferences: { ...preferences, defaultMode: "plan" } },
  );
  await expect(page.getByRole("radio", { name: "Plan" })).toBeChecked();
});

test("pending approval requests native review without passing executable authority", async ({
  page,
}) => {
  const view = {
    ...initialView("project"),
    connected: true,
    busy: true,
    watching: true,
    status: "Waiting",
    sessionId: "session-1",
    sessions: [{ id: "session-1", title: "Workspace checks" }],
    version: "0.11.6",
    messages: [
      { id: "u1", role: "user" as const, text: "Run the checks" },
      {
        id: "a1",
        role: "assistant" as const,
        text: "I’ll check the affected modules.",
      },
    ],
    interactions: [
      {
        id: "interaction-1",
        kind: "approval" as const,
        title: "Check workspace code",
        respondable: true,
      },
    ],
    tools: [
      {
        id: "call-1",
        name: "shell.run",
        state: "waiting approval",
        summary: "Run the focused checks",
      },
    ],
  };
  await open(page, view);
  await page.getByRole("button", { name: "Review and respond" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { actions: unknown[] }).actions,
      ),
    )
    .toContainEqual({ type: "respond", id: "interaction-1" });
  await expect(
    page.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Task for Colossus" }),
  ).toBeEnabled();
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.screenshot({
    path: "artifacts/sidebar-approval.png",
    fullPage: true,
  });
});
