import {
  IconCheck,
  IconChevronRight,
  IconExternalLink,
  IconX,
} from "@tabler/icons-react";
import { useRef, useState } from "react";
import type { GitFile, GitRepository } from "../../git";
import { gitFileGroups } from "../../git";
import { FileName, GitFilter, StatusBadge } from "./GitPrimitives";

export function GitChanges({
  repository,
  onOpenFile,
}: {
  repository: GitRepository;
  onOpenFile: (path: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const lastSelected = useRef<HTMLButtonElement | null>(null);
  const view = useRef<HTMLDivElement | null>(null);
  const selected = repository.files.find((file) => file.path === selectedPath);
  const matching = repository.files.filter((file) =>
    file.path.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const groups = gitFileGroups(matching).filter(
    (group) => group.files.length > 0,
  );
  function toggle(label: string) {
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(label)) next.delete(label);
      else next.add(label);
      return next;
    });
  }
  return (
    <div className="git-changes-view" ref={view}>
      {repository.files.length > 0 ? (
        <GitFilter
          value={query}
          onChange={(value) => {
            setQuery(value);
            setCollapsed(new Set());
          }}
          label="Filter files"
        />
      ) : null}
      {repository.truncated ? (
        <p className="git-notice">
          Showing a limited file list. Counts reflect the displayed files.
        </p>
      ) : null}
      <div className="git-change-list">
        {repository.files.length === 0 ? (
          <div className="git-empty">
            <IconCheck size={24} aria-hidden="true" />
            <h3>
              {repository.truncated
                ? "No displayable changes"
                : "Working tree clean"}
            </h3>
            <p>
              {repository.truncated
                ? "Some paths could not be displayed."
                : "Changes will appear here as you work."}
            </p>
          </div>
        ) : matching.length === 0 ? (
          <p className="git-empty" role="status">
            No files match “{query}”.
          </p>
        ) : null}
        {query && matching.length > 0 ? (
          <p className="git-result-count" role="status">
            {matching.length} of {repository.files.length} files
          </p>
        ) : null}
        {groups.map((group) => {
          const expanded = !collapsed.has(group.label);
          const id = `git-files-${group.label.toLowerCase()}`;
          return (
            <section
              className="git-file-group"
              key={group.label}
              aria-label={group.label}
            >
              <h3>
                <button
                  type="button"
                  className="git-group-toggle"
                  aria-expanded={expanded}
                  aria-controls={id}
                  onClick={() => {
                    toggle(group.label);
                  }}
                >
                  <IconChevronRight size={14} aria-hidden="true" />
                  <span>{group.label}</span>
                  <span className="git-group-count">{group.files.length}</span>
                </button>
              </h3>
              <div id={id} hidden={!expanded}>
                {group.files.map((file) => {
                  const status =
                    group.label === "Conflicts"
                      ? "conflict"
                      : group.label === "Staged"
                        ? file.staged!
                        : group.label === "Unstaged"
                          ? file.unstaged!
                          : "new";
                  return (
                    <button
                      type="button"
                      className="git-file-row"
                      key={file.path}
                      aria-label={`${file.path}, ${group.label}: ${status}`}
                      aria-pressed={selectedPath === file.path}
                      onClick={(event) => {
                        lastSelected.current = event.currentTarget;
                        setSelectedPath(file.path);
                      }}
                    >
                      <FileName path={file.path} />
                      <StatusBadge status={status} />
                    </button>
                  );
                })}
              </div>
            </section>
          );
        })}
      </div>
      {selected ? (
        <FileDetails
          file={selected}
          onOpenFile={onOpenFile}
          onClose={() => {
            setSelectedPath(null);
            if (lastSelected.current?.getClientRects().length)
              lastSelected.current.focus();
            else
              view.current?.querySelector<HTMLInputElement>("input")?.focus();
          }}
        />
      ) : repository.files.length > 0 ? (
        <p className="git-selection-hint">
          Select a file to inspect its changes
        </p>
      ) : null}
    </div>
  );
}

function FileDetails({
  file,
  onOpenFile,
  onClose,
}: {
  file: GitFile;
  onOpenFile: (path: string) => void;
  onClose: () => void;
}) {
  const deleted =
    file.unstaged === "deleted" ||
    (file.staged === "deleted" && !file.untracked && file.unstaged === null);
  return (
    <section className="git-file-inspector" aria-label="Changed file details">
      <header>
        <span>File details</span>
        <button
          type="button"
          className="icon-button"
          aria-label="Close file details"
          onClick={onClose}
        >
          <IconX size={15} />
        </button>
      </header>
      <div className="git-inspector-name">
        <FileName path={file.path} />
      </div>
      <p className="git-full-path">{file.path}</p>
      {file.previousPath ? (
        <p className="git-rename">Renamed from {file.previousPath}</p>
      ) : null}
      <div className="git-file-states">
        {file.conflicted ? (
          <span>
            <StatusBadge status="conflict" />
            Needs resolution
          </span>
        ) : (
          <>
            {file.staged ? (
              <span>
                <StatusBadge status={file.staged} />
                Staged
              </span>
            ) : null}
            {file.unstaged ? (
              <span>
                <StatusBadge status={file.unstaged} />
                Unstaged
              </span>
            ) : null}
            {file.untracked ? (
              <span>
                <StatusBadge status="new" />
                Untracked
              </span>
            ) : null}
          </>
        )}
      </div>
      {deleted ? (
        <p>This file has been deleted.</p>
      ) : (
        <button
          type="button"
          className="button secondary compact git-open-file"
          onClick={() => onOpenFile(file.path)}
        >
          <IconExternalLink size={14} />
          Open current file
        </button>
      )}
    </section>
  );
}
