import assert from "node:assert/strict";
import { test } from "node:test";
import { toolActivityLabel } from "../src/lib/tool-activity.ts";

test("all hosts preserve the same released tool label without a builtin-name map", () => {
  for (const [name, summary] of [
    ["repo.map", "Mapped repository structure"],
    ["mcp.call", "Called GitLab · list_issues"],
    ["extension.review", "Reviewed deployment readiness"],
  ]) {
    assert.equal(toolActivityLabel(summary, name), summary);
  }
});

test("legacy placeholders and missing summaries use only a generic identity fallback", () => {
  for (const summary of [
    undefined,
    "",
    "validated tool call requested",
    "tool execution started at turn 1",
    "tool execution completed",
    "Tool completed",
    "tool execution was cancelled before start at turn 2",
  ]) {
    assert.equal(
      toolActivityLabel(summary, "repo.map"),
      "Tool activity · repo.map",
    );
  }
  assert.equal(toolActivityLabel(null, ""), "Tool activity");
});

test("labels are single-line plain text with bounded Unicode characters", () => {
  assert.equal(
    toolActivityLabel("  Mapped\nrepository\u202e structure\t", "repo.map"),
    "Mapped repository structure",
  );
  const label = toolActivityLabel("界".repeat(321), "repo.map");
  assert.equal(Array.from(label).length, 320);
  assert.ok(label.endsWith("…"));
  assert.equal(Array.from(toolActivityLabel("", "界".repeat(400))).length, 320);
  assert.equal(
    toolActivityLabel("<img src=x onerror=alert(1)>", "repo.map"),
    "<img src=x onerror=alert(1)>",
  );
});

test("labels do not infer unreleased target names from tool identity", () => {
  assert.equal(
    toolActivityLabel("Preparing to call an MCP tool", "mcp.call"),
    "Preparing to call an MCP tool",
  );
  assert.equal(toolActivityLabel("", "mcp.call"), "Tool activity · mcp.call");
});
