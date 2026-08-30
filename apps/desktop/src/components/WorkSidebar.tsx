import {
  IconActivity,
  IconAlertCircle,
  IconArchive,
  IconBriefcase2,
  IconCheck,
  IconChevronDown,
  IconFolder,
  IconLibrary,
  IconLoader2,
  IconDots,
  IconPencil,
  IconPencilPlus,
  IconPin,
  IconPlugConnected,
  IconPlus,
  IconPointFilled,
  IconRestore,
  IconSearch,
  IconSettings,
  IconTerminal2,
  IconTopologyStar3,
  IconWorld,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  presentRunStatus,
  runModeLabel,
  selectRecentWork,
  shortDateLabel,
} from "../presenters";
import {
  MAX_WORK_SIDEBAR_WIDTH,
  MIN_WORK_SIDEBAR_WIDTH,
  clampWorkSidebarWidth,
  defaultWorkSidebarWidth,
} from "../sidebar-width";
import {
  MAX_THREAD_NAME_CHARACTERS,
  normalizeThreadName,
} from "../thread-names";
import type {
  ConnectionState,
  DesktopCapabilities,
  Run,
  SpaceSearchResult,
  SpaceSummary,
} from "../types";
import { isTerminalStatus } from "../types";
import type { WorkspaceSurface } from "./ProductRail";

export type SpaceSearchScope = "space" | "all";

export interface SpaceActionFeedback {
  tone: "progress" | "success" | "error";
  message: string;
}

export interface SpaceStartup {
  spaceId: string | null;
  displayName: string;
}

interface WorkSidebarProps {
  runs: readonly Run[];
  spaces: readonly SpaceSummary[];
  selectedSpaceId: string | null;
  surface: WorkspaceSurface;
  connectionState: ConnectionState;
  capabilities: DesktopCapabilities;
  terminalEnabled: boolean;
  terminalAvailable: boolean;
  activeSessionId: string | null;
  pinnedSessionIds: ReadonlySet<string>;
  resolveThreadTitle: (
    spaceId: string | null,
    sessionId: string,
    fallback: string,
  ) => string;
  query: string;
  searchScope: SpaceSearchScope;
  includeArchived: boolean;
  searchResults: readonly SpaceSearchResult[];
  searchBusy: boolean;
  searchError: string;
  searchHasMore: boolean;
  spaceThreadPreviews: ReadonlyMap<string, readonly SpaceSearchResult[]>;
  spaceThreadPreviewBusyIds: ReadonlySet<string>;
  spaceThreadPreviewErrors: ReadonlyMap<string, string>;
  busy: boolean;
  error: string;
  spaceActionFeedback: SpaceActionFeedback | null;
  hasMore: boolean;
  disabled: boolean;
  spaceStartup: SpaceStartup | null;
  threadLifecycleBusySessionId: string | null;
  sidebarWidth: number | null;
  drawerOpen: boolean;
  onQueryChange: (query: string) => void;
  onSearchScopeChange: (scope: SpaceSearchScope) => void;
  onIncludeArchivedChange: (include: boolean) => void;
  onNewWork: () => void;
  onSelect: (run: Run) => void;
  onSelectSearchResult: (result: SpaceSearchResult) => void;
  onLoadMore: () => void;
  onLoadMoreSearch: () => void;
  onLoadSpaceThreadPreview: (spaceId: string) => void;
  onSelectSpace: (spaceId: string) => void;
  onCreateSpace: () => void;
  onRenameSpace: (spaceId: string, displayName: string) => void;
  onArchiveSpace: (spaceId: string) => void;
  onRestoreSpace: (spaceId: string) => void;
  onArchiveThread: (run: Run) => void;
  onRenameThread: (run: Run, name: string) => void;
  onToggleThreadPinned: (run: Run) => void;
  onRestoreThread: (result: SpaceSearchResult) => void;
  onSelectSurface: (surface: WorkspaceSurface) => void;
  onOpenTerminal: () => void;
  onOpenShell: () => void;
  onSidebarWidthPreview: (width: number) => void;
  onSidebarWidthCommit: (width: number) => void;
  onSidebarWidthReset: () => void;
  onDrawerOpen: () => void;
  onDrawerClose: () => void;
}

const DESTINATIONS = [
  { id: "work", label: "Work", Icon: IconBriefcase2 },
  { id: "fleet", label: "Capabilities", Icon: IconTopologyStar3 },
  { id: "activity", label: "Activity", Icon: IconActivity },
  { id: "library", label: "Library", Icon: IconLibrary },
  { id: "connections", label: "Connections", Icon: IconPlugConnected },
  { id: "settings", label: "Settings", Icon: IconSettings },
] as const;

function statusIcon(tone: string) {
  if (tone === "success") {
    return <IconCheck size={13} stroke={2.2} aria-hidden="true" />;
  }
  if (tone === "danger" || tone === "attention") {
    return <IconAlertCircle size={13} stroke={2} aria-hidden="true" />;
  }
  return <IconPointFilled size={12} stroke={2} aria-hidden="true" />;
}

