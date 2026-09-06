import { expect, test } from "@playwright/test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { AcceptanceProcesses } from "./support/acceptance-processes";

test("acceptance commands retain complete output and report a failed exit after close", async () => {
  const processes = new AcceptanceProcesses();
  try {
    const output = await processes.execute(
      process.execPath,
      ["-e", 'process.stdout.write("x".repeat(256 * 1024))'],
      tmpdir(),
      process.env,
    );
    expect(output).toBe("x".repeat(256 * 1024));
    await expect(
      processes.execute(
        process.execPath,
        ["-e", 'process.stderr.write("fixture failure"); process.exitCode = 7'],
        tmpdir(),
        process.env,
      ),
    ).rejects.toThrow("Acceptance command exited 7: fixture failure");
    expect(processes.activeCount).toBe(0);
  } finally {
    await processes.close();
  }
});

test("acceptance shutdown closes an in-flight bridge before deleting its working directory", async () => {
  const fixture = await mkdtemp(join(tmpdir(), "colossus-process-test-"));
  const processes = new AcceptanceProcesses();
  const outcome = processes
    .execute(
      process.execPath,
      [
        "-e",
        'require("node:fs").writeFileSync("ready", "ready"); setInterval(() => {}, 1000)',
      ],
      fixture,
      process.env,
    )
    .catch((error: unknown) => error);
  try {
    await expect
      .poll(() => readFile(join(fixture, "ready"), "utf8").catch(() => ""))
      .toBe("ready");
    expect(processes.activeCount).toBe(1);
    await processes.close();
    expect(await outcome).toBeInstanceOf(Error);
    expect(processes.activeCount).toBe(0);
    // On Windows a live child pins its cwd: no retry should be necessary once
    // shutdown has completed. A background UI refresh cannot start another child.
    await rm(fixture, { recursive: true });
    expect(() => processes.start(process.execPath, [], {})).toThrow(
      "shutting down",
    );
    await processes.close();
  } finally {
    await processes.close();
    await rm(fixture, { recursive: true, force: true });
  }
});

test("acceptance timeout and output overflow reject only after terminating the child", async () => {
  const processes = new AcceptanceProcesses();
  try {
    await expect(
      processes.execute(
        process.execPath,
        ["-e", "setInterval(() => {}, 1000)"],
        tmpdir(),
        process.env,
        "",
        250,
      ),
    ).rejects.toThrow("Acceptance command timed out");
    expect(processes.activeCount).toBe(0);
    await expect(
      processes.execute(
        process.execPath,
        ["-e", 'process.stdout.write("x".repeat(5 * 1024 * 1024))'],
        tmpdir(),
        process.env,
      ),
    ).rejects.toThrow("output exceeds limit");
    expect(processes.activeCount).toBe(0);
  } finally {
    await processes.close();
  }
});

test("acceptance spawn failure releases ownership without an unhandled error", async () => {
  const fixture = await mkdtemp(join(tmpdir(), "colossus-process-test-"));
  const processes = new AcceptanceProcesses();
  try {
    await expect(
      processes.execute(
        join(fixture, "not-an-executable"),
        [],
        fixture,
        process.env,
      ),
    ).rejects.toMatchObject({ code: "ENOENT" });
    expect(processes.activeCount).toBe(0);
  } finally {
    await processes.close();
    await rm(fixture, { recursive: true, force: true });
  }
});
