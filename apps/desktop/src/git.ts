export interface GitFile {
  path: string;
  previousPath: string | null;
  staged: string | null;
  unstaged: string | null;
  untracked: boolean;
  conflicted: boolean;
}

export type GitDiffSource = "staged" | "unstaged" | "untracked" | "commit";
export interface GitDiffSelection {
  repositoryId: string;
  source: GitDiffSource;
  commitId?: string;
}
export interface GitFileVersion {
  state:
    | "text"
    | "absent"
    | "binary"
    | "too_large"
    | "unsupported"
    | "unavailable"
    | "conflict";
  content: string | null;
  sizeBytes: number | null;
}
export interface GitDiffLine {
  kind: "added" | "removed" | "context";
  oldLine: number | null;
  newLine: number | null;
}
export interface GitDiffHunk {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: GitDiffLine[];
}
export interface GitFileDiff {
  path: string;
  previousPath: string | null;
  language: string;
  before: GitFileVersion;
  after: GitFileVersion;
  hunks: GitDiffHunk[];
  additions: number;
  deletions: number;
  truncated: boolean;
  note: string | null;
}
export interface GitRepository {
  id: string;
  name: string;
  branch: string | null;
  head: string | null;
  linkedWorktree: boolean;
  scoped: boolean;
  operation: string | null;
  files: GitFile[];
  truncated: boolean;
  notes: string[];
}
export interface GitStatus {
  state: "ready" | "not_repository" | "permission_required";
  repository: GitRepository | null;
}
export interface GitCommit {
  id: string;
  subject: string;
  author: string;
  timestamp: number;
}
export interface GitCommitPage {
  head: string | null;
  commits: GitCommit[];
  nextCursor: string | null;
  limited: boolean;
}
export interface GitCommitDetails {
  commit: GitCommit;
  message: string;
  parents: string[];
  files: { path: string; previousPath: string | null; status: string }[];
  truncated: boolean;
}
export function gitBranchLabel(repository: GitRepository): string {
  return (
    repository.branch ??
    `Detached · ${repository.head?.slice(0, 7) ?? "unknown"}`
  );
}
export function gitFileGroups(files: readonly GitFile[]) {
  return [
    { label: "Conflicts", files: files.filter((file) => file.conflicted) },
    {
      label: "Staged",
      files: files.filter((file) => !file.conflicted && file.staged !== null),
    },
    {
      label: "Unstaged",
      files: files.filter((file) => !file.conflicted && file.unstaged !== null),
    },
    {
      label: "Untracked",
      files: files.filter((file) => !file.conflicted && file.untracked),
    },
  ];
}
