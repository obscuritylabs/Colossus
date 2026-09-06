import {
  spawn,
  type ChildProcess,
  type SpawnOptions,
} from "node:child_process";

/** Own every test subprocess until its streams close, including failed commands. */
export class AcceptanceProcesses {
  private readonly children = new Map<ChildProcess, Promise<void>>();
  private closing = false;

  get activeCount(): number {
    return this.children.size;
  }

  start(binary: string, args: string[], options: SpawnOptions): ChildProcess {
    if (this.closing) throw new Error("Acceptance processes are shutting down");
    const child = spawn(binary, args, options);
    const closed = new Promise<void>((resolve) => {
      // A failed spawn also emits close. Execute reports the error; background
      // workers are checked by the readiness assertion without an unhandled event.
      child.on("error", () => {});
      child.once("close", () => {
        this.children.delete(child);
        resolve();
      });
    });
    this.children.set(child, closed);
    return child;
  }

  execute(
    binary: string,
    args: string[],
    cwd: string,
    env: NodeJS.ProcessEnv,
    input = "",
    timeoutMs = 45_000,
  ): Promise<string> {
    const child = this.start(binary, args, { cwd, env, stdio: "pipe" });
    return new Promise((resolve, reject) => {
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      let stdoutSize = 0;
      let stderrSize = 0;
      let failure: Error | undefined;
      const fail = (error: Error) => {
        failure ??= error;
        child.kill("SIGKILL");
      };
      const timeout = setTimeout(
        () => fail(new Error("Acceptance command timed out")),
        timeoutMs,
      );
      child.on("error", (error) => {
        failure ??= error;
      });
      child.stdin?.on("error", fail);
      child.stdout?.on("data", (bytes: Buffer) => {
        stdoutSize += bytes.length;
        if (stdoutSize > 4 * 1024 * 1024)
          fail(new Error("Acceptance command output exceeds limit"));
        else stdout.push(bytes);
      });
      child.stderr?.on("data", (bytes: Buffer) => {
        const bounded = bytes.subarray(0, Math.max(0, 64 * 1024 - stderrSize));
        if (bounded.length > 0) stderr.push(bounded);
        stderrSize += bounded.length;
      });
      // exit can precede the final output and Windows handle release. Never let
      // command completion or a timeout race deletion of the process's cwd.
      child.once("close", (code, signal) => {
        clearTimeout(timeout);
        if (failure) reject(failure);
        else if (code === 0) resolve(Buffer.concat(stdout).toString());
        else
          reject(
            new Error(
              `Acceptance command exited ${code ?? signal}: ${Buffer.concat(stderr).toString()}`,
            ),
          );
      });
      child.stdin?.end(input);
    });
  }

  async close(): Promise<void> {
    this.closing = true;
    const owned = [...this.children];
    if (owned.length === 0) return;
    for (const [child] of owned) child.kill();
    const force = setTimeout(() => {
      for (const [child] of owned)
        if (this.children.has(child)) child.kill("SIGKILL");
    }, 1_000);
    let deadline: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        Promise.all(owned.map(([, closed]) => closed)),
        new Promise<never>((_, reject) => {
          deadline = setTimeout(
            () =>
              reject(
                new Error(
                  "Acceptance subprocesses did not close; retaining fixture",
                ),
              ),
            10_000,
          );
        }),
      ]);
    } finally {
      clearTimeout(force);
      clearTimeout(deadline);
    }
  }
}
