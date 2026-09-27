import { describe, expect, it } from "vitest";
import { gitBranchLabel, gitFileGroups } from "./git";
import type { GitFile, GitRepository } from "./git";

describe("Git presentation", () => {
  it("shows mixed changes in both groups but isolates conflicts", () => {
    const mixed: GitFile = {
      path: "mixed",
      previousPath: null,
      staged: "modified",
      unstaged: "modified",
      untracked: false,
      conflicted: false,
    };
    const groups = gitFileGroups([
      mixed,
      { ...mixed, path: "conflicted", conflicted: true },
      { ...mixed, path: "new", staged: null, unstaged: null, untracked: true },
    ]);
    expect(
      groups.map((group) => [
        group.label,
        group.files.map((file) => file.path),
      ]),
    ).toEqual([
      ["Conflicts", ["conflicted"]],
      ["Staged", ["mixed"]],
      ["Unstaged", ["mixed"]],
      ["Untracked", ["new"]],
    ]);
  });
  it("distinguishes an unborn branch from detached HEAD", () => {
    const repository: GitRepository = {
      id: "repo",
      name: "Project",
      branch: "main",
      head: null,
      files: [],
      operation: null,
      scoped: false,
      linkedWorktree: false,
      truncated: false,
      notes: [],
    };
    expect(gitBranchLabel(repository)).toBe("main");
    expect(
      gitBranchLabel({ ...repository, branch: null, head: "123456789abcdef" }),
    ).toBe("Detached · 1234567");
  });
});
