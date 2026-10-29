import { describe, expect, it } from "vitest";

import type { ToolActivity } from "./types";
import { presentNotice, presentToolActivity } from "./activity-labels";

function activity(
  toolName: string,
  summary: string,
  state: ToolActivity["state"] = "completed",
): ToolActivity {
  return { callId: "call-1", toolName, state, summary };
}

describe("activity labels", () => {
  it.each([
    ["repo.map", "Mapped repository structure"],
    ["filesystem.write", "Created sample.txt"],
    ["shell.run", "Running git status --short"],
    ["mcp.call", "Called GitLab · list_issues"],
    ["extension.review", "Reviewed deployment readiness"],
  ])("preserves the canonical summary for %s", (tool, summary) => {
    expect(presentToolActivity(activity(tool, summary)).title).toBe(summary);
  });

  it("retains namespace icon styling without generating another semantic label", () => {
    expect(
      presentToolActivity(activity("browser.snapshot", "Read the page")).kind,
    ).toBe("browser");
    expect(
      presentToolActivity(activity("mcp.call", "Called GitLab")).kind,
    ).toBe("mcp");
    expect(
      presentToolActivity(activity("repo.map", "Mapped repository structure"))
        .kind,
    ).toBe("generic");
  });

  it("uses generic identity for legacy placeholders and does not infer targets from input", () => {
    expect(
      presentToolActivity({
        ...activity("filesystem.read", "Reading workspace files", "started"),
        input: '{"path":"do-not-derive-a-title.txt"}',
      }).title,
    ).toBe("Reading workspace files");
    expect(
      presentToolActivity(
        activity("repo.map", "tool execution completed at turn 1"),
      ).title,
    ).toBe("Tool activity · repo.map");
    expect(presentToolActivity(activity("mcp.call", "")).title).toBe(
      "Tool activity · mcp.call",
    );
  });

  it("translates research event identifiers and awkward details", () => {
    expect(
      presentNotice(
        "research.planning.started",
        "Planning bounded research queries.",
      ),
    ).toEqual({
      title: "Planning research",
      detail: "Preparing focused, bounded research queries.",
      kind: "research",
    });
    expect(
      presentNotice(
        "research.collecting.completed",
        "released 20 repository source(s)",
      ).detail,
    ).toBe("Added 20 repository sources.");
    expect(
      presentNotice(
        "research.collecting.skipped",
        "bounded research worker or source budget exhausted",
      ).detail,
    ).toBe("Source or worker limit reached.");
  });
});