function runtimeLabel(space: SpaceSummary): string {
  switch (space.state) {
    case "ready":
      return "Ready";
    case "starting":
    case "restarting":
      return "Starting";
    case "sleeping":
      return "Idle";
    case "needs_workspace":
      return "Choose folder";
    case "stopping":
      return "Stopping";
    case "failed":
      return "Failed";
    case "needs_provider":
      return "Setup needed";
    case "archived":
      return "Archived";
  }
}

function searchResultTone(result: SpaceSearchResult): string {
  return presentRunStatus(result.status).tone;
}

export function WorkSidebar({
  runs,
  spaces,
  selectedSpaceId,
  surface,
  connectionState,
  capabilities,
  terminalEnabled,
  terminalAvailable,
  activeSessionId,
  pinnedSessionIds,
  resolveThreadTitle,
  query,
  searchScope,
  includeArchived,
  searchResults,
  searchBusy,
  searchError,
  searchHasMore,
  spaceThreadPreviews,
  spaceThreadPreviewBusyIds,
  spaceThreadPreviewErrors,
  busy,
  error,
  spaceActionFeedback,
  hasMore,
  disabled,
  spaceStartup,
  threadLifecycleBusySessionId,
  sidebarWidth,
  drawerOpen,
  onQueryChange,
  onSearchScopeChange,
  onIncludeArchivedChange,
  onNewWork,
  onSelect,
  onSelectSearchResult,
  onLoadMore,
  onLoadMoreSearch,
  onLoadSpaceThreadPreview,
  onSelectSpace,
  onCreateSpace,
  onRenameSpace,
  onArchiveSpace,
  onRestoreSpace,
  onArchiveThread,
  onRenameThread,
  onToggleThreadPinned,
  onRestoreThread,
  onSelectSurface,
  onOpenTerminal,
  onOpenShell,
  onSidebarWidthPreview,
  onSidebarWidthCommit,
  onSidebarWidthReset,
  onDrawerOpen,
  onDrawerClose,
}: WorkSidebarProps) {
  const sidebarRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const focusSearchWhenOpened = useRef(false);
  const sidebarResizeRef = useRef<{
    pointerId: number;
    startX: number;
    startWidth: number;
    width: number;
  } | null>(null);
  const [announcedSidebarWidth, setAnnouncedSidebarWidth] =
    useState(sidebarWidth);
  const [renameSpaceId, setRenameSpaceId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameThreadSessionId, setRenameThreadSessionId] = useState<
    string | null
  >(null);
  const [renameThreadDraft, setRenameThreadDraft] = useState("");
  const [threadShelfOpen, setThreadShelfOpen] = useState(true);
  const [showAllRecent, setShowAllRecent] = useState(false);
  const [expandedSpaceIds, setExpandedSpaceIds] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const displayRuns = useMemo(
    () =>
      runs.map((run) => {
        const title = resolveThreadTitle(
          selectedSpaceId,
          run.sessionId,
          run.title,
        );
        return title === run.title ? run : { ...run, title };
      }),
    [resolveThreadTitle, runs, selectedSpaceId],
  );
  const groups = useMemo(
    () => selectRecentWork(displayRuns, { query: "", pinnedSessionIds }),
    [displayRuns, pinnedSessionIds],
  );
  const hasRecentGroup = groups.some((group) => group.key === "recent");
  const runsById = useMemo(
    () => new Map(runs.map((run) => [run.runId, run])),
    [runs],
  );
  const activeSpaces = spaces.filter((space) => !space.archived);
  const archivedSpaces = spaces.filter((space) => space.archived);
  const selectedSpace = spaces.find(
    (space) => space.spaceId === selectedSpaceId,
  );
  const startingSpace =
    spaceStartup?.spaceId === null
      ? undefined
      : spaces.find((space) => space.spaceId === spaceStartup?.spaceId);
  const displayedSpace = startingSpace ?? selectedSpace;
  const otherSpaces = activeSpaces.filter(
    (space) => space.spaceId !== displayedSpace?.spaceId,
  );
  const actionsDisabled = disabled || spaceStartup !== null;
  const searching = query.trim() !== "";
  // These are stable information-architecture destinations, even when the
  // selected Workspace currently has no entries for a catalog. The destination's
  // empty state explains that absence without making the navigation itself
  // appear to change as sidecar capabilities come and go.
  const visibleDestinations = DESTINATIONS;

  function toggleSpacePreview(spaceId: string) {
    const opening = !expandedSpaceIds.has(spaceId);
    setExpandedSpaceIds((current) => {
      const next = new Set(current);
      if (opening) {
        next.add(spaceId);
      } else {
        next.delete(spaceId);
      }
      return next;
    });
    if (
      opening &&
      !spaceThreadPreviews.has(spaceId) &&
      !spaceThreadPreviewBusyIds.has(spaceId)
    ) {
      onLoadSpaceThreadPreview(spaceId);
    }
  }

  useEffect(() => {
    setThreadShelfOpen(true);
    setRenameThreadSessionId(null);
    setRenameThreadDraft("");
  }, [displayedSpace?.spaceId]);

  useEffect(() => {
    function onKeyDown(event: globalThis.KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        const activeModal = document.querySelector<HTMLElement>(
          '[role="dialog"][aria-modal="true"]',
        );
        if (activeModal !== null && activeModal.id !== "work-navigation") {
          return;
        }
        if (window.matchMedia("(max-width: 980px)").matches && !drawerOpen) {
          focusSearchWhenOpened.current = true;
          onDrawerOpen();
        } else {
          setThreadShelfOpen(true);
          window.requestAnimationFrame(() => searchRef.current?.focus());
        }
      }
      if (
        event.key === "Escape" &&
        document.activeElement === searchRef.current
      ) {
        onQueryChange("");
        searchRef.current?.blur();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [drawerOpen, onDrawerOpen, onQueryChange]);

  useEffect(() => {
    if (!drawerOpen) {
      return;
    }

    const primary = document.querySelector<HTMLElement>("#primary-workspace");
    const obscured =
      primary === null
        ? []
        : [{ element: primary, wasInert: primary.hasAttribute("inert") }];
    for (const { element } of obscured) {
      element.setAttribute("inert", "");
    }

    const focusTimer = window.setTimeout(() => {
      const shouldFocusSearch = focusSearchWhenOpened.current;
      focusSearchWhenOpened.current = false;
      if (sidebarRef.current?.contains(document.activeElement)) {
        return;
      }
      if (shouldFocusSearch) {
        searchRef.current?.focus();
      } else {
        closeButtonRef.current?.focus();
      }
    }, 180);
    function onDrawerKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onDrawerClose();
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusable = Array.from(
        sidebarRef.current?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled), summary, [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter((element) => !element.hidden);
      const first = focusable[0];
      const last = focusable.at(-1);
      if (first === undefined || last === undefined) {
        return;
      }
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    window.addEventListener("keydown", onDrawerKeyDown);
    return () => {
      window.clearTimeout(focusTimer);
      window.removeEventListener("keydown", onDrawerKeyDown);
      for (const { element, wasInert } of obscured) {
        if (!wasInert) {
          element.removeAttribute("inert");
        }
      }
    };
  }, [drawerOpen, onDrawerClose]);

  function beginRename(space: SpaceSummary) {
    setRenameSpaceId(space.spaceId);
    setRenameDraft(space.displayName);
  }

  function submitRename() {
    if (renameSpaceId === null || renameDraft.trim() === "") {
      return;
    }
    onRenameSpace(renameSpaceId, renameDraft.trim());
    setRenameSpaceId(null);
    setRenameDraft("");
  }

  function beginThreadRename(run: Run, title: string) {
    setRenameThreadSessionId(run.sessionId);
    setRenameThreadDraft(title);
  }

  function submitThreadRename(run: Run) {
    const name = normalizeThreadName(renameThreadDraft);
    if (name === null) {
      return;
    }
    onRenameThread(run, name);
    setRenameThreadSessionId(null);
    setRenameThreadDraft("");
  }

  function chooseSearchScope(scope: SpaceSearchScope) {
    onSearchScopeChange(scope);
    searchRef.current?.focus();
  }

  function previewSidebarWidth(width: number): number {
    const nextWidth = clampWorkSidebarWidth(width);
    onSidebarWidthPreview(nextWidth);
    return nextWidth;
  }

  function commitSidebarWidth(width: number) {
    const nextWidth = previewSidebarWidth(width);
    setAnnouncedSidebarWidth(nextWidth);
    onSidebarWidthCommit(nextWidth);
  }

  function finishSidebarResize(pointerId: number, handle: HTMLElement) {
    const resize = sidebarResizeRef.current;
    if (resize === null || resize.pointerId !== pointerId) {
      return;
    }
    sidebarResizeRef.current = null;
    if (handle.hasPointerCapture(pointerId)) {
      handle.releasePointerCapture(pointerId);
    }
    commitSidebarWidth(resize.width);
  }

  return (
    <aside
      ref={sidebarRef}
      className={`work-sidebar space-sidebar${drawerOpen ? " is-drawer-open" : ""}`}
      id="work-navigation"
      role={drawerOpen ? "dialog" : undefined}
      aria-modal={drawerOpen ? true : undefined}
      aria-label={drawerOpen ? "Workspace navigation" : "Colossus navigation"}
    >
      <div
        className="sidebar-resize-handle"
        role="separator"
        aria-label="Resize Workspace sidebar"
        aria-orientation="vertical"
        aria-valuemin={MIN_WORK_SIDEBAR_WIDTH}
        aria-valuemax={MAX_WORK_SIDEBAR_WIDTH}
        aria-valuenow={announcedSidebarWidth ?? defaultWorkSidebarWidth()}
        title="Drag to resize. Double-click to reset."
        tabIndex={0}
        onPointerDown={(event) => {
          if (event.button !== 0 || sidebarRef.current === null) {
            return;
          }
          event.preventDefault();
          const startWidth = sidebarRef.current.getBoundingClientRect().width;
          sidebarResizeRef.current = {
            pointerId: event.pointerId,
            startX: event.clientX,
            startWidth,
            width: startWidth,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const resize = sidebarResizeRef.current;
          if (resize === null || resize.pointerId !== event.pointerId) {
            return;
          }
          resize.width = previewSidebarWidth(
            resize.startWidth + event.clientX - resize.startX,
          );
        }}
        onPointerUp={(event) =>
          finishSidebarResize(event.pointerId, event.currentTarget)
        }
        onPointerCancel={(event) =>
          finishSidebarResize(event.pointerId, event.currentTarget)
        }
        onKeyDown={(event) => {
          const currentWidth =
            sidebarRef.current?.getBoundingClientRect().width ??
            announcedSidebarWidth ??
            defaultWorkSidebarWidth();
          const increment = event.shiftKey ? 24 : 8;
          let nextWidth: number | null = null;
          if (event.key === "ArrowLeft") {
            nextWidth = currentWidth - increment;
          } else if (event.key === "ArrowRight") {
            nextWidth = currentWidth + increment;
          } else if (event.key === "Home") {
            nextWidth = MIN_WORK_SIDEBAR_WIDTH;
          } else if (event.key === "End") {
            nextWidth = MAX_WORK_SIDEBAR_WIDTH;
          }
          if (nextWidth !== null) {
            event.preventDefault();
            commitSidebarWidth(nextWidth);
          }
        }}
        onDoubleClick={() => {
          setAnnouncedSidebarWidth(null);
          onSidebarWidthReset();
        }}
      />
      <button
        ref={closeButtonRef}
        className="icon-button compact-drawer-close"
        type="button"
        aria-label="Close navigation"
        onClick={onDrawerClose}
      >
        <IconX size={19} stroke={1.8} aria-hidden="true" />
      </button>

      <section className="space-shelves" aria-labelledby="spaces-heading">
        <div className="space-shelves-heading">
          <span id="spaces-heading">Workspaces</span>
          <details
            className="space-library-menu"
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget)) {
                event.currentTarget.removeAttribute("open");
              }
            }}
          >
            <summary
              aria-label="Manage Workspaces"
              title="Manage Workspaces"
              role="button"
            >
              <IconDots size={16} stroke={1.9} aria-hidden="true" />
            </summary>
            <div className="space-library-popover">
              <button
                type="button"
                disabled={actionsDisabled}
                onClick={(event) => {
                  event.currentTarget
                    .closest("details")
                    ?.removeAttribute("open");
                  onCreateSpace();
                }}
              >
                <IconPlus size={15} stroke={1.8} aria-hidden="true" />
                Add Workspace from folder
              </button>
              {selectedSpace !== undefined ? (
                <>
                  <div className="space-library-separator" />
                  <span className="space-library-label">Current Workspace</span>
                  <button
                    type="button"
                    disabled={actionsDisabled}
                    onClick={(event) => {
                      event.currentTarget
                        .closest("details")
                        ?.removeAttribute("open");
                      beginRename(selectedSpace);
                    }}
                  >
                    <IconPencil size={15} stroke={1.8} aria-hidden="true" />
                    Rename {selectedSpace.displayName}
                  </button>
                  <button
                    type="button"
                    disabled={actionsDisabled}
                    onClick={(event) => {
                      event.currentTarget
                        .closest("details")
                        ?.removeAttribute("open");
                      onArchiveSpace(selectedSpace.spaceId);
                    }}
                  >
                    <IconArchive size={15} stroke={1.8} aria-hidden="true" />
                    Archive {selectedSpace.displayName}
                  </button>
                  {capabilities.tui ? (
                    <button
                      type="button"
                      disabled={
                        actionsDisabled ||
                        !terminalAvailable ||
                        !terminalEnabled
                      }
                      onClick={onOpenTerminal}
                    >
                      <IconTerminal2
                        size={15}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                      Open Colossus TUI
                    </button>
                  ) : null}
                  {capabilities.shellTerminal ? (
                    <button
                      type="button"
                      disabled={
                        actionsDisabled ||
                        !terminalAvailable ||
                        !terminalEnabled
                      }
                      onClick={onOpenShell}
                    >
                      <IconTerminal2
                        size={15}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                      Open local terminal
                    </button>
                  ) : null}
                </>
              ) : null}
              {archivedSpaces.length > 0 ? (
                <>
                  <div className="space-library-separator" />
                  <span className="space-library-label">Archived</span>
                  {archivedSpaces.map((space) => (
                    <button
                      type="button"
                      key={space.spaceId}
                      disabled={actionsDisabled}
                      onClick={() => onRestoreSpace(space.spaceId)}
                    >
                      <IconRestore size={15} stroke={1.8} aria-hidden="true" />
                      Restore {space.displayName}
                    </button>
                  ))}
                </>
              ) : null}
            </div>
          </details>
        </div>

        {spaceActionFeedback !== null ? (
          <p
            className={`space-action-feedback is-${spaceActionFeedback.tone}`}
            role={spaceActionFeedback.tone === "error" ? "alert" : "status"}
          >
            {spaceActionFeedback.tone === "progress" ? (
              <IconLoader2
                className="spin-icon"
                size={14}
                stroke={1.8}
                aria-hidden="true"
              />
            ) : spaceActionFeedback.tone === "success" ? (
              <IconCheck size={14} stroke={2} aria-hidden="true" />
            ) : (
              <IconAlertCircle size={14} stroke={1.9} aria-hidden="true" />
            )}
            <span>{spaceActionFeedback.message}</span>
          </p>
        ) : null}

        <div className="work-search">
          <IconSearch size={17} stroke={1.7} aria-hidden="true" />
          <input
            ref={searchRef}
            type="search"
            aria-label="Search threads"
            value={query}
            placeholder="Search threads"
            onChange={(event) => onQueryChange(event.target.value)}
          />
          <kbd aria-hidden="true">⌘K</kbd>
        </div>
        <div className="search-scope-switcher" aria-label="Thread search scope">
          <button
            type="button"
            aria-pressed={searchScope === "all"}
            onClick={() => chooseSearchScope("all")}
          >
            <IconWorld size={13} stroke={1.8} aria-hidden="true" />
            All Workspaces
          </button>
          <button
            type="button"
            aria-pressed={searchScope === "space"}
            disabled={selectedSpaceId === null}
            onClick={() => chooseSearchScope("space")}
          >
            <IconFolder size={13} stroke={1.8} aria-hidden="true" />
            This Workspace
          </button>
        </div>
        {searchScope === "all" ? (
          <label className="include-archived-search">
            <input
              type="checkbox"
              checked={includeArchived}
              onChange={(event) =>
                onIncludeArchivedChange(event.target.checked)
              }
            />
            Include archived Workspaces and threads
          </label>
        ) : null}
      </section>

      <div className="space-tree-scroll">
        {displayedSpace === undefined ? (
          <button
            className="space-empty-add"
            type="button"
            disabled={actionsDisabled}
            onClick={onCreateSpace}
          >
            <IconPlus size={16} stroke={1.8} aria-hidden="true" />
            Add a Workspace
          </button>
        ) : (
          <div
            className={`space-shelf is-active${spaceStartup !== null ? " is-starting" : ""}`}
            aria-busy={spaceStartup !== null}
          >
            <div className="space-shelf-row">
              <button
                type="button"
                className="space-shelf-identity"
                aria-current="page"
                title={displayedSpace.displayPath}
                onClick={() => setThreadShelfOpen(true)}
              >
                <span className="space-shelf-folder" aria-hidden="true">
                  {spaceStartup === null ? (
                    <IconFolder size={17} stroke={1.7} />
                  ) : (
                    <IconLoader2 className="spin-icon" size={17} stroke={1.8} />
                  )}
                </span>
                <strong>{displayedSpace.displayName}</strong>
              </button>
              <span className="space-shelf-state">
                <i
                  className={`space-health ${
                    spaceStartup !== null
                      ? "space-health-loading"
                      : `space-health-${displayedSpace.state}`
                  }`}
                  aria-hidden="true"
                />
                {spaceStartup !== null
                  ? "Starting"
                  : runtimeLabel(displayedSpace)}
              </span>
              {displayedSpace.attentionCount > 0 ? (
                <span className="space-attention-badge">
                  {Math.min(displayedSpace.attentionCount, 99)}
                </span>
              ) : null}
              <button
                className="space-shelf-chevron"
                type="button"
                aria-label={`${threadShelfOpen ? "Collapse" : "Expand"} ${displayedSpace.displayName} threads`}
                aria-expanded={threadShelfOpen}
                onClick={() => setThreadShelfOpen((open) => !open)}
              >
                <IconChevronDown size={16} stroke={1.8} aria-hidden="true" />
              </button>
              <button
                className="space-compose-action"
                type="button"
                aria-label={`New thread in ${displayedSpace.displayName}`}
                title={`New thread in ${displayedSpace.displayName}`}
                disabled={actionsDisabled || selectedSpace === undefined}
                onClick={onNewWork}
              >
                <IconPencilPlus size={17} stroke={1.8} aria-hidden="true" />
              </button>
            </div>
            {renameSpaceId === displayedSpace.spaceId ? (
              <form
                className="space-rename-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  submitRename();
                }}
              >
                <label>
                  <span className="sr-only">Workspace name</span>
                  <input
                    autoFocus
                    maxLength={80}
                    value={renameDraft}
                    onChange={(event) => setRenameDraft(event.target.value)}
                  />
                </label>
                <button className="text-button" type="submit">
                  Save
                </button>
                <button
                  className="text-button"
                  type="button"
                  onClick={() => setRenameSpaceId(null)}
                >
                  Cancel
                </button>
              </form>
            ) : null}
            {spaceStartup !== null ? (
              <span className="sr-only" role="status" aria-live="polite">
                Starting the local agent for {spaceStartup.displayName}. Search
                and navigation remain available.
              </span>
            ) : null}
          </div>
        )}
        <div
          className={`space-thread-stack${threadShelfOpen ? "" : " is-collapsed"}`}
          aria-hidden={!threadShelfOpen}
        >
          <nav
            className="work-list"
            aria-label="Threads"
            aria-busy={busy || searchBusy}
          >
            {searching
              ? searchResults.map((result) => {
                  const status = presentRunStatus(result.status);
                  const restoring =
                    threadLifecycleBusySessionId === result.sessionId;
                  const title = resolveThreadTitle(
                    result.spaceId,
                    result.sessionId,
                    result.title,
                  );
                  return (
                    <div
                      className="work-item-row"
                      key={`${result.spaceId}:${result.runId}`}
                    >
                      <button
                        className="work-item search-result-item"
                        type="button"
                        disabled={actionsDisabled || result.threadArchived}
                        onClick={() => onSelectSearchResult(result)}
                      >
                        <span className="work-item-copy">
                          <strong>{title}</strong>
                          <span>
                            {result.spaceName} · {runModeLabel(result.mode)} ·{" "}
                            {result.threadArchived
                              ? "Archived"
                              : shortDateLabel(result.updatedAt)}
                          </span>
                        </span>
                        <span
                          className={`work-item-state tone-${searchResultTone(result)}`}
                          title={status.copy}
                        >
                          {statusIcon(status.tone)}
                          <span className="sr-only">{status.label}</span>
                        </span>
                      </button>
                      {result.threadArchived ? (
                        <button
                          className="work-item-action"
                          type="button"
                          aria-label={`Restore ${title}`}
                          title="Restore thread"
                          disabled={actionsDisabled || restoring}
                          onClick={() => onRestoreThread(result)}
                        >
                          {restoring ? (
                            <IconLoader2 className="spin-icon" size={15} />
                          ) : (
                            <IconRestore size={15} stroke={1.8} />
                          )}
                        </button>
                      ) : null}
                    </div>
                  );
                })
              : groups.map((group) => (
                  <section className="work-group" key={group.key}>
                    <div className="work-group-heading">
                      <h2>
                        {group.key === "pinned" ? (
                          <IconPin size={12} stroke={1.8} aria-hidden="true" />
                        ) : group.key === "attention" ? (
                          <IconAlertCircle
                            size={12}
                            stroke={1.8}
                            aria-hidden="true"
                          />
                        ) : group.key === "active" ? (
                          <IconPointFilled
                            size={12}
                            stroke={1.8}
                            aria-hidden="true"
                          />
                        ) : (
                          <IconActivity
                            size={12}
                            stroke={1.8}
                            aria-hidden="true"
                          />
                        )}
                        {group.label}
                      </h2>
                      <span>{group.items.length}</span>
                    </div>
                    <div className="work-group-items">
                      {(group.key === "recent" && !showAllRecent
                        ? group.items.slice(0, 3)
                        : group.items
                      ).map((item) => {
                        const run = runsById.get(item.runId);
                        if (run === undefined) {
                          return null;
                        }
                        const pinned = pinnedSessionIds.has(run.sessionId);
                        const archiving =
                          threadLifecycleBusySessionId === run.sessionId;
                        const renaming =
                          renameThreadSessionId === run.sessionId;
                        return (
                          <div
                            className={`work-item-row${
                              activeSessionId === run.sessionId
                                ? " is-current"
                                : ""
                            }`}
                            key={item.runId}
                          >
                            {renaming ? (
                              <form
                                className="thread-rename-form"
                                onSubmit={(event) => {
                                  event.preventDefault();
                                  submitThreadRename(run);
                                }}
                              >
                                <label>
                                  <span className="sr-only">Thread name</span>
                                  <input
                                    autoFocus
                                    required
                                    maxLength={MAX_THREAD_NAME_CHARACTERS}
                                    value={renameThreadDraft}
                                    aria-label="Thread name"
                                    onChange={(event) =>
                                      setRenameThreadDraft(event.target.value)
                                    }
                                    onKeyDown={(event) => {
                                      if (event.key === "Escape") {
                                        event.preventDefault();
                                        setRenameThreadSessionId(null);
                                        setRenameThreadDraft("");
                                      }
                                    }}
                                  />
                                </label>
                                <button
                                  className="thread-rename-action"
                                  type="button"
                                  aria-label="Cancel thread rename"
                                  title="Cancel"
                                  onClick={() => {
                                    setRenameThreadSessionId(null);
                                    setRenameThreadDraft("");
                                  }}
                                >
                                  <IconX
                                    size={14}
                                    stroke={1.8}
                                    aria-hidden="true"
                                  />
                                </button>
                                <button
                                  className="thread-rename-action is-save"
                                  type="submit"
                                  aria-label="Save thread name"
                                  title="Save"
                                >
                                  <IconCheck
                                    size={14}
                                    stroke={2}
                                    aria-hidden="true"
                                  />
                                </button>
                              </form>
                            ) : (
                              <>
                                <button
                                  className="work-item"
                                  type="button"
                                  aria-current={
                                    activeSessionId === run.sessionId
                                      ? "page"
                                      : undefined
                                  }
                                  disabled={actionsDisabled}
                                  onClick={() => onSelect(run)}
                                >
                                  <span className="work-item-copy">
                                    <strong>{item.title}</strong>
                                    <span>
                                      {item.modeLabel} · {item.updatedLabel}
                                    </span>
                                  </span>
                                  <span
                                    className={`work-item-state tone-${item.statusTone}`}
                                    title={item.statusCopy}
                                  >
                                    {statusIcon(item.statusTone)}
                                    <span className="sr-only">
                                      {item.statusLabel}
                                    </span>
                                  </span>
                                </button>
                                <details
                                  className="thread-actions-menu"
                                  onBlur={(event) => {
                                    const menu = event.currentTarget;
                                    window.requestAnimationFrame(() => {
                                      if (
                                        !menu.contains(document.activeElement)
                                      ) {
                                        menu.removeAttribute("open");
                                      }
                                    });
                                  }}
                                  onKeyDown={(event) => {
                                    if (event.key === "Escape") {
                                      event.preventDefault();
                                      event.currentTarget.removeAttribute(
                                        "open",
                                      );
                                      event.currentTarget
                                        .querySelector("summary")
                                        ?.focus();
                                    }
                                  }}
                                >
                                  <summary
                                    className="work-item-action"
                                    role="button"
                                    aria-haspopup="menu"
                                    aria-label={`Thread actions for ${item.title}`}
                                    title="Thread actions"
                                  >
                                    <IconDots
                                      size={17}
                                      stroke={1.9}
                                      aria-hidden="true"
                                    />
                                  </summary>
                                  <div
                                    className="thread-actions-popover"
                                    aria-label={`Actions for ${item.title}`}
                                  >
                                    <button
                                      type="button"
                                      aria-label={`Rename ${item.title}`}
                                      disabled={spaceStartup !== null}
                                      onClick={(event) => {
                                        event.currentTarget
                                          .closest("details")
                                          ?.removeAttribute("open");
                                        beginThreadRename(run, item.title);
                                      }}
                                    >
                                      <IconPencil
                                        size={15}
                                        stroke={1.8}
                                        aria-hidden="true"
                                      />
                                      Rename
                                    </button>
                                    <button
                                      type="button"
                                      aria-label={`${pinned ? "Unpin" : "Pin"} ${item.title}`}
                                      aria-pressed={pinned}
                                      disabled={spaceStartup !== null}
                                      onClick={(event) => {
                                        event.currentTarget
                                          .closest("details")
                                          ?.removeAttribute("open");
                                        onToggleThreadPinned(run);
                                      }}
                                    >
                                      <IconPin
                                        size={15}
                                        stroke={1.8}
                                        fill={pinned ? "currentColor" : "none"}
                                        aria-hidden="true"
                                      />
                                      {pinned ? "Unpin" : "Pin"}
                                    </button>
                                    <button
                                      type="button"
                                      aria-label={`Archive ${item.title}`}
                                      title={
                                        isTerminalStatus(run.status)
                                          ? "Archive thread"
                                          : "Finish or cancel this thread before archiving"
                                      }
                                      disabled={
                                        actionsDisabled ||
                                        archiving ||
                                        !isTerminalStatus(run.status)
                                      }
                                      onClick={(event) => {
                                        event.currentTarget
                                          .closest("details")
                                          ?.removeAttribute("open");
                                        onArchiveThread(run);
                                      }}
                                    >
                                      {archiving ? (
                                        <IconLoader2
                                          className="spin-icon"
                                          size={15}
                                        />
                                      ) : (
                                        <IconArchive
                                          size={15}
                                          stroke={1.8}
                                          aria-hidden="true"
                                        />
                                      )}
                                      {archiving ? "Archiving…" : "Archive"}
                                    </button>
                                  </div>
                                </details>
                              </>
                            )}
                          </div>
                        );
                      })}
                    </div>
                    {group.key === "recent" &&
                    (group.items.length > 3 || hasMore) ? (
                      <button
                        className="work-group-more"
                        type="button"
                        disabled={busy || actionsDisabled}
                        onClick={() => {
                          if (!showAllRecent && group.items.length > 3) {
                            setShowAllRecent(true);
                            return;
                          }
                          if (hasMore) {
                            setShowAllRecent(true);
                            onLoadMore();
                            return;
                          }
                          setShowAllRecent(false);
                        }}
                      >
                        {!showAllRecent && group.items.length > 3
                          ? `View ${group.items.length - 3} more recent ${group.items.length - 3 === 1 ? "thread" : "threads"}`
                          : hasMore
                            ? "Load older threads"
                            : "Show fewer"}
                      </button>
                    ) : null}
                  </section>
                ))}
            {!busy &&
            !searchBusy &&
            (searching ? searchResults.length === 0 : groups.length === 0) ? (
              <p className="work-list-empty">
                {searching
                  ? "No threads match this search."
                  : "Threads in this Workspace will appear here."}
              </p>
            ) : null}
          </nav>

          <div className="work-list-footer">
            {busy || searchBusy ? (
              <span className="loading-copy" role="status">
                <IconLoader2
                  className="spin-icon"
                  size={15}
                  aria-hidden="true"
                />
                {searching ? "Searching" : "Loading threads"}
              </span>
            ) : null}
            {searching && searchHasMore ? (
              <button
                className="text-button"
                type="button"
                disabled={searchBusy || actionsDisabled}
                onClick={onLoadMoreSearch}
              >
                More results
              </button>
            ) : null}
            {!searching && hasMore && !hasRecentGroup ? (
              <button
                className="text-button"
                type="button"
                disabled={busy || actionsDisabled}
                onClick={onLoadMore}
              >
                Load older threads
              </button>
            ) : null}
            {(searching ? searchError : error) !== "" ? (
              <p className="sidebar-error" role="alert">
                {searching ? searchError : error}
              </p>
            ) : null}
          </div>
        </div>

        {otherSpaces.length > 0 ? (
          <nav className="space-shelf-list" aria-label="Other Workspaces">
            {otherSpaces.map((space) => {
              const starting = spaceStartup?.spaceId === space.spaceId;
              const expanded = expandedSpaceIds.has(space.spaceId);
              const previewItems = spaceThreadPreviews.get(space.spaceId);
              const previewBusy = spaceThreadPreviewBusyIds.has(space.spaceId);
              const previewError = spaceThreadPreviewErrors.get(space.spaceId);
              const previewId = `space-thread-preview-${space.spaceId}`;
              return (
                <div
                  className={`space-shelf${expanded ? " is-expanded" : ""}`}
                  key={space.spaceId}
                >
                  <div className="space-shelf-row">
                    <button
                      type="button"
                      className="space-shelf-identity"
                      aria-current={space.selected ? "true" : undefined}
                      aria-busy={starting}
                      disabled={actionsDisabled || space.selected}
                      title={space.displayPath}
                      onClick={() => onSelectSpace(space.spaceId)}
                    >
                      <span className="space-shelf-folder" aria-hidden="true">
                        {starting ? (
                          <IconLoader2
                            className="spin-icon"
                            size={17}
                            stroke={1.8}
                          />
                        ) : (
                          <IconFolder size={17} stroke={1.7} />
                        )}
                      </span>
                      <strong>{space.displayName}</strong>
                    </button>
                    <span className="space-shelf-state">
                      <i
                        className={`space-health ${
                          starting
                            ? "space-health-loading"
                            : `space-health-${space.state}`
                        }`}
                        aria-hidden="true"
                      />
                      {starting ? "Starting" : runtimeLabel(space)}
                    </span>
                    {space.attentionCount > 0 ? (
                      <span className="space-attention-badge">
                        {Math.min(space.attentionCount, 99)}
                      </span>
                    ) : null}
                    <button
                      className="space-shelf-chevron"
                      type="button"
                      aria-label={`${expanded ? "Collapse" : "Expand"} ${space.displayName} threads`}
                      aria-controls={previewId}
                      aria-expanded={expanded}
                      onClick={() => toggleSpacePreview(space.spaceId)}
                    >
                      <IconChevronDown
                        size={16}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                    </button>
                  </div>
                  <div
                    className="space-thread-preview"
                    id={previewId}
                    hidden={!expanded}
                    aria-busy={previewBusy}
                  >
                    {previewBusy ? (
                      <span
                        className="space-thread-preview-message"
                        role="status"
                      >
                        <IconLoader2
                          className="spin-icon"
                          size={13}
                          aria-hidden="true"
                        />
                        Loading threads
                      </span>
                    ) : previewError !== undefined ? (
                      <span className="space-thread-preview-message is-error">
                        {previewError}
                      </span>
                    ) : previewItems !== undefined &&
                      previewItems.length > 0 ? (
                      previewItems.map((result) => {
                        const status = presentRunStatus(result.status);
                        const title = resolveThreadTitle(
                          result.spaceId,
                          result.sessionId,
                          result.title,
                        );
                        return (
                          <button
                            className="space-thread-preview-item"
                            type="button"
                            key={`${result.spaceId}:${result.runId}`}
                            aria-label={`Open ${title} in ${space.displayName}`}
                            disabled={actionsDisabled || result.threadArchived}
                            onClick={() => onSelectSearchResult(result)}
                          >
                            <span
                              className={`space-thread-preview-status tone-${status.tone}`}
                              title={status.copy}
                            >
                              {statusIcon(status.tone)}
                            </span>
                            <span className="space-thread-preview-copy">
                              <strong>{title}</strong>
                              <small>
                                {runModeLabel(result.mode)} ·{" "}
                                {shortDateLabel(result.updatedAt)}
                              </small>
                            </span>
                          </button>
                        );
                      })
                    ) : previewItems !== undefined ? (
                      <span className="space-thread-preview-message">
                        No recent threads
                      </span>
                    ) : null}
                  </div>
                </div>
              );
            })}
          </nav>
        ) : null}
      </div>

      <nav className="space-destinations" aria-label="Workspace destinations">
        {visibleDestinations.map(({ id, label, Icon }) => (
          <button
            type="button"
            key={id}
            aria-label={label}
            aria-current={surface === id ? "page" : undefined}
            onClick={() => onSelectSurface(id)}
          >
            <Icon size={17} stroke={1.7} aria-hidden="true" />
            <span>{label}</span>
            {id === "work" &&
            selectedSpace !== undefined &&
            selectedSpace.attentionCount > 0 ? (
              <span className="space-attention-badge">
                {Math.min(selectedSpace.attentionCount, 99)}
              </span>
            ) : null}
          </button>
        ))}
      </nav>

      <footer className="space-sidebar-footer">
        <span
          className={`connection-dot connection-${connectionState}`}
          aria-hidden="true"
        />
        {connectionState === "connected" ? "Agent online" : "Agent offline"}
      </footer>
    </aside>
  );
}
