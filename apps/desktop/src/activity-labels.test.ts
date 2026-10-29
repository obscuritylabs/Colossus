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

  it.each([
    ["browser.snapshot", "browser"],
    ["mcp.call", "mcp"],
    ["image.view", "image"],
    ["filesystem.read", "read"],
    ["fs.read", "read"],
    ["patch.preview", "read"],
    ["repo.read_many", "read"],
    ["repo.file_summary", "read"],
    ["workspace.inspect", "read"],
    ["filesystem.write", "write"],
    ["fs.write", "write"],
    ["filesystem.replace", "write"],
    ["workspace.apply_patch", "write"],
    ["patch.apply", "write"],
    ["patch.reverse", "write"],
    ["filesystem.list", "list"],
    ["fs.list", "list"],
    ["repo.map", "list"],
    ["repo.map_structure", "list"],
    ["filesystem.search", "search"],
    ["fs.search", "search"],
    ["repo.search", "search"],
    ["repo.symbol_search", "search"],
    ["repo.references", "search"],
    ["web.search", "web"],
    ["web.fetch", "web"],
    ["docs.fetch", "web"],
    ["shell.run", "run"],
    ["shell.read", "run"],
    ["shell.wait", "run"],
    ["shell.list", "run"],
    ["shell.stop", "run"],
    ["process.spawn", "run"],
    ["process.start", "run"],
    ["process.poll", "run"],
    ["process.read", "run"],
    ["process.write", "run"],
    ["process.stop", "run"],
    ["process.list", "run"],
    ["agent.delegate", "delegate"],
    ["agent.result", "delegate"],
    ["agent.subagent_update", "delegate"],
    ["extension.review", "generic"],
    ["fs.read.extra", "generic"],
    ["shell.stop.extra", "generic"],
    ["process.unknown", "generic"],
    ["mcp.custom", "mcp"],
    ["browser.custom", "browser"],
  ])(
    "retains the existing %s icon family and canonical title",
    (tool, kind) => {
      expect(
        presentToolActivity(activity(tool, "Canonical released summary")),
      ).toEqual({
        kind,
        title: "Canonical released summary",
      });
    },
  );

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
