import { beforeEach, describe, expect, it, vi } from "vitest";

const native = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => native);

describe("renderer operational logging", () => {
  beforeEach(() => {
    vi.resetModules();
    native.invoke.mockReset().mockResolvedValue(undefined);
    native.isTauri.mockReturnValue(true);
    vi.stubGlobal("window", { location: { search: "" } });
  });

  it("sends fixed categories, deduplicates bursts, and never sends error content", async () => {
    const { reportRendererFailure } = await import("./renderer-logging");
    reportRendererFailure("render");
    reportRendererFailure("render");
    reportRendererFailure("unhandled_promise");
    expect(native.invoke.mock.calls).toEqual([
      ["report_renderer_failure", { failure: "render" }],
      ["report_renderer_failure", { failure: "unhandled_promise" }],
    ]);
  });

  it("does not grant logging IPC to approval or terminal surfaces", async () => {
    vi.stubGlobal("window", {
      location: { search: "?surface=command-approval" },
    });
    const { reportRendererFailure } = await import("./renderer-logging");
    reportRendererFailure("script");
    expect(native.invoke).not.toHaveBeenCalled();
  });

  it("drops script and promise error payloads at the event boundary", async () => {
    const listeners = new Map<string, () => void>();
    vi.stubGlobal("window", {
      location: { search: "" },
      addEventListener: (name: string, listener: () => void) =>
        listeners.set(name, listener),
    });
    const { installRendererFailureHandlers } =
      await import("./renderer-logging");
    installRendererFailureHandlers();
    listeners.get("error")?.();
    listeners.get("unhandledrejection")?.();
    expect(native.invoke.mock.calls).toEqual([
      ["report_renderer_failure", { failure: "script" }],
      ["report_renderer_failure", { failure: "unhandled_promise" }],
    ]);
  });
});
