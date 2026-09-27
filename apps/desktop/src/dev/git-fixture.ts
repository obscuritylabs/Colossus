import type { GitCommitDetails, GitCommitPage, GitStatus } from "../git";

const id = "3fee310e70fccd9448643e5b37a74325442b8d75";
const commit = {
  id,
  subject: "Improve desktop setup and workspace settings",
  author: "Colossus contributor",
  timestamp: 1790472000,
};
const files = [
  {
    path: "src/components/WorkSurface.tsx",
    previousPath: null,
    staged: "modified",
    unstaged: "modified",
    untracked: false,
    conflicted: false,
  },
  {
    path: "src/components/git/GitPane.tsx",
    previousPath: null,
    staged: null,
    unstaged: null,
    untracked: true,
    conflicted: false,
  },
  {
    path: "README.md",
    previousPath: null,
    staged: null,
    unstaged: "modified",
    untracked: false,
    conflicted: false,
  },
];
export const gitFixture = {
  async status(workspaceId: string): Promise<GitStatus> {
    const scenario = new URLSearchParams(window.location.search).get("git");
    const research = workspaceId === "fixture-research";
    if (scenario === "slow" || (scenario === "switch" && !research))
      await new Promise((resolve) => window.setTimeout(resolve, 1200));
    if (scenario === "error")
      throw new Error("Git metadata is not readable. Refresh to try again.");
    if (scenario === "none")
      return { state: "not_repository", repository: null };
    return {
      state: "ready",
      repository: {
        id: `fixture-repository-${workspaceId}`,
        name: research ? "Research Lab" : "Colossus",
        branch:
          scenario === "detached"
            ? null
            : research
              ? "research"
              : "codex/desktop-git",
        head: id,
        linkedWorktree: true,
        scoped: false,
        operation: null,
        files: scenario === "clean" ? [] : files,
        truncated: false,
        notes: [],
      },
    };
  },
  async history(cursor: string | null): Promise<GitCommitPage> {
    const expired =
      new URLSearchParams(window.location.search).get("git") === "expired";
    if (expired && cursor)
      throw new Error("This history page expired. Reload history.");
    return {
      head: id,
      commits: [commit],
      nextCursor: expired ? "expired-cursor" : null,
      limited: false,
    };
  },
  async details(commitId: string): Promise<GitCommitDetails> {
    return {
      commit: { ...commit, id: commitId },
      message: `${commit.subject}\n\nKeep workspace changes clear and preserve the current conversation.`,
      parents: ["fe12d3ba9f5dc7edc00207abcbd4d8b43d6dcd8f"],
      files: [
        {
          path: "src/components/WorkSurface.tsx",
          previousPath: null,
          status: "modified",
        },
      ],
      truncated: false,
    };
  },
};
