import {
  IconChevronsUp,
  IconChevronDown,
  IconChevronRight,
  IconFileCode,
  IconFolder,
  IconRefresh,
  IconSearch,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { WorkspaceDirectory, WorkspaceSummary } from "../../types";
import type { WorkspaceSearchResults } from "../../api";

export type DirectoryLoader = (
  workspaceId: string,
  path?: string,
) => Promise<WorkspaceDirectory>;
export type SearchLoader = (
  workspaceId: string,
  query: string,
) => Promise<WorkspaceSearchResults>;
// Serialize searches across workspace switches. Obsolete queries are skipped before
// dispatch so typing never accumulates filesystem traversals.
let searchTail: Promise<unknown> = Promise.resolve();

export function FileExplorer({
  workspace,
  listDirectory,
  onRootLoaded,
  searchFiles,
  activePath,
  reveal,
  onOpen,
}: {
  workspace: WorkspaceSummary;
  listDirectory: DirectoryLoader;
  onRootLoaded: (root: WorkspaceDirectory) => void;
  searchFiles: SearchLoader;
  activePath: string | null;
  reveal: { path: string; sequence: number } | null;
  onOpen: (path: string) => void;
}) {
  const [directories, setDirectories] = useState<
    Map<string, WorkspaceDirectory>
  >(new Map());
  const cache = useRef(new Map<string, WorkspaceDirectory>());
  const [expanded, setExpanded] = useState(new Set([""]));
  const [loading, setLoading] = useState(new Set<string>());
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<WorkspaceSearchResults | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState("");
  const [notice, setNotice] = useState("");
  const generation = useRef(0);
  const nav = useRef<HTMLElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const load = useCallback(
    async (path: string) => {
      if (cache.current.has(path)) return cache.current.get(path)!;
      const version = generation.current;
      setLoading((old) => new Set(old).add(path));
      try {
        const value = await listDirectory(workspace.workspaceId, path);
        if (version !== generation.current) return null;
        cache.current.set(path, value);
        setDirectories(new Map(cache.current));
        return value;
      } catch (e) {
        if (version === generation.current)
          setError(
            e instanceof Error ? e.message : "This folder could not be opened.",
          );
        return null;
      } finally {
        if (version === generation.current)
          setLoading((old) => {
            const next = new Set(old);
            next.delete(path);
            return next;
          });
      }
    },
    [listDirectory, workspace.workspaceId],
  );
  useEffect(() => {
    void load("").then((root) => {
      if (root) onRootLoaded(root);
    });
    return () => {
      generation.current++;
    };
  }, [load, onRootLoaded]);
  useEffect(() => {
    let live = true;
    setResults(null);
    setSearchError("");
    if (!query.trim()) {
      setSearching(false);
      return;
    }
    setSearching(true);
    const timer = window.setTimeout(() => {
      const action = async () => {
        if (!live) return;
        try {
          const value = await searchFiles(workspace.workspaceId, query);
          if (live) setResults(value);
        } catch (e) {
          if (live)
            setSearchError(
              e instanceof Error ? e.message : "File search failed. Try again.",
            );
        } finally {
          if (live) setSearching(false);
        }
      };
      searchTail = searchTail.then(action, action);
    }, 250);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [query, searchFiles, workspace.workspaceId]);
  useEffect(() => {
    if (!reveal) return;
    let live = true;
    setQuery("");
    setNotice("");
    void (async () => {
      const parts = reveal.path.split("/");
      let parent = "";
      let found = false;
      for (let i = 0; i < parts.length; i++) {
        const directory = await load(parent);
        if (!live) return;
        const currentParent = parent;
        setExpanded((old) => new Set(old).add(currentParent));
        const entry = directory?.entries.find(
          (entry) => entry.name === parts[i],
        );
        if (!entry) {
          setNotice(
            directory?.truncated
              ? "This folder's tree is limited. Search the full path to locate the file."
              : "This file is not in the current workspace tree. Its Git version remains available here.",
          );
          break;
        }
        parent = entry.path;
        found = i === parts.length - 1;
      }
      if (found)
        requestAnimationFrame(() => {
          if (!live) return;
          const item = Array.from(
            nav.current?.querySelectorAll<HTMLButtonElement>(
              "button[data-path]",
            ) ?? [],
          ).find((node) => node.dataset.path === reveal.path);
          item?.scrollIntoView({ block: "nearest" });
          item?.focus({ preventScroll: true });
        });
    })();
    return () => {
      live = false;
    };
  }, [reveal, load]);
  function toggle(path: string) {
    setExpanded((old) => {
      const next = new Set(old);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
    if (!expanded.has(path)) void load(path);
  }
  function tree(path: string, depth = 0): React.ReactNode {
    const directory = directories.get(path);
    if (!directory)
      return loading.has(path) ? (
        <p className="file-tree-loading" role="status">
          Loading…
        </p>
      ) : null;
    return (
      <>
        {directory.entries.map((entry) => (
          <div className="file-tree-node" key={entry.path}>
            <button
              type="button"
              data-path={entry.path}
              className={activePath === entry.path ? "is-active" : undefined}
              style={{ paddingLeft: 8 + depth * 14 }}
              aria-expanded={
                entry.kind === "directory"
                  ? expanded.has(entry.path)
                  : undefined
              }
              aria-current={activePath === entry.path ? "true" : undefined}
              title={entry.path}
              onClick={() =>
                entry.kind === "directory"
                  ? toggle(entry.path)
                  : onOpen(entry.path)
              }
            >
              <span className="file-tree-chevron">
                {entry.kind === "directory" ? (
                  expanded.has(entry.path) ? (
                    <IconChevronDown size={14} />
                  ) : (
                    <IconChevronRight size={14} />
                  )
                ) : null}
              </span>
              <span className="file-tree-icon">
                {entry.kind === "directory" ? (
                  <IconFolder size={16} />
                ) : (
                  <IconFileCode size={16} />
                )}
              </span>
              <span>{entry.name}</span>
            </button>
            {entry.kind === "directory" && expanded.has(entry.path)
              ? tree(entry.path, depth + 1)
              : null}
          </div>
        ))}
        {directory.entries.length === 0 ? (
          <p className="file-tree-loading">No visible files</p>
        ) : null}
        {directory.truncated ? (
          <p className="file-tree-loading">
            Folder list limited. Use search to find more files.
          </p>
        ) : null}
      </>
    );
  }
  return (
    <aside className="file-explorer" aria-label="Workspace files">
      <header className="file-explorer-header">
        <div>
          <h1>{workspace.displayName}</h1>
          <span title={workspace.displayPath}>{workspace.displayPath}</span>
        </div>
        <button
          type="button"
          aria-label="Refresh workspace files"
          title="Refresh workspace files"
          onClick={() => {
            generation.current++;
            cache.current.clear();
            setExpanded(new Set([""]));
            setDirectories(new Map());
            setLoading(new Set());
            setError("");
            setNotice("");
            void load("");
          }}
        >
          <IconRefresh size={16} />
        </button>
      </header>
      <div className="file-explorer-tools">
        <label>
          <IconSearch size={15} aria-hidden="true" />
          <input
            ref={searchRef}
            type="search"
            aria-label="Find files by name or path"
            placeholder="Find files…"
            maxLength={256}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setQuery("");
            }}
          />
        </label>
        <button
          type="button"
          className="icon-button"
          aria-label={query ? "Clear file search" : "Collapse folders"}
          title={query ? "Clear file search" : "Collapse folders"}
          onClick={() => (query ? setQuery("") : setExpanded(new Set([""])))}
        >
          {query ? <IconX size={15} /> : <IconChevronsUp size={15} />}
        </button>
      </div>
      {error ? (
        <p className="file-tree-notice" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p className="file-tree-notice" role="status">
          {notice}
        </p>
      ) : null}
      <nav
        ref={nav}
        className="file-tree"
        aria-label={query ? "File search results" : "Workspace tree"}
        onKeyDown={(e) => {
          if (!(e.target instanceof HTMLButtonElement)) return;
          const buttons = Array.from(
            e.currentTarget.querySelectorAll<HTMLButtonElement>("button"),
          );
          const index = buttons.indexOf(e.target);
          if (
            e.key === "ArrowDown" ||
            e.key === "ArrowUp" ||
            e.key === "Home" ||
            e.key === "End"
          ) {
            e.preventDefault();
            buttons[
              e.key === "Home"
                ? 0
                : e.key === "End"
                  ? buttons.length - 1
                  : Math.min(
                      buttons.length - 1,
                      Math.max(0, index + (e.key === "ArrowDown" ? 1 : -1)),
                    )
            ]?.focus();
          } else if (
            e.key === "ArrowRight" &&
            e.target.getAttribute("aria-expanded") === "false"
          ) {
            e.preventDefault();
            e.target.click();
          } else if (
            e.key === "ArrowLeft" &&
            e.target.getAttribute("aria-expanded") === "true"
          ) {
            e.preventDefault();
            e.target.click();
          }
        }}
      >
        {query ? (
          <>
            {searching ? (
              <p role="status" className="file-tree-loading">
                Searching files…
              </p>
            ) : null}
            {searchError ? <p role="alert">{searchError}</p> : null}
            {results?.paths.map((path) => (
              <button
                className="file-search-result"
                type="button"
                key={path}
                title={path}
                onClick={() => onOpen(path)}
              >
                <IconFileCode size={15} />
                <span>
                  <strong>{path.split("/").at(-1)}</strong>
                  <small>{path}</small>
                </span>
              </button>
            ))}
            {results ? (
              <p className="file-tree-loading" role="status">
                {results.paths.length}{" "}
                {results.paths.length === 1 ? "file" : "files"}
                {results.truncated
                  ? " · Search limit reached; narrow the path or filename."
                  : " found"}
              </p>
            ) : null}
          </>
        ) : (
          tree("")
        )}
      </nav>
      <footer className="file-explorer-footer">
        Read-only · Protected and generated entries hidden
      </footer>
    </aside>
  );
}
