import type { GitDiffSelection, GitFileDiff } from "../git";

export async function readFixtureDiff(
  _workspaceId: string,
  path: string,
  selection: GitDiffSelection,
): Promise<GitFileDiff> {
  const scenario = new URLSearchParams(window.location.search).get("diff");
  if (scenario === "slow")
    await new Promise((resolve) => setTimeout(resolve, 1200));
  if (scenario === "error")
    throw new Error("Git changed during comparison. Refresh the diff.");
  const before =
    Array.from({ length: 24 }, (_, i) =>
      i === 2
        ? "const title = 'Before';"
        : i === 20
          ? "return false;"
          : `// Context line ${i + 1}`,
    ).join("\n") + "\n";
  const after = before
    .replace(
      "'Before'",
      selection.source === "staged" ? "'Staged'" : "'Working'",
    )
    .replace("return false", "return true");
  const result: GitFileDiff = {
    path,
    previousPath: null,
    language: "typescript",
    before: { state: "text", content: before, sizeBytes: before.length },
    after: { state: "text", content: after, sizeBytes: after.length },
    additions: 2,
    deletions: 2,
    truncated: false,
    note:
      selection.source === "staged"
        ? "HEAD → index. Only staged changes are shown."
        : selection.source === "commit"
          ? "Compared with the first parent."
          : "Index → working file. Raw text is compared; external Git filters are not run.",
    hunks: [3, 21].map((line) => ({
      oldStart: line - 1,
      oldLines: 3,
      newStart: line - 1,
      newLines: 3,
      lines: [
        { kind: "context", oldLine: line - 1, newLine: line - 1 },
        { kind: "removed", oldLine: line, newLine: null },
        { kind: "added", oldLine: null, newLine: line },
        { kind: "context", oldLine: line + 1, newLine: line + 1 },
      ],
    })),
  };
  if (scenario === "binary" || scenario === "large") {
    result.after = {
      state: scenario === "binary" ? "binary" : "too_large",
      content: null,
      sizeBytes: null,
    };
    result.hunks = [];
    result.additions = 0;
    result.deletions = 0;
  }
  if (scenario === "empty") {
    result.before = { state: "absent", content: null, sizeBytes: null };
    result.after = { state: "text", content: "", sizeBytes: 0 };
    result.hunks = [];
    result.additions = 0;
    result.deletions = 0;
  }
  if (scenario === "deleted") {
    result.after = { state: "absent", content: null, sizeBytes: null };
    result.hunks = [
      {
        oldStart: 1,
        oldLines: 24,
        newStart: 0,
        newLines: 0,
        lines: Array.from({ length: 24 }, (_, i) => ({
          kind: "removed",
          oldLine: i + 1,
          newLine: null,
        })),
      },
    ];
    result.additions = 0;
    result.deletions = 24;
  }
  if (scenario === "limited") result.truncated = true;
  return result;
}
