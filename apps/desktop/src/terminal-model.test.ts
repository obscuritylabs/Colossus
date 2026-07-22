import { describe, expect, it } from "vitest";

import {
  decodeTerminalOutput,
  terminalContextChanged,
  terminalDimensions,
  terminalInputChunks,
  terminalLaunchRequested,
} from "./terminal-model";

function decodeChunks(chunks: readonly string[]): Uint8Array {
  const decoded = chunks.flatMap((chunk) =>
    Array.from(decodeTerminalOutput(chunk)),
  );
  return Uint8Array.from(decoded);
}

describe("terminal renderer bounds", () => {
  it("invalidates terminal tabs when native workspace authority changes", () => {
    const context = {
      enabled: true,
      contextGeneration: 4,
      launchRequestId: 7,
      workspaceId: "workspace-1",
      workspaceName: "Colossus",
      requestedKind: null,
    };

    expect(terminalContextChanged(null, context)).toBe(true);
    expect(terminalContextChanged(context, context)).toBe(false);
    expect(
      terminalContextChanged(context, {
        ...context,
        contextGeneration: 5,
      }),
    ).toBe(true);
    expect(
      terminalContextChanged(context, {
        ...context,
        enabled: false,
        workspaceId: null,
      }),
    ).toBe(true);
  });

  it("opens the requested terminal kind without invalidating existing tabs", () => {
    const context = {
      enabled: true,
      contextGeneration: 4,
      launchRequestId: 7,
      workspaceId: "workspace-1",
      workspaceName: "Colossus",
      requestedKind: null,
    };

    expect(terminalLaunchRequested(null, context)).toBe(false);
    expect(terminalLaunchRequested(context, context)).toBe(false);
    expect(
      terminalLaunchRequested(context, {
        ...context,
        launchRequestId: 8,
        requestedKind: "colossus_tui" as const,
      }),
    ).toBe(true);
    expect(
      terminalContextChanged(context, {
        ...context,
        launchRequestId: 8,
        requestedKind: "colossus_tui" as const,
      }),
    ).toBe(false);
    expect(
      terminalLaunchRequested(context, {
        ...context,
        requestedKind: "colossus_tui" as const,
      }),
    ).toBe(true);
    expect(
      terminalLaunchRequested(null, {
        ...context,
        requestedKind: "colossus_tui" as const,
      }),
    ).toBe(true);
  });

  it("chunks terminal input below the native per-request boundary", () => {
    const input = `${"x".repeat(48 * 1024 - 1)}🙂done`;
    const chunks = terminalInputChunks(input);

    expect(chunks).toHaveLength(2);
    expect(decodeChunks(chunks)).toEqual(new TextEncoder().encode(input));
    for (const chunk of chunks) {
      expect(decodeTerminalOutput(chunk).byteLength).toBeLessThanOrEqual(
        48 * 1024,
      );
    }
  });

  it("does not issue an empty native write for empty terminal input", () => {
    expect(terminalInputChunks("")).toEqual([]);
  });

  it("rejects an oversized input event before queuing native writes", () => {
    expect(() => terminalInputChunks("x".repeat(256 * 1024 + 1))).toThrow(
      "Terminal input event exceeds the renderer limit.",
    );
  });

  it("rejects an oversized terminal output event before decoding it", () => {
    const oversized = "A".repeat(Math.ceil((64 * 1024) / 3) * 4 + 1);

    expect(() => decodeTerminalOutput(oversized)).toThrow(
      "Terminal output event exceeds the renderer limit.",
    );
  });

  it("clamps PTY dimensions to the native contract", () => {
    expect(terminalDimensions(0, 0)).toEqual({ cols: 2, rows: 2 });
    expect(terminalDimensions(8_400, 17_200)).toEqual({
      cols: 512,
      rows: 512,
    });
    expect(terminalDimensions(840, 344)).toEqual({ cols: 100, rows: 20 });
  });
});
