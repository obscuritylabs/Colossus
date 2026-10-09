import { expect, test } from "@playwright/test";
import { createServer } from "node:net";
import { processRuntimeHost } from "./support/process-runtime-host";
interface ShellSummary {
  id: string;
  created_at_ms: number;
  status: string;
  exit_code: number | null;
}
interface ShellPage {
  sessions: ShellSummary[];
}
interface ShellSnapshot {
  session: ShellSummary;
  chunks: { stdout: string; stderr: string }[];
}

test.skip(
  process.env.COLOSSUS_APPROVAL_RUNTIME_ACCEPTANCE !== "1",
  "Run the native approval acceptance tier",
);

test("real shell output survives small budgets and managed short exits report their outcome", async () => {
  test.setTimeout(180000);
  const host = await processRuntimeHost();
  try {
    for (const managed of [false, true]) {
      for (const limit of [1024, 4096]) {
        for (const exitCode of [0, 7]) {
          const command =
            process.platform === "win32"
              ? `[Console]::Out.WriteLine('SMALL_OUT'); [Console]::Error.WriteLine('SMALL_ERR'); exit ${exitCode}`
              : `printf 'SMALL_OUT\\n'; printf 'SMALL_ERR\\n' >&2; exit ${exitCode}`;
          const result = await host.run({
            command,
            max_output_bytes: limit,
            ...(managed ? { yield_time_ms: 1000, lifetime: "workspace" } : {}),
          });
          expect(result.approvals).toBe(1);
          const previews = result.activity.activity.flatMap((item) =>
            item.preview ? [JSON.parse(item.preview)] : [],
          );
          expect(previews).toHaveLength(1);
          let output = previews[0];
          if (managed && output.exit_code === null) {
            // Cold Windows process startup may exceed the requested one-second yield.
            // The preview must honestly report an active process, never success.
            expect(["starting", "running"]).toContain(output.status);
            const page = (await host.invoke(
              "list_shell_sessions",
            )) as ShellPage;
            const latest = page.sessions.sort(
              (a, b) => b.created_at_ms - a.created_at_ms,
            )[0];
            const snapshot = (await host.invoke("read_shell_session", {
              sessionId: latest.id,
              waitMs: 10000,
            })) as ShellSnapshot;
            output = {
              stdout: snapshot.chunks.map((c) => c.stdout).join(""),
              stderr: snapshot.chunks.map((c) => c.stderr).join(""),
              exit_code: snapshot.session.exit_code,
              status: snapshot.session.status,
            };
          }
          expect(output.stdout, JSON.stringify(previews)).toContain(
            "SMALL_OUT",
          );
          expect(output.stderr).toContain("SMALL_ERR");
          expect(output.exit_code).toBe(exitCode);
          if (managed) expect(output.status).toBe("exited");
          expect(previews[0].command_details_withheld).toBe(true);
          expect(JSON.stringify(previews)).not.toContain("[Console]");
          if (!managed || previews[0].exit_code !== null) {
            expect(result.activity.activity.at(-1)!.state).toBe(
              exitCode === 0 ? "Completed" : "Failed",
            );
          }
        }
      }
    }
  } finally {
    await host.close();
  }
});

