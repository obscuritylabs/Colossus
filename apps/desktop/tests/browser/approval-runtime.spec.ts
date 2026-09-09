import { expect, test } from "@playwright/test";
import { createServer } from "node:http";
import { createInterface } from "node:readline";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { AcceptanceProcesses } from "./support/acceptance-processes";
import { approvalMarkerCommand } from "./support/approval-command";

test.skip(
  process.env.COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE !== "1",
  "Run npm run test:approval-runtime",
);

async function removePrivateFixture(path: string): Promise<void> {
  // Only this test's fresh mkdtemp tree, after its processes exit. Never follow links.
  await chmod(path, 0o700);
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink())
      await removePrivateFixture(join(path, entry.name));
  }
  await rm(path, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 100,
  });
}

for (const outcome of ["allow", "deny", "cancel"] as const) {
  test(`browser → production review adapter → authenticated worker and broker: ${outcome}`, async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const root = await realpath(
      await mkdtemp(
        join(process.platform === "win32" ? homedir() : tmpdir(), "ca-"),
      ),
    );
    await chmod(root, 0o700);
    const workspace = join(root, "work");
    await mkdir(workspace, { mode: 0o700 });
    await mkdir(join(root, "instance"), { mode: 0o700 });
    const sidecar = join(
      root,
      process.platform === "win32"
        ? "colossus-sidecar.exe"
        : "colossus-sidecar",
    );
    await cp(process.env.COLOSSUS_APPROVAL_TEST_SIDECAR!, sidecar);
    await chmod(sidecar, 0o500);
    const marker = join(workspace, "approved-marker.txt");
    const requests: string[] = [];
    const server = createServer(async (request, response) => {
      let input = "";
      for await (const chunk of request) input += chunk.toString();
      requests.push(input);
      const script = approvalMarkerCommand(
        process.platform === "win32",
        outcome === "allow",
      );
      const delta =
        requests.length === 1
          ? {
              tool_calls: [
                {
                  index: 0,
                  id: "approval-command",
                  type: "function",
                  function: {
                    name: "shell_run",
                    arguments: JSON.stringify({
                      command: script,
                      cwd: ".",
                      justification:
                        "Verify command approval with a local marker.",
                    }),
                  },
                },
              ],
            }
          : { content: "Command finished." };
      response.writeHead(200, { "content-type": "text/event-stream" });
      response.end(
        `data: ${JSON.stringify({ id: `turn-${requests.length}`, choices: [{ index: 0, delta, finish_reason: requests.length === 1 ? "tool_calls" : "stop" }] })}\n\ndata: [DONE]\n\n`,
      );
    });
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("loopback server unavailable");
    const processes = new AcceptanceProcesses();
    const bridge = processes.start(
      process.env.COLOSSUS_APPROVAL_TEST_BRIDGE!,
      [sidecar, root, `http://127.0.0.1:${address.port}/v1`],
      {
        cwd: workspace,
        env: { ...process.env, HOME: root, COLOSSUS_HOME: join(root, "home") },
        stdio: "pipe",
      },
    );
    let diagnostic = "";
    bridge.stderr?.on("data", (chunk: Buffer) => {
      diagnostic = (diagnostic + chunk.toString()).slice(-64 * 1024);
    });
    const lines: unknown[] = [];
    const reader = createInterface({ input: bridge.stdout! });
    reader.on("line", (line) => {
      lines.push(JSON.parse(line));
    });
    const pendingLines = () => {
      if (bridge.exitCode !== null || bridge.signalCode !== null)
        throw new Error(`Native approval bridge exited: ${diagnostic}`);
      return lines.length;
    };
    let chain: Promise<unknown> = Promise.resolve();
    const invoke = (command: string, args: unknown = {}) => {
      const pending = chain.then(async () => {
        bridge.stdin!.write(`${JSON.stringify({ command, args })}\n`);
        // Only terminal activity collection waits through the normal effect
        // budget: 45 seconds in the bridge plus bounded diagnostic/IPC time.
        await expect
          .poll(pendingLines, {
            timeout: command === "released_activity" ? 55_000 : 15_000,
            message: `native approval bridge: ${command}`,
          })
          .toBeGreaterThan(0);
        const result = lines.shift() as {
          result?: unknown;
          error?: string;
          diagnostics?: { run_status: string; pending_count?: number };
        };
        if (result.error)
          throw new Error(
            `${command}: ${result.error}${result.diagnostics ? ` (${JSON.stringify(result.diagnostics)})` : ""}`,
          );
        return result.result;
      });
      chain = pending.catch(() => undefined);
      return pending;
    };
    try {
      await expect
        .poll(pendingLines, {
          timeout: 45_000,
          message: "native sidecar approval readiness",
        })
        .toBeGreaterThan(0);
      expect(lines.shift(), diagnostic).toEqual({ ready: true });
      expect(requests[0]).toContain("justification");
      expect(await readFile(marker, "utf8").catch(() => "")).toBe("");
      await page.exposeFunction("nativeCommandApproval", invoke);
      await page.addInitScript(() =>
        Object.assign(window, {
          __TAURI_INTERNALS__: {
            invoke: (command: string, args: unknown) =>
              (
                window as unknown as {
                  nativeCommandApproval: (
                    command: string,
                    args: unknown,
                  ) => Promise<unknown>;
                }
              ).nativeCommandApproval(command, args),
          },
        }),
      );
      await page.goto("/?surface=command-approval");
      const full = page.getByRole("region", {
        name: "Full prepared argument vector, executable first",
      });
      await expect(full).toContainText("COMMAND_TAIL");
      await expect(
        page.getByText("Verify command approval with a local marker.", {
          exact: true,
        }),
      ).toBeVisible();
      await expect(full).not.toContainText("fixture-private-token");
      await expect(full).not.toContainText("fixture-bearer-suffix");
      await expect(full).not.toContainText("fixture-concat-tail");
      await expect(full).not.toContainText("fixture-name-tail");
      await expect(full).not.toContainText("fixture-keygen-tail");
      await expect(full).not.toContainText("fixture-storepass-tail");
      await expect(full).not.toContainText("fixture-pwri-tail");
      await expect(full).not.toContainText("fixture-openpgp-tail");
      await full.focus();
      await page.keyboard.press("End");
      await expect
        .poll(() =>
          full.evaluate(
            (element) =>
              element.scrollTop + element.clientHeight >=
              element.scrollHeight - 1,
          ),
        )
        .toBe(true);
      expect(await readFile(marker, "utf8").catch(() => "")).toBe("");
      if (outcome === "cancel") {
        await invoke("cancel_run");
        await page
          .getByRole("button", { name: "Continue to native confirmation" })
          .click();
        await expect(page.getByRole("alert")).toContainText(
          "changed or expired",
        );
      } else {
        await page
          .getByRole("button", {
            name:
              outcome === "allow" ? "Continue to native confirmation" : "Deny",
            exact: true,
          })
          .click();
        await expect(
          page.getByRole("button", { name: "Awaiting native confirmation…" }),
        ).toBeDisabled();
        // Re-fetch after submission must reject the no-longer-pending challenge.
        await expect(invoke("command_review_context")).rejects.toThrow(
          "stale or unavailable",
        );
      }
      const activity = (await invoke("released_activity")) as {
        name: string;
        state: string;
        input: string | null;
        preview: string | null;
      }[];
      const shell = activity.filter((item) => item.name === "shell.run");
      expect(shell.length).toBeGreaterThan(0);
      expect(shell.every((item) => item.input === null)).toBe(true);
      const released = JSON.stringify(shell);
      for (const withheld of [
        "fixture-private-token",
        "fixture-bearer-suffix",
        "fixture-concat-tail",
        "fixture-name-tail",
        "fixture-keygen-tail",
        "fixture-storepass-tail",
        "fixture-pwri-tail",
        "fixture-openpgp-tail",
        "resolved_argv",
        "invocation",
      ])
        expect(released).not.toContain(withheld);
      if (outcome === "allow") {
        const completed = shell.find((item) => item.state === "Completed");
        expect(completed, released).toBeDefined();
        expect(JSON.parse(completed!.preview!)).toMatchObject({
          exit_code: 0,
          command_details_withheld: true,
        });
        expect(await readFile(marker, "utf8")).toBe(
          process.platform === "win32" ? "approved\r\n" : "approved\n",
        );
      } else {
        expect(await readFile(marker, "utf8").catch(() => "")).toBe("");
      }
      await page.screenshot({
        path: `output/playwright/command-approval-${outcome}.png`,
      });
    } finally {
      await page.goto("about:blank");
      bridge.stdin?.end(`${JSON.stringify({ command: "close" })}\n`);
      await new Promise<void>((resolve) => {
        if (bridge.exitCode !== null || bridge.signalCode !== null)
          return resolve();
        const deadline = setTimeout(resolve, 15_000);
        bridge.once("close", () => {
          clearTimeout(deadline);
          resolve();
        });
      });
      reader.close();
      await processes.close();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      await removePrivateFixture(root);
    }
  });
}
