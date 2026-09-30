import { expect, test } from "@playwright/test";

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
