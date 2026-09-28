import {
  IconFileCode,
  IconGitCompare,
  IconLoader2,
  IconRefresh,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
  WorkspaceFile,
  WorkspaceSummary,
  WorkspaceDirectory,
} from "../types";
import type { GitDiffSelection, GitFileDiff } from "../git";
import { useAppearance } from "../theme/AppearanceProvider";
import {
  FileExplorer,
  type DirectoryLoader,
  type SearchLoader,
} from "./files/FileExplorer";
import { HighlightedCode } from "./files/SourcePreview";
import { DiffViewer } from "./files/DiffViewer";
import "./files/files.css";

export interface WorkspaceFileOpenRequest {
  workspaceId: string;
  path: string;
  requestId: number;
  diff?: GitDiffSelection | undefined;
}
interface WorkspaceFilesProps {
  workspace: WorkspaceSummary | null;
  available: boolean;
  listDirectory: DirectoryLoader;
  searchFiles: SearchLoader;
  readFile: (workspaceId: string, path: string) => Promise<WorkspaceFile>;
  readDiff: (
    workspaceId: string,
    path: string,
    selection: GitDiffSelection,
  ) => Promise<GitFileDiff>;
  onOpenSettings: () => void;
  openRequest: WorkspaceFileOpenRequest | null;
}
interface Document {
  id: string;
  path: string;
  selection?: GitDiffSelection | undefined;
  file?: WorkspaceFile;
  diff?: GitFileDiff;
  loading: boolean;
  error: string;
  version: number;
}

export function WorkspaceFiles(props: WorkspaceFilesProps) {
  if (!props.available || !props.workspace)
    return (
      <section className="file-explorer-unavailable">
        <strong>Managed Local files unavailable</strong>
        <p>
          Select the local workspace and enable Development or Allow all access
          to browse it.
        </p>
        <button
          className="button secondary"
          type="button"
          onClick={props.onOpenSettings}
        >
          Open settings
        </button>
      </section>
    );
  // A selection change disposes all file contents and outstanding UI requests.
  return (
    <WorkspaceFilesView
      key={props.workspace.workspaceId}
      {...props}
      workspace={props.workspace}
    />
  );
}

