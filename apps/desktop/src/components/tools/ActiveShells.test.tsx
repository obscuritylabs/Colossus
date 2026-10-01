import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ActiveShells } from "./ActiveShells";
import {
  appendShellLog,
  shellIsActive,
  type ShellSession,
} from "../../shellSessions";

describe("managed shell output", () => {
  it("bounds a long log while preserving the newest released output", () => {
    const result = appendShellLog("old".repeat(30000), [
      { sequence: 19, stdout: "latest stdout", stderr: "latest stderr" },
    ]);
    expect(result.truncated).toBe(true);
    expect(result.text.length).toBe(65536);
    expect(result.text.endsWith("latest stdoutlatest stderr")).toBe(true);
  });
  it("keeps stopping active until termination is confirmed", () => {
    expect(shellIsActive("stopping")).toBe(true);
    for (const status of [
      "stopped",
      "timed_out",
      "outcome_unknown",
      "interrupted",
    ] as const)
      expect(shellIsActive(status)).toBe(false);
  });
  it("renders untrusted command text without creating markup or executable links", () => {
    const session: ShellSession = {
      id: "shell-1",
      session_id: "chat-1",
      run_id: "run-1",
      lifetime: "workspace",
      status: "running",
      command: '<script>alert(1)</script><a href="javascript:evil">x</a>',
      cwd: "/workspace",
      created_at_ms: 1,
      deadline_ms: 200,
      exit_code: null,
      reason: null,
      truncated: false,
      output_sequence: 0,
    };
    const markup = renderToStaticMarkup(
      createElement(ActiveShells, {
        scope: "target",
        sessions: [session],
        error: null,
        refresh: vi.fn(),
      }),
    );
    expect(markup).toContain("Active shells · 1");
    expect(markup).toContain("&lt;script&gt;");
    expect(markup).not.toContain("<script>");
    expect(markup).not.toContain("<a href=");
  });
});
