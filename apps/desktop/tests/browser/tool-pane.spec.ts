import { expect, test, type Page } from "@playwright/test";

test("tool switching preserves browser tabs and the conversation draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  const prompt = page.getByRole("textbox", { name: "Prompt", exact: true });
  await prompt.fill("Keep this draft while switching tools.");
  await page.getByRole("button", { name: "Open browser", exact: true }).click();
  const address = page.getByRole("textbox", { name: "Web address" });
  await address.fill("https://example.com/docs");
  await address.press("Enter");
  await page.getByRole("button", { name: "Switch pane tool" }).click();
  await page.getByRole("menuitemradio", { name: /^Terminal/ }).click();
  await expect(
    page.getByRole("region", { name: "Terminal pane" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Switch pane tool" }).click();
  await page.getByRole("menuitemradio", { name: /^Browser/ }).click();
  await expect(address).toHaveValue("https://example.com/docs");
  await expect(prompt).toHaveValue("Keep this draft while switching tools.");
});

test("compact terminal reserves native bounds and tool menus work with the keyboard", async ({
  page,
}) => {
  await page.setViewportSize({ width: 880, height: 640 });
  await page.goto("/?fixture=operations-studio");
  const tools = page.getByRole("button", { name: "Open tools", exact: true });
  await tools.focus();
  await tools.press("ArrowDown");
  await page.keyboard.press("Home");
  await expect(
    page.getByRole("menuitemradio", { name: /^Files/ }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(
    page.getByRole("menuitemradio", { name: /^Terminal/ }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  const terminal = page.getByRole("region", { name: "Terminal pane" });
  await expect(terminal).toBeVisible();
  const bounds = await terminal.boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(48);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(641);
  const switcher = page.getByRole("button", { name: "Switch pane tool" });
  await switcher.click();
  await page.keyboard.press("Escape");
  await expect(switcher).toBeFocused();
  await expect(terminal).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(terminal).toHaveCount(0);
  await expect(tools).toBeFocused();
});

test("terminal scope changes cannot replay the previous workspace's plan", async ({
  page,
}) => {
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("button", { name: "Open tools", exact: true }),
  ).toBeVisible();
  const mounts = await page.evaluate(async () => {
    // Reuse the loaded renderer's React instances, including Vite's versioned
    // module URLs, so this exercises the actual component's passive effects.
    const moduleUrl = (filename: string) => {
      const resource = performance
        .getEntriesByType("resource")
        .reverse()
        .find((entry) => new URL(entry.name).pathname.endsWith(filename));
      if (!resource) throw new Error(`Loaded module missing: ${filename}`);
      return resource.name;
    };
    const { default: React } = await import(moduleUrl("/react.js"));
    const { default: ReactDOM } = await import(
      moduleUrl("/react-dom_client.js")
    );
    const modulePath = "/src/components/tools/TerminalDock.tsx";
    const { TerminalDock } = await import(modulePath);
    type Mount = {
      expectedScope: string | null;
      request: {
        kind: string;
        sessionId?: string;
        planId?: string;
      } | null;
    };
    const calls: Mount[] = [];
    let mounted: (() => void) | undefined;
    const host = window as unknown as { __TAURI_INTERNALS__?: unknown };
    const previousBridge = host.__TAURI_INTERNALS__;
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: Mount) => {
        if (command !== "mount_terminal_pane") return undefined;
        calls.push(args);
        mounted?.();
        return calls.length;
      },
    };
    const container = document.createElement("div");
    document.body.append(container);
    const root = ReactDOM.createRoot(container);
    const request = {
      scope: "workspace-A",
      kind: "colossus_tui",
      planContext: { sessionId: "session-A", planId: "plan-A" },
      sequence: 1,
    };
    const render = (scope: string, launch: typeof request | null) =>
      new Promise<void>((resolve) => {
        mounted = resolve;
        root.render(
          React.createElement(TerminalDock, {
            ready: true,
            fixture: false,
            scope,
            request: launch,
            onSettings: () => undefined,
          }),
        );
      });
    try {
      await render("workspace-A", request);
      // A workspace change renders before the parent's cleanup effect clears
      // its request. Deliberately keep A's request for B's first render.
      await render("workspace-B", request);
      return calls;
    } finally {
      root.unmount();
      container.remove();
      if (previousBridge === undefined) delete host.__TAURI_INTERNALS__;
      else host.__TAURI_INTERNALS__ = previousBridge;
    }
  });
  expect(mounts[0]).toMatchObject({
    expectedScope: "workspace-A",
    request: {
      kind: "colossus_tui",
      sessionId: "session-A",
      planId: "plan-A",
    },
  });
  expect(mounts.slice(1)).not.toHaveLength(0);
  for (const mount of mounts.slice(1)) {
    expect(mount).toMatchObject({
      expectedScope: "workspace-B",
      request: null,
    });
  }
});

async function mountTerminalIntentHarness(
  page: Page,
  failFirst = false,
  initialRequest = false,
) {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("button", { name: "Open tools", exact: true }),
  ).toBeVisible();
  await page.evaluate(
    async ({ failFirst, initialRequest }) => {
      const moduleUrl = (filename: string) => {
        const resource = performance
          .getEntriesByType("resource")
          .reverse()
          .find((entry) => new URL(entry.name).pathname.endsWith(filename));
        if (!resource) throw new Error(`Loaded module missing: ${filename}`);
        return resource.name;
      };
      const { default: React } = await import(moduleUrl("/react.js"));
      const { default: ReactDOM } = await import(
        moduleUrl("/react-dom_client.js")
      );
      const modulePath = "/src/components/WorkSurface.tsx";
      const { WorkSurface } = await import(modulePath);
      document.getElementById("root")!.hidden = true;
      const container = document.createElement("div");
      container.dataset.testid = "terminal-intent-harness";
      container.dataset.genericSelections = "0";
      document.body.append(container);
      const calls: unknown[] = [];
      (
        window as unknown as { __TAURI_INTERNALS__: unknown }
      ).__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: unknown) => {
          if (command === "browser_context")
            return {
              available: false,
              generation: 0,
              tabs: [],
              selectedTabId: null,
            };
          if (command !== "mount_terminal_pane") return undefined;
          calls.push(args);
          container.dataset.mounts = JSON.stringify(calls);
          if (failFirst && calls.length === 1)
            throw new Error("The first launch failed. Retry the same request.");
          return calls.length;
        },
      };
      const noop = () => undefined;
      const request = {
        scope: "workspace-A",
        kind: "colossus_tui",
        planContext: { sessionId: "session-A", planId: "plan-A" },
        sequence: 1,
      };
      function Harness() {
        const [intent, setIntent] = React.useState(
          initialRequest ? request : null,
        );
        const [tuiReady, setTuiReady] = React.useState(true);
        return React.createElement(
          React.Fragment,
          null,
          React.createElement(
            "button",
            { onClick: () => setIntent(request) },
            "Launch explicit TUI",
          ),
          React.createElement(
            "button",
            { onClick: () => setTuiReady(false) },
            "Stop managed runtime",
          ),
          React.createElement(WorkSurface, {
            title: "Terminal intent test",
            view: undefined,
            conversationViews: [],
            connection: {
              state: "connected",
              message: "Connected",
              targetId: "workspace-A",
            },
            connecting: false,
            cancelling: false,
            runLoadError: "",
            actionError: null,
            participants: [],
            sessionMap: null,
            sessionMapLoading: false,
            sessionMapError: "",
            selectedParticipantId: null,
            delegateView: undefined,
            delegateInspection: null,
            delegateLoading: false,
            delegateError: "",
            artifacts: [],
            selectedSpaceName: "Workspace A",
            threadPinned: false,
            followRequestSequence: 0,
            composer: () => React.createElement("div"),
            filesPanel: React.createElement("div", null, "Workspace files"),
            filesAvailable: true,
            artifactsAvailable: false,
            asideView: undefined,
            asideConversationViews: [],
            asideHistory: [],
            asideBusy: false,
            asideError: null,
            asideReadOnly: false,
            planContinuationAvailable: false,
            planWorkflowAvailable: false,
            workNavigationOpen: false,
            onCloseWorkNavigation: noop,
            onBackToThreadDetails: noop,
            browserScope: "workspace-A",
            terminalSupported: true,
            // Shell remains available when the managed TUI goes offline. An
            // explicit TUI request must still obey its own readiness requirement.
            terminalReady: intent === null || tuiReady,
            terminalRequest: intent,
            onOpenGenericTerminal: () => {
              container.dataset.genericSelections = String(
                Number(container.dataset.genericSelections) + 1,
              );
              setIntent(null);
            },
          }),
        );
      }
      ReactDOM.createRoot(container).render(React.createElement(Harness));
    },
    { failFirst, initialRequest },
  );
  const fixture = page.getByTestId("terminal-intent-harness");
  if (!initialRequest)
    await fixture.getByRole("button", { name: "Launch explicit TUI" }).click();
  await expect(fixture).toHaveAttribute("data-mounts", /session-A/);
  return fixture;
}