test("real workspace server survives later turns, reconnects in Active shells, and stops", async ({
  page,
}) => {
  test.skip(
    process.platform !== "win32",
    "Windows on-device server acceptance",
  );
  test.setTimeout(120000);
  const host = await processRuntimeHost();
  const occupied = createServer();
  await new Promise<void>((resolve) =>
    occupied.listen(0, "127.0.0.1", resolve),
  );
  try {
    const bound = occupied.address();
    if (!bound || typeof bound === "string")
      throw new Error("missing occupied test port");
    const failed = await host.run({
      command: `$ErrorActionPreference='Stop'; try { $l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,${bound.port}); $l.Start() } catch { [Console]::Error.WriteLine('PORT_IN_USE'); exit 9 }`,
      max_output_bytes: 4096,
      yield_time_ms: 1000,
      lifetime: "workspace",
    });
    let shells = (await host.invoke("list_shell_sessions")) as ShellPage;
    const collision = (await host.invoke("read_shell_session", {
      sessionId: shells.sessions[0].id,
      waitMs: 10000,
    })) as ShellSnapshot;
    expect(collision.session.exit_code, JSON.stringify(failed)).toBe(9);
    expect(collision.chunks.map((c) => c.stderr).join("")).toContain(
      "PORT_IN_USE",
    );
    const command =
      "$ErrorActionPreference='Stop'; $l=[Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback,0); $l.Start(); [Console]::Out.WriteLine(('READY_PORT='+$l.LocalEndpoint.Port)); while ($true) { $c=$l.AcceptTcpClient(); $s=$c.GetStream(); $b=[Text.Encoding]::ASCII.GetBytes(\"HTTP/1.1 200 OK`r`nContent-Length: 2`r`nConnection: close`r`n`r`nOK\"); $s.Write($b,0,$b.Length); $c.Close() }";
    const started = await host.run({
      command,
      max_output_bytes: 4096,
      yield_time_ms: 1000,
      lifetime: "workspace",
    });
    await host.run(null, started.run.session_id);
    shells = (await host.invoke("list_shell_sessions")) as ShellPage;
    const server = shells.sessions.find((s) => s.status === "running")!;
    expect(server).toBeDefined();
    const snapshot = (await host.invoke("read_shell_session", {
      sessionId: server.id,
    })) as ShellSnapshot;
    const port = /READY_PORT=(\d+)/.exec(
      snapshot.chunks.map((c) => c.stdout).join(""),
    )?.[1];
    expect(port).toBeDefined();
    expect(
      await (
        await fetch(`http://127.0.0.1:${port}`, {
          signal: AbortSignal.timeout(5000),
        })
      ).text(),
    ).toBe("OK");
    let disconnected = true;
    await page.exposeFunction(
      "processFixtureInvoke",
      async (command: string, args: unknown) => {
        if (command === "list_setup_packages") return [];
        if (!command.includes("shell_session")) return null;
        if (disconnected) throw new Error("fixture disconnect");
        return host.invoke(command, args);
      },
    );
    await page.addInitScript(() =>
      Object.assign(window, {
        __TAURI_INTERNALS__: {
          invoke: (command: string, args: unknown) =>
            (
              window as unknown as {
                processFixtureInvoke(
                  command: string,
                  args: unknown,
                ): Promise<unknown>;
              }
            ).processFixtureInvoke(command, args),
        },
      }),
    );
    await page.goto("/?fixture=operations-studio");
    await page.evaluate(async () => {
      const path = "/src/dev/active-shells-harness.tsx";
      const module = await import(/* @vite-ignore */ path);
      (window as unknown as { stopProcessFixture(): void }).stopProcessFixture =
        module.mountLive("isolated-fixture");
    });
    const panel = page.getByRole("region", {
      name: "Active shells",
      exact: true,
    });
    await expect(panel.locator(".shells-toolbar")).toContainText("Unavailable");
    disconnected = false;
    await panel.getByRole("button", { name: "Refresh", exact: true }).click();
    await expect(panel.locator(".shells-toolbar")).toContainText(
      "Active shells · 1",
    );
    await panel.getByRole("button", { name: /running · workspace/ }).click();
    await expect(panel.getByLabel("Released shell output")).toContainText(
      `READY_PORT=${port}`,
    );
    await panel.getByRole("button", { name: "Stop", exact: true }).click();
    await expect(panel.locator(".shells-detail strong")).toHaveText(
      /^stopped(?: · exit -?\d+)?$/,
      {
        timeout: 15000,
      },
    );
    await expect
      .poll(async () =>
        fetch(`http://127.0.0.1:${port}`, {
          signal: AbortSignal.timeout(1000),
        }).then(
          () => false,
          () => true,
        ),
      )
      .toBe(true);
    await page.screenshot({
      path: "output/playwright/managed-shell-stopped.png",
    });
    await page.evaluate(() =>
      (
        window as unknown as { stopProcessFixture(): void }
      ).stopProcessFixture(),
    );
  } finally {
    await new Promise<void>((resolve) => occupied.close(() => resolve()));
    await host.close();
  }
});
