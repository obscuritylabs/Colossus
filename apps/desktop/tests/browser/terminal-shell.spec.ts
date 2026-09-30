import { expect, test } from "@playwright/test";

test("TUI and shell tabs keep independent sessions and route typed commands", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.goto("/?fixture=operations-studio");
  await expect(
    page.getByRole("button", { name: "Open tools", exact: true }),
  ).toBeVisible();
  await page.evaluate(async () => {
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
    const modulePath = "/src/TerminalWindow.tsx";
    const { default: TerminalWindow } = await import(modulePath);
    document.getElementById("root")!.hidden = true;
    const container = document.createElement("div");
    container.dataset.testid = "shell-harness";
    document.body.append(container);
    const opens: unknown[] = [],
      closes: string[] = [],
      writes: unknown[] = [];
    let first = true;
    let callbackId = 0;
    Object.assign(window, {
      __TAURI_INTERNALS__: {
        transformCallback: () => ++callbackId,
        unregisterCallback: () => {},
        invoke: async (
          command: string,
          args: {
            request?: { kind?: string; sessionId: string; dataBase64: string };
          },
        ) => {
          if (command === "terminal_context") {
            const requestedKind = first ? "colossus_tui" : null;
            first = false;
            return {
              enabled: true,
              shellEnabled: true,
              tuiEnabled: true,
              contextGeneration: 1,
              workspaceId: "test-workspace",
              workspaceName: "Demo workspace",
              launchRequestId: 1,
              requestedKind,
              requestedPlanSessionId: null,
              requestedPlanId: null,
            };
          }
          if (command === "open_terminal") {
            opens.push(args.request);
            container.dataset.opens = JSON.stringify(opens);
            return { sessionId: `${args.request!.kind}-${opens.length}` };
          }
          if (command === "close_terminal") {
            closes.push(args.request!.sessionId);
            container.dataset.closes = JSON.stringify(closes);
          }
          if (command === "write_terminal") {
            writes.push({
              sessionId: args.request!.sessionId,
              text: atob(args.request!.dataBase64),
            });
            container.dataset.writes = JSON.stringify(writes);
          }
        },
      },
    });
    ReactDOM.createRoot(container).render(React.createElement(TerminalWindow));
  });
  const fixture = page.getByTestId("shell-harness");
  const tabs = page.getByRole("navigation", { name: "Terminal sessions" });
  await expect(
    tabs.getByRole("button", { name: "Colossus TUI 1", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "New shell", exact: true }).click();
  await expect(
    tabs.getByRole("button", { name: "Shell 1", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await expect(fixture).toHaveAttribute("data-opens", /"kind":"shell"/);
  const input = page.locator(
    ".terminal-pane:not([hidden]) .xterm-helper-textarea",
  );
  await input.pressSequentially("Get-Location");
  await input.press("Enter");
  await expect
    .poll(async () => {
      const writes = JSON.parse(
        (await fixture.getAttribute("data-writes")) ?? "[]",
      );
      return writes.map((entry: { text: string }) => entry.text).join("");
    })
    .toBe("Get-Location\r");
  const writes = JSON.parse((await fixture.getAttribute("data-writes"))!);
  expect(
    writes.every(
      (entry: { sessionId: string }) => entry.sessionId === "shell-2",
    ),
  ).toBe(true);
  await tabs
    .getByRole("button", { name: "Colossus TUI 1", exact: true })
    .click();
  expect(JSON.parse((await fixture.getAttribute("data-opens"))!)).toHaveLength(
    2,
  );
  expect(await fixture.getAttribute("data-closes")).toBeNull();
  await tabs
    .getByRole("button", { name: "Close Shell 1", exact: true })
    .click();
  await expect(fixture).toHaveAttribute("data-closes", '["shell-2"]');
  await expect(
    tabs.getByRole("button", { name: "Colossus TUI 1", exact: true }),
  ).toHaveAttribute("aria-current", "page");
});