test("an explicit terminal request opens when returning from settings without an active run", async ({
  page,
}) => {
  const fixture = await mountTerminalIntentHarness(page, false, true);
  await expect(
    fixture.getByRole("button", { name: "Close tool pane" }),
  ).toBeVisible();
  await expect(fixture).toHaveAttribute("data-generic-selections", "0");
  const mounts = JSON.parse((await fixture.getAttribute("data-mounts"))!);
  expect(mounts).toHaveLength(1);
  expect(mounts[0]).toMatchObject({
    expectedScope: "workspace-A",
    request: { kind: "colossus_tui", sessionId: "session-A", planId: "plan-A" },
  });
});

for (const entry of ["shortcut", "tools menu", "pane selector"] as const) {
  test(`generic Terminal via ${entry} retires a previous explicit TUI request`, async ({
    page,
  }) => {
    const fixture = await mountTerminalIntentHarness(page);
    await fixture.getByRole("button", { name: "Stop managed runtime" }).click();
    await expect(
      fixture.getByRole("heading", { name: "Set up your local terminal" }),
    ).toBeVisible();
    if (entry === "shortcut") {
      await fixture.getByRole("button", { name: "Close tool pane" }).click();
      await fixture
        .getByRole("button", { name: "Open terminal", exact: true })
        .click();
    } else {
      await fixture
        .getByRole("button", {
          name: entry === "tools menu" ? "Open tools" : "Switch pane tool",
          exact: true,
        })
        .click();
      await fixture.getByRole("menuitemradio", { name: /^Terminal/ }).click();
    }
    await expect(fixture).toHaveAttribute("data-generic-selections", "1");
    await expect(
      fixture.getByRole("heading", { name: "Set up your local terminal" }),
    ).toHaveCount(0);
    await expect
      .poll(async () => {
        const mounts = JSON.parse((await fixture.getAttribute("data-mounts"))!);
        return mounts.at(-1);
      })
      .toMatchObject({ expectedScope: "workspace-A", request: null });
  });
}

