import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ApiFailure,
  awaitReceipt,
  projectPath,
  request,
  taskStatus,
  terminalStatuses,
  visibleOutput,
  type Task,
} from "./api";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
describe("browser authority and released data", () => {
  it("reconciles identity after a project denial without creating an authentication loop", async () => {
    const dispatchEvent = vi.fn();
    vi.stubGlobal("window", { dispatchEvent });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() =>
        Promise.resolve(
          new Response(JSON.stringify({ error: "permission_denied" }), {
            status: 403,
          }),
        ),
      ),
    );
    await expect(request("/api/projects/p/tasks")).rejects.toBeInstanceOf(
      ApiFailure,
    );
    expect(dispatchEvent).toHaveBeenCalledOnce();
    expect(dispatchEvent.mock.calls[0]?.[0].type).toBe(
      "colossus:web:reconcile-identity",
    );
    await expect(request("/api/me")).rejects.toBeInstanceOf(ApiFailure);
    expect(dispatchEvent).toHaveBeenCalledOnce();
  });
  it("reports a runtime rejection even when HTTP accepted the queued action", async () => {
    await expect(
      awaitReceipt("/api/projects/p/tasks/t", {
        command_id: "c",
        reply: {
          kind: "failed",
          error: { code: "conflict", message: "The interaction changed." },
        },
      }),
    ).rejects.toThrow("The interaction changed.");
  });
  it("waits for the exact receipt without submitting a second mutation", async () => {
    vi.useFakeTimers();
    const fetch = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          command: { command_id: "c", reply: { kind: "responded" } },
        }),
      ),
    );
    vi.stubGlobal("fetch", fetch);
    const settled = awaitReceipt("/api/projects/p/tasks/t", {
      command_id: "c",
      reply: null,
    });
    await vi.advanceTimersByTimeAsync(300);
    expect(await settled).toBe(true);
    expect(fetch.mock.calls).toHaveLength(1);
    expect(fetch.mock.calls[0]?.[0]).toBe("/api/projects/p/tasks/t/commands/c");
    expect(fetch.mock.calls[0]?.[1].method).toBe("GET");
  });
  it("uses same-origin cookies and explicit CSRF protection for mutations", async () => {
    const fetch = vi
      .fn()
      .mockResolvedValue(new Response("{}", { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    await request("/api/projects/engineering/tasks", { node_id: "node-a" });
    expect(fetch).toHaveBeenCalledWith("/api/projects/engineering/tasks", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-Colossus-CSRF": "1" },
      body: '{"node_id":"node-a"}',
    });
    expect(JSON.stringify(fetch.mock.calls)).not.toContain("Authorization");
  });
  it("preserves abort signals and treats permission failures as failures", async () => {
    const abort = new AbortController();
    const fetch = vi
      .fn()
      .mockResolvedValue(
        new Response('{"error":"permission_denied"}', { status: 403 }),
      );
    vi.stubGlobal("fetch", fetch);
    await expect(
      request("/api/me", undefined, abort.signal),
    ).rejects.toBeInstanceOf(ApiFailure);
    expect(fetch.mock.calls[0]?.[1].signal).toBe(abort.signal);
    expect(fetch.mock.calls[0]?.[1].method).toBe("GET");
  });
  it("never interpolates a project into a foreign URL or extra path", () => {
    expect(projectPath("../foreign?x=1")).toBe(
      "/api/projects/..%2Fforeign%3Fx%3D1",
    );
  });
  it("keeps an unknown allocation explicit instead of suggesting a rerun", () => {
    const task = {
      dispatch_error: { code: "outcome_unknown", message: "reconcile" },
      snapshot: null,
    } as Task;
    expect(taskStatus(task)).toBe("outcome_unknown");
    expect(terminalStatuses.has(taskStatus(task))).toBe(true);
  });
  it("renders only released output and replaces deltas with the authoritative result", () => {
    const events = [
      { update: { output_delta: "hel" } },
      { update: { provider_private: "do not display", output_delta: "lo" } },
      { update: { result: { output: "hello, completed" } } },
    ].map((event, index) => ({
      ...event,
      run_id: "run-a",
      sequence: index + 1,
      created_at: "2026-10-04T00:00:00Z",
    }));
    expect(visibleOutput(events)).toBe("hello, completed");
  });
});
