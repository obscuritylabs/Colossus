import { expect, test, type Page, type WebSocketRoute } from "@playwright/test";
import type {
  HumanTerminalOperation,
  HumanTerminalReply,
  HumanTerminalSession,
} from "../../src/api";
import { browserFixture } from "./fixture";
import { appearanceCases, selectAppearance } from "./appearance";

async function terminalFixture(
  page: Page,
  options: { gap?: boolean; terminal?: boolean; existing?: boolean } = {},
) {
  const fixture = await browserFixture(page, {
    terminal: options.terminal ?? true,
  });
  await page.route("**/nodes/browser-node/analytics?*", (route) =>
    route.fulfill({
      json: {
        window_days: 7,
        counts: { runs: 0, completed: 0, failed: 0, active: 0, queued: 0 },
        activity: [],
        usage: {
          input_tokens: null,
          output_tokens: null,
          estimated_cost: null,
          currency: null,
        },
      },
    }),
  );
  const operations: HumanTerminalOperation[] = [];
  let socket: WebSocketRoute | null = null;
  let session: HumanTerminalSession | null = options.existing
    ? {
        terminal_id: "fixture-terminal",
        controller_id: "another-tab",
        writer_epoch: 1,
        rows: 24,
        cols: 80,
        status: "running",
        first_sequence: 1,
        next_sequence: 1,
        complete_history: true,
        exit_code: null,
      }
    : null;
  const frames = [
    {
      sequence: 1,
      kind: {
        kind: "output" as const,
        data_base64: Buffer.from(
          "Retained shell output 界\r\n\u001b]52;c;dW50cnVzdGVkIGNsaXBib2FyZA==\u0007$ ",
        ).toString("base64"),
      },
    },
  ];
  const reply = (
    patch: Partial<HumanTerminalReply> = {},
  ): HumanTerminalReply => ({
    available: true,
    incarnation: "fixture-incarnation",
    sessions: session ? [session] : [],
    session,
    frames: [],
    next_sequence: 0,
    gap: false,
    input_sequence: 0,
    ...patch,
  });
  await page.routeWebSocket(
    "**/api/projects/browser-project/nodes/browser-node/terminal",
    (route) => {
      socket = route;
      route.onMessage(async (data) => {
        const { id, operation } = JSON.parse(data.toString()) as {
          id: string;
          operation: HumanTerminalOperation;
        };
        operations.push(operation);
        let result = reply();
        if (operation.operation === "open") {
          session = {
            terminal_id: "fixture-terminal",
            controller_id: operation.controller_id,
            writer_epoch: 1,
            rows: operation.rows,
            cols: operation.cols,
            status: "running",
            first_sequence: 1,
            next_sequence: 1,
            complete_history: true,
            exit_code: null,
          };
          result = reply();
        } else if (operation.operation === "attach" && session) {
          if (
            session.controller_id !== operation.controller_id &&
            !operation.takeover
          ) {
            route.send(JSON.stringify({ id, error: { code: "conflict" } }));
            return;
          }
          session = {
            ...session,
            controller_id: operation.controller_id,
            writer_epoch: session.writer_epoch + 1,
          };
          result = reply();
        } else if (operation.operation === "read") {
          if (operation.after_sequence === 0)
            result = reply({
              frames,
              next_sequence: 1,
              gap: options.gap ?? false,
            });
          else {
            await new Promise((resolve) => setTimeout(resolve, 100));
            result = reply({ next_sequence: operation.after_sequence });
          }
        } else if (operation.operation === "write")
          result = reply({ input_sequence: operation.input_sequence });
        else if (operation.operation === "close" && session) {
          session = { ...session, status: "exited", exit_code: 0 };
          result = reply();
        }
        try {
          route.send(JSON.stringify({ id, reply: result }));
        } catch {
          /* The operator detached while this read was in flight. */
        }
      });
    },
  );
  return {
    ...fixture,
    operations,
    disconnect: () =>
      socket?.close({ code: 1012, reason: "Synthetic connection loss" }),
  };
}
const workspaceUrl = "/projects/browser-project/agents/browser-node/overview";