test("Retry preserves an explicit terminal request without selecting generic Terminal", async ({
  page,
}) => {
  const fixture = await mountTerminalIntentHarness(page, true);
  await fixture.getByRole("button", { name: "Retry", exact: true }).click();
  await expect
    .poll(
      async () =>
        JSON.parse((await fixture.getAttribute("data-mounts"))!).length,
    )
    .toBe(2);
  await expect(fixture).toHaveAttribute("data-generic-selections", "0");
  const mounts = JSON.parse((await fixture.getAttribute("data-mounts"))!);
  expect(mounts[1]).toMatchObject({
    expectedScope: "workspace-A",
    request: { kind: "colossus_tui", sessionId: "session-A", planId: "plan-A" },
    requestSequence: 1,
  });
});

test("Aside composer stays inside the pane while long content scrolls and the window resizes", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 950 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Open tools", exact: true }).click();
  await page.getByRole("menuitemradio", { name: /^Aside/ }).click();
  const aside = page.getByRole("region", {
    name: "Aside conversation",
    exact: true,
  });
  await expect(aside).toBeVisible();
  await page.locator(".aside-scroll").evaluate((element) => {
    const content = document.createElement("p");
    content.textContent =
      "A long conversation should scroll inside its pane. ".repeat(300);
    element.append(content);
  });
  for (const size of [
    { width: 1440, height: 950 },
    { width: 880, height: 640 },
    { width: 390, height: 844 },
    { width: 1100, height: 600 },
  ]) {
    await page.setViewportSize(size);
    await page.evaluate(() => {
      document.documentElement.dataset.textSize = "large";
    });
    const composer = aside.locator(".aside-composer");
    const bounds = await aside.boundingBox();
    const input = await composer.boundingBox();
    expect(Math.abs(input!.x - bounds!.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(input!.width - bounds!.width)).toBeLessThanOrEqual(1);
    expect(input!.y + input!.height).toBeLessThanOrEqual(
      bounds!.y + bounds!.height + 1,
    );
    expect(input!.y + input!.height).toBeLessThanOrEqual(size.height + 1);
    expect(
      await aside
        .locator(".aside-scroll")
        .evaluate((element) => element.scrollHeight > element.clientHeight),
    ).toBe(true);
    await aside
      .getByRole("textbox", { name: "Aside message", exact: true })
      .fill("Keep this composer visible");
    await expect(
      aside.getByRole("button", { name: "Send Aside message" }),
    ).toBeInViewport();
  }
});

test("Tools and pane menus switch from Aside to Thread details and fit the viewport", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/?fixture=operations-studio");
  await page.getByRole("button", { name: "Open tools", exact: true }).click();
  await page.getByRole("menuitemradio", { name: /^Aside/ }).click();
  const aside = page.getByRole("region", {
    name: "Aside conversation",
    exact: true,
  });
  await expect(aside).toBeVisible();
  await aside
    .getByRole("textbox", { name: "Aside message", exact: true })
    .fill("Retain this Aside draft");
  for (const triggerName of ["Open tools", "Switch pane tool"]) {
    await page.getByRole("button", { name: triggerName, exact: true }).click();
    const menu = page.getByRole("menu", { name: "Workspace tools" });
    const bounds = await menu.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(701);
    await page.getByRole("menuitemradio", { name: /^Thread details/ }).click();
    await expect(
      page.getByRole("heading", { name: "Thread details", exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Switch pane tool" }).click();
    await page.getByRole("menuitemradio", { name: /^Aside/ }).click();
    await expect(
      aside.getByRole("textbox", { name: "Aside message", exact: true }),
    ).toHaveValue("Retain this Aside draft");
  }
});
