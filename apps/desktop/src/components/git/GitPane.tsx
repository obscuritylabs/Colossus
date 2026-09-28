import {
  IconGitBranch,
  IconLock,
  IconLoader2,
  IconRefresh,
  IconX,
} from "@tabler/icons-react";
import { useState, type Ref } from "react";
import { gitBranchLabel } from "../../git";
import { GitChanges } from "./GitChanges";
import { GitHistory } from "./GitHistory";
import type { GitController } from "./useGit";
import "./git.css";

export function GitIndicator({
  git,
  open,
  onClick,
  compact = false,
}: {
  git: GitController;
  open: boolean;
  onClick: () => void;
  compact?: boolean;
}) {
  const repository = git.status?.repository;
  const label = repository
    ? gitBranchLabel(repository)
    : git.status?.state === "not_repository"
      ? "No repository"
      : "Git";
  return (
    <button
      className="button secondary compact git-indicator"
      type="button"
      aria-label={`Open Git: ${label}${repository ? `, ${repository.files.length}${repository.truncated ? "+" : ""} changed files` : ""}${git.error ? ", refresh failed" : ""}`}
      aria-expanded={open}
      aria-controls="work-side-drawer"
      title={
        git.error ||
        (git.status?.state === "not_repository"
          ? "No Git repository in this folder"
          : "View Git changes and commit history")
      }
      onClick={onClick}
    >
      {git.showProgress ? (
        <IconLoader2 size={15} className="git-spinner" aria-hidden="true" />
      ) : (
        <IconGitBranch size={15} aria-hidden="true" />
      )}
      {!compact ? <span className="git-control-label">Git</span> : null}
      {label !== "Git" && (repository || !compact) ? (
        <span className="git-branch-label">{label}</span>
      ) : null}
      {repository && repository.files.length > 0 ? (
        <span
          className="git-change-count"
          title={`${repository.files.length}${repository.truncated ? "+" : ""} changed files`}
        >
          {repository.files.length}
          {repository.truncated ? "+" : ""}
        </span>
      ) : null}
      {git.error ? <span aria-hidden="true">!</span> : null}
    </button>
  );
}

export function GitPane({
  git,
  onClose,
  onOpenFile,
  onOpenDiff,
  closeRef,
}: {
  git: GitController;
  onClose: () => void;
  onOpenFile: (path: string) => void;
  onOpenDiff: (
    path: string,
    selection: import("../../git").GitDiffSelection,
  ) => void;
  closeRef: Ref<HTMLButtonElement>;
}) {
  const [tab, setTab] = useState<"changes" | "history">("changes");
  const repository = git.status?.repository;
  return (
    <section className="git-pane" aria-label="Workspace Git">
      <header className="git-pane-header">
        <div>
          <h2>
            <IconGitBranch size={19} aria-hidden="true" />
            Git
          </h2>
          <span className="git-repository-name" title={repository?.name}>
            {repository?.name ?? "Current workspace"}
          </span>
        </div>
        <div>
          <button
            type="button"
            className="icon-button"
            aria-label="Refresh Git"
            disabled={git.busy}
            onClick={() => void git.refresh()}
          >
            <IconRefresh
              size={18}
              className={git.showProgress ? "git-spinner" : undefined}
            />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Close Git panel"
            ref={closeRef}
            onClick={onClose}
          >
            <IconX size={18} />
          </button>
        </div>
      </header>
      {git.error ? (
        <div className="git-notice" role="alert">
          {git.error}
          {repository ? " Showing the last successful refresh." : ""}
        </div>
      ) : null}
      {!git.status && git.busy ? (
        <p className="git-empty" role="status">
          Reading repository…
        </p>
      ) : null}
      {git.status?.state === "not_repository" ? (
        <div className="git-empty">
          <IconGitBranch size={30} />
          <h3>No Git repository</h3>
          <p>
            This workspace is not inside a Git repository. If you initialize one
            in your terminal, refresh to see it here.
          </p>
        </div>
      ) : null}
      {git.status?.state === "permission_required" ? (
        <div className="git-empty">
          <h3>Connect repository metadata</h3>
          <p>
            This folder uses Git metadata outside the workspace. Allow read-only
            inspection to show its branch and history.
          </p>
          <button
            type="button"
            className="button secondary"
            disabled={git.busy}
            onClick={() => void git.refresh(true)}
          >
            Connect Git
          </button>
        </div>
      ) : null}
      {repository ? (
        <>
          <div className="git-branch-context">
            <IconGitBranch size={15} aria-hidden="true" />
            <strong title={gitBranchLabel(repository)}>
              {gitBranchLabel(repository)}
            </strong>
            {repository.linkedWorktree ? (
              <span className="git-worktree-label" title="Linked worktree">
                Worktree
              </span>
            ) : null}
          </div>
          {repository.head === null ? (
            <p className="git-context-note">No commits yet</p>
          ) : null}
          {repository.operation ? (
            <p className="git-notice git-operation">{repository.operation}</p>
          ) : null}
          {repository.scoped ? (
            <p className="git-context-note">
              Files in this workspace · History for the repository
            </p>
          ) : null}
          {repository.notes.length > 0 ? (
            <details className="git-repository-notes">
              <summary>Repository notes ({repository.notes.length})</summary>
              {repository.notes.map((note) => (
                <p key={note}>{note}</p>
              ))}
            </details>
          ) : null}
          <div className="git-tabs" role="tablist" aria-label="Git views">
            {(["changes", "history"] as const).map((value) => (
              <button
                key={value}
                id={`git-tab-${value}`}
                role="tab"
                type="button"
                aria-selected={tab === value}
                aria-controls="git-tab-panel"
                aria-label={
                  value === "changes"
                    ? `Changes (${repository.files.length}${repository.truncated ? "+" : ""})`
                    : "History"
                }
                tabIndex={tab === value ? 0 : -1}
                onClick={() => setTab(value)}
                onKeyDown={(event) => {
                  if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
                    event.preventDefault();
                    const next = value === "changes" ? "history" : "changes";
                    setTab(next);
                    document.getElementById(`git-tab-${next}`)?.focus();
                  }
                }}
              >
                {value === "changes" ? (
                  <>
                    Changes{" "}
                    <span className="git-tab-count">
                      {repository.files.length}
                      {repository.truncated ? "+" : ""}
                    </span>
                  </>
                ) : (
                  "History"
                )}
              </button>
            ))}
          </div>
          <div
            className="git-pane-body"
            id="git-tab-panel"
            role="tabpanel"
            aria-labelledby={`git-tab-${tab}`}
          >
            {tab === "changes" ? (
              <GitChanges
                key={repository.id}
                repository={repository}
                onOpenFile={onOpenFile}
                onOpenDiff={onOpenDiff}
              />
            ) : (
              <GitHistory
                key={`${repository.id}:${repository.head}`}
                repository={repository}
                git={git}
                onOpenDiff={onOpenDiff}
              />
            )}
          </div>
        </>
      ) : null}
      <footer className="git-pane-footer">
        <span>
          <IconLock size={12} aria-hidden="true" />
          Read-only
        </span>
        <span role="status">
          {git.showProgress ? (
            <>
              <IconLoader2
                size={12}
                className="git-spinner"
                aria-hidden="true"
              />
              Refreshing…
            </>
          ) : git.error ? (
            "Refresh failed"
          ) : (
            "Auto-refresh on"
          )}
        </span>
      </footer>
    </section>
  );
}
