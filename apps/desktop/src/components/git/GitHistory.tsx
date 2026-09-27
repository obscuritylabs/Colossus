import { IconArrowLeft, IconGitCommit, IconLoader2 } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { GitCommit, GitCommitDetails, GitRepository } from "../../git";
import type { GitController } from "./useGit";
import { FileName, GitFilter, StatusBadge } from "./GitPrimitives";

export function GitHistory({
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
  const [query, setQuery] = useState("");
  const selectedButton = useRef<HTMLButtonElement | null>(null);
  const backButton = useRef<HTMLButtonElement | null>(null);
  const request = useRef(0);
  useEffect(() => {
    if (selected !== null) backButton.current?.focus();
  }, [selected]);
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
  function back() {
    request.current += 1;
    setSelected(null);
    setDetails(null);
    setBusy(false);
    setError("");
    requestAnimationFrame(() => selectedButton.current?.focus());
  }
  const matching = commits.filter((commit) =>
    `${commit.subject} ${commit.author} ${commit.id}`
      .toLowerCase()
      .includes(query.trim().toLowerCase()),
  );
  return (
    <div className="git-history-view">
      <div className="git-history-list" hidden={selected !== null}>
        {commits.length > 0 ? (
          <GitFilter
            value={query}
            onChange={setQuery}
            label="Filter loaded commits"
          />
        ) : null}
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
          {matching.map((commit) => (
            <button
              className="git-commit-row"
              disabled={busy}
              key={commit.id}
              type="button"
              aria-pressed={selected === commit.id}
              onClick={(event) => {
                selectedButton.current = event.currentTarget;
                void inspect(commit.id);
              }}
            >
              <span className="git-timeline-node">
                <IconGitCommit size={16} aria-hidden="true" />
              </span>
              <span className="git-commit-label">
                <strong>{commit.subject}</strong>
                <small>
                  <span>{commit.author}</span>
                  <time>
                    {new Date(commit.timestamp * 1000).toLocaleDateString()}
                  </time>
                </small>
              </span>
              <code>{commit.id.slice(0, 7)}</code>
            </button>
          ))}
        </div>
        {query && matching.length === 0 && !busy ? (
          <p className="git-empty" role="status">
            No loaded commits match “{query}”.
          </p>
        ) : null}
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
      </div>
      {selected !== null ? (
        <section className="git-commit-inspector" aria-label="Commit details">
          <header className="git-detail-toolbar">
            <button
              ref={backButton}
              type="button"
              className="git-back-button"
              onClick={back}
            >
              <IconArrowLeft size={16} aria-hidden="true" />
              Back to history
            </button>
            <code>{selected.slice(0, 7)}</code>
          </header>
          {busy ? (
            <p className="git-progress" role="status">
              <IconLoader2 size={16} className="git-spinner" />
              Reading commit…
            </p>
          ) : null}
          {error ? (
            <div className="git-notice" role="alert">
              {error}
              <button
                type="button"
                className="button secondary compact"
                disabled={busy}
                onClick={() => void inspect(selected)}
              >
                Retry commit
              </button>
            </div>
          ) : null}
          {details ? (
            <div className="git-commit-detail-body">
              <h3>{details.commit.subject}</h3>
              <p className="git-commit-byline">
                {details.commit.author} ·{" "}
                {new Date(details.commit.timestamp * 1000).toLocaleString()}
              </p>
              {details.message.trim() !== details.commit.subject ? (
                <pre>
                  {details.message.split("\n")[0] === details.commit.subject
                    ? details.message
                        .slice(details.commit.subject.length)
                        .trimStart()
                    : details.message}
                </pre>
              ) : null}
              <details className="git-commit-metadata">
                <summary>Commit metadata</summary>
                <dl>
                  <dt>Commit</dt>
                  <dd>
                    <code>{details.commit.id}</code>
                  </dd>
                  <dt>{details.parents.length === 1 ? "Parent" : "Parents"}</dt>
                  <dd>
                    {details.parents.length === 0
                      ? "None — initial commit"
                      : null}
                    {details.parents.map((parent) => (
                      <div key={parent}>
                        <code>{parent}</code>
                      </div>
                    ))}
                  </dd>
                </dl>
              </details>
              <h4>
                Affected files{" "}
                <span className="git-group-count">
                  {details.files.length}
                  {details.truncated ? "+" : ""}
                </span>
              </h4>
              {details.parents.length > 1 ? (
                <p>Compared with the first parent.</p>
              ) : details.parents.length === 0 ? (
                <p>Initial commit.</p>
              ) : null}
              {details.files.map((file) => (
                <div key={file.path}>
                  <div className="git-file-row">
                    <FileName path={file.path} />
                    <StatusBadge status={file.status} />
                  </div>
                  {file.previousPath ? (
                    <p className="git-previous-path">
                      Renamed from {file.previousPath}
                    </p>
                  ) : null}
                </div>
              ))}
              {details.files.length === 0 ? (
                <p>No affected files in this workspace.</p>
              ) : null}
              {details.truncated ? (
                <p>Some commit details were truncated.</p>
              ) : null}
            </div>
          ) : null}
        </section>
      ) : null}
    </div>
  );
}
function errorText(error: unknown): string {
  return error instanceof Error
    ? error.message
    : typeof error === "object" && error !== null && "message" in error
      ? String(error.message)
      : "Could not load Git history.";
}