function WorkspaceFilesView({
  workspace,
  listDirectory,
  searchFiles,
  readFile,
  readDiff,
  openRequest,
}: WorkspaceFilesProps & { workspace: WorkspaceSummary }) {
  const { resolvedColorTheme } = useAppearance();
  const [documents, setDocuments] = useState<Document[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [reveal, setReveal] = useState<{
    path: string;
    sequence: number;
  } | null>(null);
  const [explorerVisible, setExplorerVisible] = useState(true);
  const request = useRef(0);
  const live = useRef(true);
  const documentNav = useRef<HTMLElement>(null);
  const tabs = useRef(documents);
  tabs.current = documents;
  const active = documents.find((doc) => doc.id === activeId);
  useEffect(() => {
    const nav = documentNav.current;
    if (!nav) return;
    const revealActiveTab = () =>
      nav
        .querySelector<HTMLButtonElement>('button[aria-pressed="true"]')
        ?.parentElement?.scrollIntoView({
          block: "nearest",
          inline: "nearest",
        });
    revealActiveTab();
    const observer = new ResizeObserver(revealActiveTab);
    observer.observe(nav);
    return () => observer.disconnect();
  }, [activeId, documents]);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const open = useCallback(
    async (path: string, selection?: GitDiffSelection, refresh = false) => {
      const id = selection
        ? `${selection.source}:${selection.commitId ?? ""}:${path}`
        : `file:${path}`;
      setActiveId(id);
      const existing = tabs.current.find((doc) => doc.id === id);
      if (!refresh && existing && !existing.error) return;
      const version = ++request.current;
      const document: Document = {
        id,
        path,
        selection,
        loading: true,
        error: "",
        version,
      };
      // Eight live documents is also the content-cache bound; closing a tab releases it.
      setDocuments((old) => [
        ...old.filter((doc) => doc.id !== id).slice(-7),
        document,
      ]);
      try {
        const result = selection
          ? { diff: await readDiff(workspace.workspaceId, path, selection) }
          : { file: await readFile(workspace.workspaceId, path) };
        if (live.current)
          setDocuments((old) =>
            old.map((doc) =>
              doc.id === id && doc.version === version
                ? { ...doc, ...result, loading: false }
                : doc,
            ),
          );
      } catch (e) {
        if (live.current)
          setDocuments((old) =>
            old.map((doc) =>
              doc.id === id && doc.version === version
                ? {
                    ...doc,
                    loading: false,
                    error:
                      e instanceof Error
                        ? e.message
                        : "This preview is unavailable. Refresh and try again.",
                  }
                : doc,
            ),
          );
      }
    },
    [workspace.workspaceId, readDiff, readFile],
  );
  useEffect(() => {
    if (!openRequest || openRequest.workspaceId !== workspace.workspaceId)
      return;
    void open(openRequest.path, openRequest.diff, true);
    setReveal({ path: openRequest.path, sequence: openRequest.requestId });
  }, [openRequest, workspace.workspaceId, open]);
  const openReadme = useCallback(
    (root: WorkspaceDirectory) => {
      const readme = root.entries.find(
        (entry) =>
          entry.kind === "file" && entry.name.toLowerCase() === "readme.md",
      );
      if (request.current === 0 && readme) void open(readme.path);
    },
    [open],
  );
  function locate(path: string) {
    setExplorerVisible(true);
    setReveal((old) => ({ path, sequence: (old?.sequence ?? 0) + 1 }));
  }
  function close(id: string) {
    requestAnimationFrame(() =>
      documentNav.current
        ?.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')
        ?.focus(),
    );
    const index = documents.findIndex((doc) => doc.id === id);
    const remaining = documents.filter((doc) => doc.id !== id);
    setDocuments(remaining);
    if (activeId === id)
      setActiveId(remaining[index]?.id ?? remaining[index - 1]?.id ?? null);
  }
  return (
    <section
      className={`workspace-files-drawer enhanced-files${explorerVisible ? "" : " explorer-hidden"}`}
      aria-label="Workspace files"
    >
      {explorerVisible ? (
        <FileExplorer
          workspace={workspace}
          listDirectory={listDirectory}
          onRootLoaded={openReadme}
          searchFiles={searchFiles}
          activePath={active?.path ?? null}
          reveal={reveal}
          onOpen={(path) => {
            void open(path);
          }}
        />
      ) : null}
      <section className="file-workspace" aria-label="File preview">
        <header className="file-viewer-header">
          <button
            type="button"
            className="button secondary compact"
            aria-expanded={explorerVisible}
            onClick={() => setExplorerVisible((v) => !v)}
          >
            {explorerVisible ? "Hide files" : "Show files"}
          </button>
          <span title={active?.path}>{active?.path ?? "Workspace files"}</span>
          <span className="file-viewer-readonly">Read-only</span>
          <button
            type="button"
            className="icon-button"
            aria-label="Refresh file preview"
            disabled={!active || active.loading}
            onClick={() =>
              active && void open(active.path, active.selection, true)
            }
          >
            <IconRefresh size={16} />
          </button>
        </header>
        <nav ref={documentNav} className="file-tabs" aria-label="Open files">
          {documents.map((doc) => (
            <div className="file-tab-wrap" key={doc.id}>
              <button
                className="file-tab"
                type="button"
                aria-pressed={activeId === doc.id}
                title={`${doc.path}${doc.selection ? ` · ${doc.selection.source}` : ""}`}
                onClick={() => setActiveId(doc.id)}
              >
                {doc.selection ? (
                  <IconGitCompare size={14} />
                ) : (
                  <IconFileCode size={14} />
                )}
                <span>
                  {doc.path.split("/").at(-1)}
                  {doc.selection
                    ? ` · ${doc.selection.source === "commit" ? doc.selection.commitId?.slice(0, 7) : doc.selection.source}`
                    : ""}
                </span>
              </button>
              <button
                className="file-tab-close"
                type="button"
                aria-label={`Close ${doc.path.split("/").at(-1)}${doc.selection ? ` ${doc.selection.source} diff` : ""}`}
                onClick={() => close(doc.id)}
              >
                <IconX size={13} />
              </button>
            </div>
          ))}
        </nav>
        {active?.loading ? (
          <div className="file-preview-empty" role="status">
            <IconLoader2 size={25} className="git-spinner" />
            <strong>Opening {active.path.split("/").at(-1)}…</strong>
          </div>
        ) : active?.error ? (
          <div className="file-preview-error" role="alert">
            <strong>Preview unavailable</strong>
            <p>{active.error}</p>
            <button
              type="button"
              className="button secondary compact"
              onClick={() => void open(active.path, active.selection, true)}
            >
              Retry preview
            </button>
          </div>
        ) : active?.diff ? (
          <DiffViewer
            key={`${active.id}:${active.version}`}
            diff={active.diff}
            onReveal={() => locate(active.path)}
            onOpenFile={() => void open(active.path)}
          />
        ) : active?.file ? (
          <section className="file-preview">
            <div className="file-preview-meta">
              <span>{active.file.language}</span>
              <span>{active.file.lineCount.toLocaleString()} lines</span>
              <span>{active.file.sizeBytes.toLocaleString()} bytes</span>
              <button
                type="button"
                className="button secondary compact"
                onClick={() => locate(active.path)}
              >
                Reveal in explorer
              </button>
            </div>
            {active.file.content === "" ? (
              <p className="diff-empty">Empty file.</p>
            ) : (
              <HighlightedCode
                key={`${active.id}:${active.version}`}
                file={active.file}
                colorTheme={resolvedColorTheme}
              />
            )}
          </section>
        ) : (
          <div className="file-preview-empty">
            <IconFileCode size={30} />
            <strong>Select a file to preview</strong>
            <p>Find a file by name or open a changed file from Git.</p>
            <p>
              Files stay read-only. Changes use the existing policy and approval
              path.
            </p>
          </div>
        )}
      </section>
    </section>
  );
}
