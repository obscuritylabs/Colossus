import { expect, test, type Page } from "@playwright/test";
import { AxeBuilder } from "@axe-core/playwright";
import { initialView, type ToolView, type WorkView } from "../../src/model.js";

const tool: ToolView = {
  id: "call-1",
  runId: "run-1",
  name: "shell.run",
  state: "started",
  summary: "Running workspace checks",
  input: '{"command":"npm test"}',
  history: [
    {
      state: "requested",
      summary: "Preparing checks",
      at: "2026-10-04T10:00:00Z",
    },
    {
      state: "started",
      summary: "Running workspace checks",
      at: "2026-10-04T10:00:01Z",
    },
  ],
};
const base: WorkView = {
  ...initialView("Colossus"),
  connected: true,
  sessionId: "session-1",
  busy: true,
  watching: true,
  status: "Working",
  messages: [
    {
      id: "user:run-1",
      runId: "run-1",
      role: "user",
      text: "Run the workspace checks",
    },
  ],
  tools: [tool],
};
async function update(
  page: Page,
  view: WorkView,
  palette = "editor",
  showToolActivity = true,
) {
  await page.evaluate(
    ({ view, palette, showToolActivity }) =>
      window.postMessage(
        { type: "state", view, preferences: { palette, showToolActivity } },
        "*",
      ),
    { view, palette, showToolActivity },
  );
}
async function open(page: Page, view: WorkView, palette = "editor") {
  await page.addInitScript(() => {
    Object.assign(window, {
      actions: [],
      acquireVsCodeApi: () => ({
        postMessage: (m: unknown) =>
          (window as unknown as { actions: unknown[] }).actions.push(m),
        getState: () => undefined,
        setState: () => undefined,
      }),
    });
  });
  await page.goto("/");
  await update(page, view, palette);
}

test("tool progress updates in place, preserves expansion/focus, and stays between each prompt and response", async ({
  page,
}) => {
  await open(page, base);
  const heading = page.locator(".tool-progress-heading");
  await heading.click();
  await expect(page.getByText("Released input", { exact: true })).toBeVisible();
  await heading.focus();
  await page.evaluate(() => {
    Object.assign(window, {
      originalRow: document.querySelector(".tool-progress"),
    });
  });
  const completed = {
    ...tool,
    state: "completed",
    summary: "All checks passed",
    preview: "33 tests passed",
    history: [
      ...tool.history!,
      {
        state: "completed",
        summary: "All checks passed",
        at: "2026-10-04T10:00:10Z",
      },
    ],
  };
  const next: WorkView = {
    ...base,
    busy: false,
    tools: [completed],
    messages: [
      ...base.messages,
      {
        id: "assistant:run-1",
        runId: "run-1",
        role: "assistant",
        text: "The workspace checks passed.",
      },
    ],
  };
  await update(page, next);
  await expect(heading).toBeFocused();
  await expect(page.locator(".tool-progress")).toHaveAttribute("open", "");
  expect(
    await page.evaluate(
      () =>
        document.querySelector(".tool-progress") ===
        (window as unknown as { originalRow: Element }).originalRow,
    ),
  ).toBe(true);
  await expect(
    page.getByText("33 tests passed", { exact: true }),
  ).toBeVisible();
  await expect(page.locator("#messages > :nth-child(1)")).toHaveClass(/user/);
  await expect(page.locator("#messages > :nth-child(2)")).toHaveClass(
    "tool-thread",
  );
  await expect(page.locator("#messages > :nth-child(3)")).toHaveClass(
    /assistant/,
  );
  await update(page, {
    ...next,
    messages: [
      ...next.messages,
      {
        id: "user:run-2",
        runId: "run-2",
        role: "user",
        text: "Inspect the plan",
      },
      {
        id: "assistant:run-2",
        runId: "run-2",
        role: "assistant",
        text: "The plan is saved.",
      },
    ],
    tools: [
      completed,
      { ...tool, runId: "run-2", name: "plan.show", state: "completed" },
    ],
  });
  await expect(page.locator(".tool-thread")).toHaveCount(2);
  await expect(page.locator("#messages > :nth-child(5)")).toHaveAttribute(
    "data-run-id",
    "run-2",
  );
  await expect(heading.first()).toBeFocused();
  await update(page, next, "editor", false);
  await expect(page.locator(".tool-thread")).toBeHidden();
});

for (const [width, palette] of [
  [260, "editor"],
  [430, "colossus"],
  [700, "hacker"],
] as const) {
  test(`tool states and plain released details are accessible at ${width}px in ${palette}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 950 });
    const unsafe =
      '<img src="https://example.invalid/steal" onerror="alert(1)">';
    const tools = [
      "completed",
      "waiting approval",
      "started",
      "failed",
      "outcome unknown",
      "cancelled",
    ].map((state, i) => ({
      ...tool,
      id: `call-${i}`,
      name: `${"long-name".repeat(5)}.run`,
      state,
      input: unsafe,
    }));
    await open(page, { ...base, tools }, palette);
    await page.locator(".tool-progress-heading").nth(4).click();
    await expect(
      page.getByText(
        "The tool outcome is unknown; no successful output is confirmed.",
      ),
    ).toBeVisible();
    await expect(
      page.locator(".tool-progress[open]").getByText(unsafe, { exact: true }),
    ).toBeVisible();
    await expect(
      page.locator(
        ".tool-thread img:not([data-brand-mark]), .tool-thread a, .tool-thread script",
      ),
    ).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    expect(
      await page
        .locator(".tool-progress-heading")
        .first()
        .evaluate((el) => getComputedStyle(el).paddingTop),
    ).toBe("4px");
  });
}

test("new tool calls follow the conversation bottom without moving someone reading earlier messages", async ({
  page,
}) => {
  await page.setViewportSize({ width: 430, height: 750 });
  const messages = [
    {
      id: "user:prior",
      runId: "prior",
      role: "user" as const,
      text: "Earlier task",
    },
    {
      id: "assistant:prior",
      runId: "prior",
      role: "assistant" as const,
      text: "Earlier output\n\n".repeat(70),
    },
    ...base.messages,
  ];
  const view = { ...base, messages };
  await open(page, view);
  const scroll = page.locator("#conversation");
  await expect
    .poll(() =>
      scroll.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight),
    )
    .toBeLessThan(2);
  const withTools = {
    ...view,
    tools: Array.from({ length: 15 }, (_, i) => ({ ...tool, id: `call-${i}` })),
  };
  await update(page, withTools);
  await expect(page.locator(".tool-progress")).toHaveCount(15);
  await expect
    .poll(() =>
      scroll.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight),
    )
    .toBeLessThan(2);
  await scroll.evaluate((el) => {
    el.scrollTop = 100;
  });
  await update(page, {
    ...withTools,
    tools: [...withTools.tools, { ...tool, id: "new-call" }],
  });
  await expect(page.locator(".tool-progress")).toHaveCount(16);
  expect(await scroll.evaluate((el) => el.scrollTop)).toBe(100);
});
