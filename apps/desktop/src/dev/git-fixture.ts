import type {
  GitCommitDetails,
  GitCommitPage,
  GitFile,
  GitStatus,
} from "../git";

const id = "3fee310e70fccd9448643e5b37a74325442b8d75";
const commit = {
  id,
  subject: "Improve desktop setup and workspace settings",
  author: "Colossus contributor",
  timestamp: 1790472000,
};
const commits = [
  commit,
  {
    ...commit,
    id: "fe12d3ba9f5dc7edc00207abcbd4d8b43d6dcd8f",
    subject: "Add browser navigation and tab controls",
    author: "Desktop team",
    timestamp: commit.timestamp - 3600,
  },
  {
    ...commit,
    id: "ab45ef6712345678901234567890123456789012ab",
    subject: "Keep the message composer responsive",
    timestamp: commit.timestamp - 86400,
  },
];
const files: [GitFile, GitFile, GitFile] = [
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
// Model the native reader's single-operation gate in browser acceptance tests.
let statusInFlight = false;
export const gitFixture = {
  async status(workspaceId: string): Promise<GitStatus> {
    if (statusInFlight) throw new Error("Git is still refreshing.");
    statusInFlight = true;
    try {
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
          files:
            scenario === "clean"
              ? []
              : scenario === "many"
                ? [
                    ...files,
                    {
                      ...files[2],
                      path: "src/components/conflict.tsx",
                      conflicted: true,
                    },
                    {
                      ...files[2],
                      path: "src/removed.ts",
                      unstaged: "deleted",
                    },
                    {
                      ...files[2],
                      path: "src/new-name.ts",
                      previousPath: "src/old-name.ts",
                      staged: "renamed",
                      unstaged: null,
                    },
                    ...Array.from({ length: 60 }, (_, i) => ({
                      ...files[2],
                      path: `src/features/feature-${i}/index.ts`,
                    })),
                  ]
                : files,
          truncated: false,
          notes: [],
        },
      };
    } finally {
      statusInFlight = false;
    }
  },
  async history(cursor: string | null): Promise<GitCommitPage> {
    const expired =
      new URLSearchParams(window.location.search).get("git") === "expired";
    if (expired && cursor)
      throw new Error("This history page expired. Reload history.");
    return {
      head: id,
      commits,
      nextCursor: expired ? "expired-cursor" : null,
      limited: false,
    };
  },
  async details(commitId: string): Promise<GitCommitDetails> {
    const selected = commits.find((entry) => entry.id === commitId) ?? commit;
    return {
      commit: { ...selected, id: commitId },
      message: `${selected.subject}\n\nKeep workspace changes clear and preserve the current conversation.`,
      parents: ["fe12d3ba9f5dc7edc00207abcbd4d8b43d6dcd8f"],
      files: [
        {
          path: "src/components/WorkSurface.tsx",
          previousPath: null,
          status: "modified",
        },
        {
          path: "src/components/git/GitPane.tsx",
          previousPath: "src/components/GitPane.tsx",
          status: "renamed",
        },
      ],
      truncated: false,
    };
  },
};
