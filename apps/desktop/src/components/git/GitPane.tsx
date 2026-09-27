import {
  IconGitBranch,
  IconGitCommit,
  IconLoader2,
  IconRefresh,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type {
  GitCommit,
  GitCommitDetails,
  GitFile,
  GitRepository,
} from "../../git";
import { gitBranchLabel, gitFileGroups } from "../../git";
import type { GitController } from "./useGit";
import "./git.css";

export function GitIndicator({
  git,
  open,
  onClick,
}: {
  git: GitController;
  open: boolean;
  onClick: () => void;
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
      title={git.error || label}
      onClick={onClick}
    >
      {git.busy ? (
        <IconLoader2 size={15} className="git-spinner" aria-hidden="true" />
      ) : (
        <IconGitBranch size={15} aria-hidden="true" />
      )}
      <span className="git-branch-label">{label}</span>
      {repository && repository.files.length > 0 ? (
        <span className="git-change-count">
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
}: {
  git: GitController;
  onClose: () => void;
  onOpenFile: (path: string) => void;
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
          <p>{repository?.name ?? "Current workspace"}</p>
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
              className={git.busy ? "git-spinner" : undefined}
            />
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Close Git panel"
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
          <div className="git-repository-summary">
            {repository.notes.map((note) => (
              <p key={note} className="git-notice">
                {note}
              </p>
            ))}
            <strong>
              <IconGitBranch size={16} />
              {gitBranchLabel(repository)}
            </strong>
            <p>
              {repository.linkedWorktree ? "Linked worktree" : "Repository"}
              {repository.head === null ? " · No commits yet" : ""}
            </p>
            {repository.operation ? (
              <p className="git-notice">{repository.operation}</p>
            ) : null}
            {repository.scoped ? (
              <p>
                File paths are limited to this workspace. History describes the
                repository.
              </p>
            ) : null}
          </div>
          <div className="git-tabs" role="tablist" aria-label="Git views">
            {(["changes", "history"] as const).map((value) => (
              <button
                key={value}
                id={`git-tab-${value}`}
                role="tab"
                type="button"
                aria-selected={tab === value}
                aria-controls="git-tab-panel"
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
                {value === "changes"
                  ? `Changes (${repository.files.length}${repository.truncated ? "+" : ""})`
                  : "History"}
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
              <Changes repository={repository} onOpenFile={onOpenFile} />
            ) : (
              <History
                key={`${repository.id}:${repository.head}`}
                repository={repository}
                git={git}
              />
            )}
          </div>
        </>
      ) : null}
      <footer className="git-pane-footer">
        Read-only · {git.busy ? "Refreshing…" : "Refreshes while you work"}
      </footer>
    </section>
  );
}

function Changes({
  repository,
  onOpenFile,
}: {
  repository: GitRepository;
  onOpenFile: (path: string) => void;
}) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const selected = repository.files.find((file) => file.path === selectedPath);
  return (
    <>
      {repository.truncated ? (
        <p className="git-notice">
          Showing a limited file list. Counts reflect the displayed files.
        </p>
      ) : null}
      {repository.files.length === 0 ? (
        <div className="git-empty">
          <h3>
            {repository.truncated
              ? "No displayable changes"
              : "Working tree clean"}
          </h3>
          <p>
            {repository.truncated
              ? "Some paths could not be displayed."
              : "There are no changes in this workspace."}
          </p>
        </div>
      ) : null}
      {gitFileGroups(repository.files)
        .filter((group) => group.files.length > 0)
        .map((group) => (
          <section
            className="git-file-group"
            key={group.label}
            aria-label={group.label}
          >
            <h3>
              {group.label}
              <span>{group.files.length}</span>
            </h3>
            {group.files.map((file) => (
              <button
                type="button"
                className="git-file-row"
                key={file.path}
                aria-pressed={file.path === selectedPath}
                onClick={() => setSelectedPath(file.path)}
              >
                <span>{file.path}</span>
                <small>
                  {group.label === "Staged"
                    ? file.staged
                    : group.label === "Unstaged"
                      ? file.unstaged
                      : group.label === "Conflicts"
                        ? "conflict"
                        : "new"}
                </small>
              </button>
            ))}
          </section>
        ))}
      {selected ? (
        <FileDetails file={selected} onOpenFile={onOpenFile} />
      ) : null}
    </>
  );
}
function FileDetails({
  file,
  onOpenFile,
}: {
  file: GitFile;
  onOpenFile: (path: string) => void;
}) {
  return (
    <section className="git-file-details" aria-label="Changed file details">
      <h3>{file.path}</h3>
      {file.previousPath ? <p>Renamed from {file.previousPath}</p> : null}
      <dl>
        <dt>Staged</dt>
        <dd>{file.staged ?? "No"}</dd>
        <dt>Unstaged</dt>
        <dd>{file.untracked ? "Untracked" : (file.unstaged ?? "No")}</dd>
        {file.conflicted ? (
          <>
            <dt>Conflict</dt>
            <dd>Needs resolution</dd>
          </>
        ) : null}
      </dl>
      {file.staged !== "deleted" && file.unstaged !== "deleted" ? (
        <button
          type="button"
          className="button secondary compact"
          onClick={() => onOpenFile(file.path)}
        >
          Open current file
        </button>
      ) : (
        <p>This file has been deleted.</p>
      )}
    </section>
  );
}

function History({
  repository,
  git,
}: {
  repository: GitRepository;
  git: GitController;
}) {
  const [commits, setCommits] = useState<GitCommit[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [limited, setLimited] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [details, setDetails] = useState<GitCommitDetails | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const request = useRef(0);
  useEffect(() => {
    let current = true;
    void git.client
      .history(repository.id, null)
      .then((page) => {
        if (current) {
          setCommits(page.commits);
          setCursor(page.nextCursor);
          setLimited(page.limited);
        }
      })
      .catch((error: unknown) => {
        if (current) setError(errorText(error));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
      request.current += 1;
    };
  }, [git.client, repository.id]);
  async function loadMore(pageCursor: string | null = cursor) {
    const version = ++request.current;
    setBusy(true);
    setError("");
    try {
      const page = await git.client.history(repository.id, pageCursor);
      if (request.current === version) {
        setCommits((old) =>
          pageCursor === null ? page.commits : [...old, ...page.commits],
        );
        if (pageCursor === null) {
          setSelected(null);
          setDetails(null);
        }
        setCursor(page.nextCursor);
        setLimited(page.limited);
      }
    } catch (error) {
      if (request.current === version) setError(errorText(error));
    } finally {
      if (request.current === version) setBusy(false);
    }
  }
  async function inspect(commitId: string) {
    const version = ++request.current;
    setSelected(commitId);
    setDetails(null);
    setBusy(true);
    setError("");
    try {
      const result = await git.client.details(repository.id, commitId);
      if (request.current === version) setDetails(result);
    } catch (error) {
      if (request.current === version) setError(errorText(error));
    } finally {
      if (request.current === version) setBusy(false);
    }
  }
  return (
    <>
      {error ? (
        <p className="git-notice" role="alert">
          {error}
          <button
            type="button"
            className="button secondary compact"
            disabled={busy}
            onClick={() => void loadMore(null)}
          >
            Retry history
          </button>
        </p>
      ) : null}
      {busy ? (
        <p className="git-progress" role="status">
          <IconLoader2 className="git-spinner" size={16} />
          Reading history…
        </p>
      ) : null}
      {!busy && !error && commits.length === 0 ? (
        <p className="git-empty">No commits yet.</p>
      ) : null}
      <div className="git-commits">
        {commits.map((commit) => (
          <button
            className="git-commit-row"
            disabled={busy}
            key={commit.id}
            type="button"
            aria-pressed={selected === commit.id}
            onClick={() => void inspect(commit.id)}
          >
            <IconGitCommit size={18} aria-hidden="true" />
            <span>
              <strong>{commit.subject}</strong>
              <small>
                {commit.author} ·{" "}
                {new Date(commit.timestamp * 1000).toLocaleDateString()} ·{" "}
                {commit.id.slice(0, 7)}
              </small>
            </span>
          </button>
        ))}
      </div>
      {cursor ? (
        <button
          className="button secondary compact"
          type="button"
          disabled={busy}
          onClick={() => void loadMore()}
        >
          Load older commits
        </button>
      ) : null}
      {limited ? (
        <p className="git-notice">Showing the most recent 400 commits.</p>
      ) : null}
      {details ? (
        <section className="git-file-details" aria-label="Commit details">
          <h3>{details.commit.subject}</h3>
          <code>{details.commit.id}</code>
          <p>
            {details.commit.author} ·{" "}
            {new Date(details.commit.timestamp * 1000).toLocaleString()}
          </p>
          <pre>{details.message}</pre>
          {details.parents.length > 0 ? (
            <dl>
              <dt>{details.parents.length === 1 ? "Parent" : "Parents"}</dt>
              <dd>
                {details.parents.map((parent) => (
                  <div key={parent}>
                    <code>{parent}</code>
                  </div>
                ))}
              </dd>
            </dl>
          ) : null}
          <h4>Affected files</h4>
          {details.parents.length > 1 ? (
            <p>Compared with the first parent.</p>
          ) : details.parents.length === 0 ? (
            <p>Initial commit.</p>
          ) : null}
          {details.files.map((file) => (
            <div className="git-file-row" key={file.path}>
              <span>
                {file.path}
                {file.previousPath ? (
                  <small> ← {file.previousPath}</small>
                ) : null}
              </span>
              <small>{file.status}</small>
            </div>
          ))}
          {details.files.length === 0 ? (
            <p>No affected files in this workspace.</p>
          ) : null}
          {details.truncated ? (
            <p>Some commit details were truncated.</p>
          ) : null}
        </section>
      ) : null}
    </>
  );
}
function errorText(error: unknown): string {
  return error instanceof Error
    ? error.message
    : typeof error === "object" && error !== null && "message" in error
      ? String(error.message)
      : "Could not load Git history.";
}
