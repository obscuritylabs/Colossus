import { expect, test } from "@playwright/test";

test("released output reconnects, follows cursors, and confirms stop", async ({
  page,
}) => {
  const session = {
    id: "shell-1",
    session_id: "chat-1",
    run_id: "run-1",
    lifetime: "workspace" as const,
    status: "running" as const,
    command: "python3 -m http.server 8000",
    cwd: "/workspace/demo",
    created_at_ms: Date.now() - 12000,
    deadline_ms: Date.now() + 900000,
    exit_code: null,
    reason: null,
    truncated: false,
    output_sequence: 1,
  };
  await page.addInitScript(
    ({ session }) => {
      const host = window as unknown as {
        __TAURI_INTERNALS__: unknown;
        shellCalls: { command: string; args: Record<string, unknown> }[];
        unmountShells: () => void;
      };
      host.shellCalls = [];
      let firstRead = true;
      let stopping = false;
      let stopReads = 0;
      host.__TAURI_INTERNALS__ = {
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          if (command === "list_setup_packages") return [];
          if (!command.endsWith("shell_session")) return null;
          host.shellCalls.push({ command, args });
          if (command === "read_shell_session" && firstRead) {
            firstRead = false;
            throw new Error("Disconnected");
          }
          if (command === "stop_shell_session") stopping = true;
          const status = stopping
            ? command === "read_shell_session" && ++stopReads > 0
              ? "stopped"
              : "stopping"
            : "running";
          return {
            session: { ...session, status },
            chunks:
              args.afterSequence === 0
                ? [
                    {
                      sequence: 1,
                      stdout:
                        "Listening on 8000\n<script>window.injected=true</script>\nGET / 200\n",
                      stderr: "",
                    },
                  ]
                : [],
            next_sequence: 1,
            gap: false,
          };
        },
      };
    },
    { session },
  );
  await page.goto("/?fixture=operations-studio");
  await page.evaluate(async (session) => {
    // Vite serves this test-only entry; the app never references it.
    const harnessPath = "/src/dev/active-shells-harness.tsx";
    const module = await import(/* @vite-ignore */ harnessPath);
    (window as unknown as { unmountShells: () => void }).unmountShells =
      module.mount(session);
  }, session);
  const panel = page.getByRole("region", {
    name: "Active shells",
    exact: true,
  });
  await panel.getByRole("button", { name: /python3/ }).click();
  await expect(panel.getByRole("alert")).toContainText("Refresh to reconnect");
  await panel.getByRole("button", { name: "Refresh", exact: true }).click();
  const output = panel.getByLabel("Released shell output");
  await expect(output).toContainText("Listening on 8000");
  await expect(output).toContainText("<script>");
  expect(
    await page.evaluate(
      () => (window as unknown as { injected?: boolean }).injected,
    ),
  ).toBeUndefined();
  await panel.getByRole("textbox", { name: "Search shell output" }).fill("GET");
  await expect(output).toHaveText("GET / 200");
  await panel.getByRole("textbox", { name: "Search shell output" }).fill("");
  await panel.getByRole("checkbox", { name: "Follow" }).uncheck();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as {
            shellCalls: { args: { afterSequence?: number } }[];
          }
        ).shellCalls.some((call) => call.args.afterSequence === 1),
      ),
    )
    .toBe(true);
  await page.screenshot({ path: "/private/tmp/colossus-active-shells.png" });
  await panel.getByRole("button", { name: "Stop", exact: true }).click();
  await expect(panel.getByRole("button", { name: "Stopping…" })).toBeDisabled();
  await expect(panel.locator(".shells-detail strong")).toHaveText("stopped");
  await expect(panel.locator(".shells-detail")).not.toContainText("s elapsed");
  await page.evaluate(() =>
    (window as unknown as { unmountShells: () => void }).unmountShells(),
  );
  const calls = await page.evaluate(
    () => (window as unknown as { shellCalls: unknown[] }).shellCalls.length,
  );
  await page.waitForTimeout(1100);
  expect(
    await page.evaluate(
      () => (window as unknown as { shellCalls: unknown[] }).shellCalls.length,
    ),
  ).toBe(calls);
});

test("unsupported runtimes do not poll and removing support cancels polling", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: unknown;
      shellListCalls: number;
    };
    host.shellListCalls = 0;
    host.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
        if (command === "list_setup_packages") return [];
        if (command === "list_shell_sessions") {
          host.shellListCalls++;
          return { sessions: [], next_cursor: null };
        }
        return null;
      },
    };
  });
  await page.goto("/?fixture=operations-studio");
  await page.clock.install();
  await page.evaluate(async () => {
    const path = "/src/dev/active-shells-harness.tsx";
    const module = await import(/* @vite-ignore */ path);
    (window as unknown as { shellPolling: unknown }).shellPolling =
      module.mountPolling();
  });
  await expect(page.getByTestId("shell-polling")).toHaveText("0");
  const count = () =>
    page.evaluate(
      () => (window as unknown as { shellListCalls: number }).shellListCalls,
    );
  await page.clock.runFor(6000);
  expect(await count()).toBe(0);
  await page.evaluate(() =>
    (
      window as unknown as {
        shellPolling: { setScope(scope: string | null): void };
      }
    ).shellPolling.setScope("supported-runtime"),
  );
  await expect.poll(count).toBe(1);
  await page.clock.runFor(5000);
  await expect.poll(count).toBe(2);
  await page.evaluate(() =>
    (
      window as unknown as {
        shellPolling: { setScope(scope: string | null): void };
      }
    ).shellPolling.setScope(null),
  );
  await page.clock.runFor(100);
  const stoppedCount = await count();
  await page.clock.runFor(10000);
  expect(await count()).toBe(stoppedCount);
  await page.evaluate(() =>
    (
      window as unknown as { shellPolling: { unmount(): void } }
    ).shellPolling.unmount(),
  );
});
