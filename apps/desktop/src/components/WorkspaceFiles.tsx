import {
  IconChevronDown,
  IconChevronRight,
  IconCode,
  IconFileCode,
  IconFileText,
  IconFolder,
  IconFolderOpen,
  IconRefresh,
  IconShieldLock,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type {
  WorkspaceDirectory,
  WorkspaceEntry,
  WorkspaceFile,
  WorkspaceSummary,
} from "../types";
import type { ResolvedColorTheme } from "../theme/appearance";
import { useAppearance } from "../theme/AppearanceProvider";

const MAX_OPEN_FILES = 8;

type DirectoryLoader = (
  workspaceId: string,
  path?: string,
) => Promise<WorkspaceDirectory>;
type FileLoader = (workspaceId: string, path: string) => Promise<WorkspaceFile>;

interface WorkspaceFilesProps {
  workspace: WorkspaceSummary | null;
  available: boolean;
  listDirectory: DirectoryLoader;
  readFile: FileLoader;
  onOpenSettings: () => void;
  openRequest: WorkspaceFileOpenRequest | null;
}

export interface WorkspaceFileOpenRequest {
  workspaceId: string;
  path: string;
  requestId: number;
}

function humanFileSize(bytes: number): string {
  if (bytes < 1_024) {
    return `${bytes} B`;
  }
  if (bytes < 1_024 * 1_024) {
    return `${(bytes / 1_024).toFixed(bytes < 10_240 ? 1 : 0)} KB`;
  }
  return `${(bytes / (1_024 * 1_024)).toFixed(1)} MB`;
}

function entryIcon(entry: WorkspaceEntry, expanded: boolean) {
  if (entry.kind === "directory") {
    return expanded ? (
      <IconFolderOpen size={16} stroke={1.7} aria-hidden="true" />
    ) : (
      <IconFolder size={16} stroke={1.7} aria-hidden="true" />
    );
  }
  const extension = entry.name.split(".").at(-1)?.toLowerCase();
  return [
    "c",
    "cc",
    "cpp",
    "css",
    "go",
    "h",
    "html",
    "java",
    "js",
    "jsx",
    "py",
    "rs",
    "sh",
    "sql",
    "ts",
    "tsx",
  ].includes(extension ?? "") ? (
    <IconFileCode size={16} stroke={1.65} aria-hidden="true" />
  ) : (
    <IconFileText size={16} stroke={1.65} aria-hidden="true" />
  );
}

async function highlight(
  content: string,
  language: string,
  colorTheme: ResolvedColorTheme,
): Promise<import("../syntax-highlighter").HighlightedLine[]> {
  const { highlightSource } = await import("../syntax-highlighter");
  return highlightSource(content, language, colorTheme);
}

function HighlightedCode({
  file,
  colorTheme,
}: {
  file: WorkspaceFile;
  colorTheme: ResolvedColorTheme;
}) {
  const [lines, setLines] = useState<
    import("../syntax-highlighter").HighlightedLine[] | null
  >(null);

  useEffect(() => {
    let current = true;
    setLines(null);
    void highlight(file.content, file.language, colorTheme)
      .then((highlighted) => {
        if (current) {
          setLines(highlighted);
        }
      })
      .catch(() => {
        if (current) {
          setLines(
            file.content
              .split("\n")
              .map((line) => [{ content: line, color: undefined }]),
          );
        }
      });
    return () => {
      current = false;
    };
  }, [colorTheme, file.content, file.language]);

  const visibleLines =
    lines ??
    file.content
      .split("\n")
      .map((line) => [{ content: line, color: undefined }]);

  return (
    <div
      className="file-code-scroll"
      aria-label={`${file.name} source preview`}
      tabIndex={0}
    >
      <div className="file-code" role="presentation">
        {visibleLines.map((line, index) => (
          <div className="file-code-line" key={`${index}-${line.length}`}>
            <span className="file-line-number" aria-hidden="true">
              {index + 1}
            </span>
            <code>
              {line.length === 0 ? (
                <span>&nbsp;</span>
              ) : (
                line.map((token, tokenIndex) => (
                  <span
                    key={`${tokenIndex}-${token.content.length}`}
                    style={
                      token.color === undefined
                        ? undefined
                        : { color: token.color }
                    }
                  >
                    {token.content}
                  </span>
                ))
              )}
            </code>
          </div>
        ))}
      </div>
    </div>
  );
}

export function WorkspaceFiles({
  workspace,
  available,
  listDirectory,
  readFile,
  onOpenSettings,
  openRequest,
}: WorkspaceFilesProps) {
  const { resolvedColorTheme } = useAppearance();
  const [directories, setDirectories] = useState<
    ReadonlyMap<string, WorkspaceDirectory>
  >(new Map());
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set([""]));
  const [directoryLoading, setDirectoryLoading] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [files, setFiles] = useState<ReadonlyMap<string, WorkspaceFile>>(
    new Map(),
  );
  const filesRef = useRef<ReadonlyMap<string, WorkspaceFile>>(new Map());
  const [openPaths, setOpenPaths] = useState<readonly string[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [loadingPath, setLoadingPath] = useState<string | null>(null);
  const [error, setError] = useState("");
  const requestGeneration = useRef(0);
  const openRequestRef = useRef(openRequest);
  openRequestRef.current = openRequest;

  const openFile = useCallback(
    async (path: string) => {
      if (workspace === null || !available) {
        return;
      }
      setError("");
      setActivePath(path);
      setOpenPaths((current) => {
        if (current.includes(path)) {
          return current;
        }
        return [...current.slice(-(MAX_OPEN_FILES - 1)), path];
      });
      if (filesRef.current.has(path)) {
        return;
      }
      const generation = requestGeneration.current;
      setLoadingPath(path);
      try {
        const preview = await readFile(workspace.workspaceId, path);
        if (generation !== requestGeneration.current) {
          return;
        }
        setFiles((current) => {
          const next = new Map(current).set(path, preview);
          filesRef.current = next;
          return next;
        });
      } catch (cause: unknown) {
        if (generation !== requestGeneration.current) {
          return;
        }
        setError(
          cause instanceof Error
            ? cause.message
            : "This file could not be previewed.",
        );
      } finally {
        if (generation === requestGeneration.current) {
          setLoadingPath((current) => (current === path ? null : current));
        }
      }
    },
    [available, readFile, workspace],
  );

  const loadDirectory = useCallback(
    async (path: string) => {
      if (workspace === null || !available) {
        return null;
      }
      setDirectoryLoading((current) => new Set(current).add(path));
      setError("");
      const generation = requestGeneration.current;
      try {
        const directory = await listDirectory(workspace.workspaceId, path);
        if (generation !== requestGeneration.current) {
          return null;
        }
        setDirectories((current) => new Map(current).set(path, directory));
        return directory;
      } catch (cause: unknown) {
        if (generation === requestGeneration.current) {
          setError(
            cause instanceof Error
              ? cause.message
              : "This folder could not be opened.",
          );
        }
        return null;
      } finally {
        if (generation === requestGeneration.current) {
          setDirectoryLoading((current) => {
            const next = new Set(current);
            next.delete(path);
            return next;
          });
        }
      }
    },
    [available, listDirectory, workspace],
  );

  const resetExplorer = useCallback(() => {
    requestGeneration.current += 1;
    setDirectories(new Map());
    setExpanded(new Set([""]));
    setDirectoryLoading(new Set());
    setFiles(new Map());
    filesRef.current = new Map();
    setOpenPaths([]);
    setActivePath(null);
    setLoadingPath(null);
    setError("");
  }, []);

  useEffect(() => {
    resetExplorer();
    if (workspace === null || !available) {
      return;
    }
    const generation = requestGeneration.current;
    void listDirectory(workspace.workspaceId, "")
      .then((root) => {
        if (generation !== requestGeneration.current) {
          return;
        }
        setDirectories(new Map([["", root]]));
        const readme = root.entries.find(
          (entry) =>
            entry.kind === "file" && entry.name.toLowerCase() === "readme.md",
        );
        if (
          readme !== undefined &&
          openRequestRef.current?.workspaceId !== workspace.workspaceId
        ) {
          void openFile(readme.path);
        }
      })
      .catch((cause: unknown) => {
        if (generation === requestGeneration.current) {
          setError(
            cause instanceof Error
              ? cause.message
              : "The workspace could not be opened.",
          );
        }
      });
  }, [available, listDirectory, openFile, resetExplorer, workspace]);

  useEffect(() => {
    if (
      openRequest === null ||
      workspace === null ||
      openRequest.workspaceId !== workspace.workspaceId
    ) {
      return;
    }
    void openFile(openRequest.path);
  }, [openFile, openRequest, workspace]);

  const activeFile = activePath === null ? undefined : files.get(activePath);
  const exclusionCount = useMemo(
    () =>
      Array.from(directories.values()).reduce(
        (total, directory) => total + directory.excludedCount,
        0,
      ),
    [directories],
  );

  function toggleDirectory(path: string) {
    const isExpanded = expanded.has(path);
    setExpanded((current) => {
      const next = new Set(current);
      if (isExpanded) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
    if (!isExpanded && !directories.has(path)) {
      void loadDirectory(path);
    }
  }

  function closeFile(path: string) {
    setOpenPaths((current) => {
      const index = current.indexOf(path);
      const next = current.filter((candidate) => candidate !== path);
      if (activePath === path) {
        setActivePath(next[index] ?? next[index - 1] ?? null);
      }
      return next;
    });
  }

  function renderDirectory(path: string, depth: number): React.ReactNode {
    const directory = directories.get(path);
    if (directory === undefined) {
      return directoryLoading.has(path) ? (
        <p
          className="file-tree-loading"
          style={{ paddingLeft: 16 + depth * 14 }}
        >
          Loading…
        </p>
      ) : null;
    }
    return directory.entries.map((entry) => {
      const isDirectory = entry.kind === "directory";
      const isExpanded = isDirectory && expanded.has(entry.path);
      return (
        <div className="file-tree-node" key={entry.path}>
          <button
            type="button"
            className={activePath === entry.path ? "is-active" : undefined}
            style={{ paddingLeft: 9 + depth * 14 }}
            aria-expanded={isDirectory ? isExpanded : undefined}
            title={entry.path}
            onClick={() => {
              if (isDirectory) {
                toggleDirectory(entry.path);
              } else {
                void openFile(entry.path);
              }
            }}
          >
            <span className="file-tree-chevron" aria-hidden="true">
              {isDirectory ? (
                isExpanded ? (
                  <IconChevronDown size={14} stroke={1.8} />
                ) : (
                  <IconChevronRight size={14} stroke={1.8} />
                )
              ) : null}
            </span>
            <span className="file-tree-icon" aria-hidden="true">
              {entryIcon(entry, isExpanded)}
            </span>
            <span>{entry.name}</span>
          </button>
          {isDirectory && isExpanded
            ? renderDirectory(entry.path, depth + 1)
            : null}
        </div>
      );
    });
  }

  return (
    <section className="workspace-files-drawer" aria-label="Workspace files">
      <aside className="file-explorer" aria-label="Workspace files">
        <header className="file-explorer-header">
          <div>
            <p>Workspace files</p>
            <h1>{workspace?.displayName ?? "Files"}</h1>
            <span>{workspace?.displayPath ?? "No workspace selected"}</span>
          </div>
          <button
            type="button"
            aria-label="Refresh workspace files"
            title="Refresh workspace files"
            disabled={!available}
            onClick={() => {
              resetExplorer();
              if (workspace !== null && available) {
                void loadDirectory("");
              }
            }}
          >
            <IconRefresh size={17} stroke={1.7} aria-hidden="true" />
          </button>
        </header>

        {available ? (
          <>
            <nav className="file-tree" aria-label="Workspace tree">
              {renderDirectory("", 0)}
              {directoryLoading.has("") ? (
                <p className="file-tree-loading">Loading workspace…</p>
              ) : null}
            </nav>
            <footer className="file-explorer-footer">
              <IconShieldLock size={15} stroke={1.7} aria-hidden="true" />
              <span>
                Read-only · {exclusionCount} protected or generated{" "}
                {exclusionCount === 1 ? "entry" : "entries"} hidden
              </span>
            </footer>
          </>
        ) : (
          <div className="file-explorer-unavailable">
            <IconShieldLock size={23} stroke={1.5} aria-hidden="true" />
            <strong>Managed Local files unavailable</strong>
            <p>
              Select the local workspace and enable Development or Allow all
              access to browse it.
            </p>
            <button
              className="button secondary"
              type="button"
              onClick={onOpenSettings}
            >
              Open settings
            </button>
          </div>
        )}
      </aside>

      <section className="file-workspace" aria-label="File preview">
        <header className="surface-header file-surface-header">
          <div className="surface-title-copy">
            <p className="surface-breadcrumb">Files / Workspace</p>
            <h2>{activeFile?.name ?? "Workspace preview"}</h2>
            <span>
              {activeFile?.path ??
                "Open a source file from the explorer to inspect it here."}
            </span>
          </div>
          <span className="file-readonly-badge">
            <IconShieldLock size={15} stroke={1.7} aria-hidden="true" />
            Read-only
          </span>
        </header>

        <nav className="file-tabs" aria-label="Open files">
          {openPaths.map((path) => {
            const opened = files.get(path);
            const name = opened?.name ?? path.split("/").at(-1) ?? path;
            return (
              <div className="file-tab-wrap" key={path}>
                <button
                  className="file-tab"
                  type="button"
                  aria-pressed={activePath === path}
                  title={path}
                  onClick={() => setActivePath(path)}
                >
                  <IconFileCode size={14} stroke={1.6} aria-hidden="true" />
                  <span>{name}</span>
                </button>
                <button
                  className="file-tab-close"
                  type="button"
                  aria-label={`Close ${name}`}
                  title={`Close ${name}`}
                  onClick={() => closeFile(path)}
                >
                  <IconX size={13} stroke={1.8} aria-hidden="true" />
                </button>
              </div>
            );
          })}
        </nav>

        {error !== "" ? (
          <div className="file-preview-error" role="alert">
            <IconShieldLock size={19} stroke={1.6} aria-hidden="true" />
            <div>
              <strong>Preview unavailable</strong>
              <p>{error}</p>
            </div>
          </div>
        ) : null}

        {activeFile !== undefined ? (
          <section className="file-preview">
            <div className="file-preview-meta">
              <span>
                <IconCode size={15} stroke={1.7} aria-hidden="true" />
                {activeFile.language}
              </span>
              <span>{activeFile.lineCount.toLocaleString()} lines</span>
              <span>{humanFileSize(activeFile.sizeBytes)}</span>
              <span>UTF-8</span>
            </div>
            <HighlightedCode
              file={activeFile}
              colorTheme={resolvedColorTheme}
            />
          </section>
        ) : loadingPath !== null ? (
          <div className="file-preview-empty">
            <IconCode size={30} stroke={1.3} aria-hidden="true" />
            <strong>Opening {loadingPath.split("/").at(-1)}</strong>
            <p>Preparing a bounded syntax-highlighted preview…</p>
          </div>
        ) : (
          <div className="file-preview-empty">
            <IconCode size={30} stroke={1.3} aria-hidden="true" />
            <strong>Select a file to preview</strong>
            <p>
              Source stays read-only here. Colossus changes files through the
              existing policy and approval path.
            </p>
          </div>
        )}
      </section>
    </section>
  );
}