test("Fleet shell supports keyboard input, detach, fresh-page replay and explicit close", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const copies: string[] = [];
    Object.assign(window, { terminalCopies: copies });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text: string) => {
          copies.push(text);
        },
      },
    });
  });
  const fixture = await terminalFixture(page);
  await page.setViewportSize({ width: 1210, height: 954 });
  await page.goto(workspaceUrl);
  await page
    .getByRole("button", { name: "Open Shell", exact: true })
    .press("Enter");
  const dialog = page.getByRole("dialog", {
    name: "Shell · Browser test workspace",
  });
  await expect(dialog.getByRole("status")).toHaveText("Connected");
  await expect(dialog.locator(".xterm-rows")).toContainText(
    "Retained shell output 界",
  );
  expect(
    await page.evaluate(
      () => (window as unknown as { terminalCopies: string[] }).terminalCopies,
    ),
  ).toEqual([]);
  const input = page.getByRole("textbox", {
    name: "Shell input for Browser test workspace",
  });
  await input.pressSequentially("echo ");
  await page.keyboard.insertText("界");
  const announcements = dialog.getByRole("button", {
    name: "Screen reader announcements",
    exact: true,
  });
  await expect(announcements).toHaveAttribute("aria-pressed", "false");
  await announcements.click();
  await expect(announcements).toHaveAttribute("aria-pressed", "true");
  await expect(dialog.locator(".xterm-accessibility")).toContainText(
    "Retained shell output 界",
  );
  // Playwright cannot drive an OS IME; exercise the browser composition boundary.
  await input.evaluate(async (element: HTMLTextAreaElement) => {
    element.dispatchEvent(
      new CompositionEvent("compositionstart", { bubbles: true }),
    );
    element.value += "文";
    element.dispatchEvent(
      new CompositionEvent("compositionupdate", { data: "文", bubbles: true }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    element.dispatchEvent(
      new CompositionEvent("compositionend", { data: "文", bubbles: true }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await input.press("Enter");
  await input.press("Control+c");
  await expect
    .poll(() =>
      fixture.operations
        .filter((operation) => operation.operation === "write")
        .map((operation) =>
          Buffer.from(operation.data_base64, "base64").toString(),
        )
        .join(""),
    )
    .toContain("echo 界文\r\u0003");
  const writesBeforeCopy = fixture.operations.filter(
    (operation) => operation.operation === "write",
  ).length;
  const screen = (await dialog.locator(".xterm-screen").boundingBox())!;
  await page.mouse.move(screen.x + 2, screen.y + 8);
  await page.mouse.down();
  await page.mouse.move(screen.x + 135, screen.y + 8, { steps: 10 });
  await page.mouse.up();
  await input.press("Control+c");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as unknown as { terminalCopies: string[] }).terminalCopies.join(
          "",
        ),
      ),
    )
    .toContain("Retained");
  expect(
    fixture.operations.filter((operation) => operation.operation === "write"),
  ).toHaveLength(writesBeforeCopy);
  await page.setViewportSize({ width: 900, height: 700 });
  await expect
    .poll(
      () =>
        fixture.operations.filter(
          (operation) => operation.operation === "resize",
        ).length,
    )
    .toBeGreaterThan(1);
  await dialog.getByRole("button", { name: "Detach", exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(
    fixture.operations.some((operation) => operation.operation === "close"),
  ).toBe(false);
  await page.reload();
  await page.getByRole("button", { name: "Open Shell", exact: true }).click();
  await expect(dialog.getByRole("status")).toHaveText("Connected");
  expect(
    fixture.operations.filter((operation) => operation.operation === "open"),
  ).toHaveLength(1);
  expect(fixture.operations).toContainEqual(
    expect.objectContaining({
      operation: "attach",
      terminal_id: "fixture-terminal",
      takeover: false,
    }),
  );
  expect(
    fixture.operations.filter(
      (operation) =>
        operation.operation === "read" && operation.after_sequence === 0,
    ),
  ).toHaveLength(2);
  await fixture.disconnect();
  await expect(
    dialog.getByRole("button", { name: "Reconnect", exact: true }),
  ).toBeVisible();
  const writes = fixture.operations.filter(
    (operation) => operation.operation === "write",
  ).length;
  await dialog.getByRole("button", { name: "Reconnect", exact: true }).click();
  await expect(dialog.getByRole("status")).toHaveText("Connected");
  expect(
    fixture.operations.filter((operation) => operation.operation === "write"),
  ).toHaveLength(writes);
  await dialog
    .getByRole("button", { name: "Close Shell", exact: true })
    .click();
  await expect(dialog).toBeHidden();
  expect(
    fixture.operations.filter((operation) => operation.operation === "close"),
  ).toHaveLength(1);
  fixture.verify();
});

test("Fleet shell shows a history gap without accepting input or displaying an incomplete suffix", async ({
  page,
}) => {
  const fixture = await terminalFixture(page, { gap: true });
  await page.goto(workspaceUrl);
  await page.getByRole("button", { name: "Open Shell", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("status")).toContainText(
    "Earlier output is no longer retained",
  );
  await expect(dialog.locator(".xterm-rows")).not.toContainText(
    "Retained shell output",
  );
  await expect(
    dialog.getByRole("button", { name: "Interrupt", exact: true }),
  ).toBeDisabled();
  expect(
    fixture.operations.some((operation) => operation.operation === "write"),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Detach", exact: true }).click();
  fixture.verify();
});

test("another browser explicitly takes control of an existing shell without creating a process", async ({
  page,
}) => {
  const fixture = await terminalFixture(page, { existing: true });
  await page.goto(workspaceUrl);
  await page.getByRole("button", { name: "Open Shell", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("status")).toContainText(
    "Another tab controls this shell",
  );
  expect(
    fixture.operations.some(
      (operation) =>
        operation.operation === "open" || operation.operation === "attach",
    ),
  ).toBe(false);
  await dialog
    .getByRole("button", { name: "Take control", exact: true })
    .click();
  await expect(dialog.getByRole("status")).toHaveText("Connected");
  expect(fixture.operations).toContainEqual(
    expect.objectContaining({
      operation: "attach",
      terminal_id: "fixture-terminal",
      takeover: true,
    }),
  );
  expect(
    fixture.operations.some((operation) => operation.operation === "open"),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Detach", exact: true }).click();
  fixture.verify();
});

test("Fleet shell remains unavailable without the explicit capability and permission", async ({
  page,
}) => {
  const fixture = await terminalFixture(page, { terminal: false });
  await page.goto(workspaceUrl);
  await expect(
    page.getByRole("heading", { name: "Browser test workspace", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open Shell", exact: true }),
  ).toHaveCount(0);
  expect(fixture.operations).toEqual([]);
  fixture.verify();
});

for (const appearance of appearanceCases) {
  test(`Fleet shell fits a narrow Large-text ${appearance.label} workspace`, async ({
    page,
  }) => {
    const fixture = await terminalFixture(page);
    await selectAppearance(page, appearance);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(workspaceUrl);
    await page.getByRole("button", { name: "Open Shell", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("status")).toHaveText("Connected");
    const bounds = await dialog.boundingBox();
    expect(bounds!.width).toBeLessThanOrEqual(390);
    expect(bounds!.height).toBeLessThanOrEqual(844);
    await expect(
      dialog.getByRole("button", { name: "Detach", exact: true }),
    ).toBeInViewport();
    await expect(
      dialog.getByRole("textbox", {
        name: "Shell input for Browser test workspace",
      }),
    ).toBeFocused();
    await page.screenshot({
      path: `output/playwright/fleet-terminal-${appearance.id}-large.png`,
    });
    await page
      .getByRole("textbox", { name: "Shell input for Browser test workspace" })
      .press("Shift+Escape");
    await expect(dialog).toBeHidden();
    fixture.verify();
  });
}
