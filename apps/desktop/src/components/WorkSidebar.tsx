import {
  IconAlertCircle,
  IconCheck,
  IconFolder,
  IconLoader2,
  IconPointFilled,
  IconPlus,
  IconSearch,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useMemo, useRef } from "react";

import { selectRecentWork } from "../presenters";
import type { Run, WorkspaceSummary } from "../types";

interface WorkSidebarProps {
  runs: readonly Run[];
  workspace: WorkspaceSummary | null;
  activeSessionId: string | null;
  query: string;
  busy: boolean;
  error: string;
  hasMore: boolean;
  disabled: boolean;
  drawerOpen: boolean;
  onQueryChange: (query: string) => void;
  onNewWork: () => void;
  onSelect: (run: Run) => void;
  onLoadMore: () => void;
  onDrawerOpen: () => void;
  onDrawerClose: () => void;
}

function statusIcon(tone: string) {
  if (tone === "success") {
    return <IconCheck size={13} stroke={2.2} aria-hidden="true" />;
  }
  if (tone === "danger" || tone === "attention") {
    return <IconAlertCircle size={13} stroke={2} aria-hidden="true" />;
  }
  return <IconPointFilled size={12} stroke={2} aria-hidden="true" />;
}

export function WorkSidebar({
  runs,
  workspace,
  activeSessionId,
  query,
  busy,
  error,
  hasMore,
  disabled,
  drawerOpen,
  onQueryChange,
  onNewWork,
  onSelect,
  onLoadMore,
  onDrawerOpen,
  onDrawerClose,
}: WorkSidebarProps) {
  const sidebarRef = useRef<HTMLElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const focusSearchWhenOpened = useRef(false);
  const groups = useMemo(
    () => selectRecentWork(runs, { query }),
    [query, runs],
  );
  const runsById = useMemo(
    () => new Map(runs.map((run) => [run.runId, run])),
    [runs],
  );

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
          searchRef.current?.focus();
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

    const obscured = [
      document.querySelector<HTMLElement>(".product-rail"),
      document.querySelector<HTMLElement>("#primary-workspace"),
    ].flatMap((element) =>
      element === null
        ? []
        : [{ element, wasInert: element.hasAttribute("inert") }],
    );
    for (const { element } of obscured) {
      element.setAttribute("inert", "");
    }

    const focusTimer = window.setTimeout(() => {
      if (focusSearchWhenOpened.current) {
        focusSearchWhenOpened.current = false;
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
          'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])',
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

  return (
    <aside
      ref={sidebarRef}
      className={`work-sidebar${drawerOpen ? " is-drawer-open" : ""}`}
      id="work-navigation"
      role={drawerOpen ? "dialog" : undefined}
      aria-modal={drawerOpen ? true : undefined}
      aria-label={drawerOpen ? "Work navigation" : "Work"}
    >
      <button
        ref={closeButtonRef}
        className="icon-button compact-drawer-close"
        type="button"
        aria-label="Close work navigation"
        onClick={onDrawerClose}
      >
        <IconX size={19} stroke={1.8} aria-hidden="true" />
      </button>
      <div className="work-sidebar-header">
        <p>Colossus Operations Studio</p>
        <h1>Work</h1>
        {workspace !== null ? (
          <div className="work-workspace" title={workspace.displayPath}>
            <IconFolder size={15} stroke={1.7} aria-hidden="true" />
            <span>
              <small>Workspace</small>
              <strong>{workspace.displayName}</strong>
            </span>
          </div>
        ) : null}
      </div>
      <button
        className="button primary new-work"
        type="button"
        disabled={disabled}
        onClick={onNewWork}
      >
        <IconPlus size={18} stroke={1.8} aria-hidden="true" />
        New work
      </button>
      <label className="work-search">
        <span className="sr-only">Search work</span>
        <IconSearch size={17} stroke={1.7} aria-hidden="true" />
        <input
          ref={searchRef}
          type="search"
          value={query}
          placeholder="Search work"
          onChange={(event) => onQueryChange(event.target.value)}
        />
        <kbd aria-hidden="true">⌘K</kbd>
      </label>

      <nav className="work-list" aria-label="Recent work" aria-busy={busy}>
        {groups.map((group) => (
          <section className="work-group" key={group.key}>
            <div className="work-group-heading">
              <h2>{group.label}</h2>
              <span>{group.items.length}</span>
            </div>
            <div className="work-group-items">
              {group.items.map((item) => {
                const run = runsById.get(item.runId);
                if (run === undefined) {
                  return null;
                }
                return (
                  <button
                    className="work-item"
                    type="button"
                    key={item.runId}
                    aria-current={
                      activeSessionId === run.sessionId ? "page" : undefined
                    }
                    disabled={disabled}
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
                      <span className="sr-only">{item.statusLabel}</span>
                    </span>
                  </button>
                );
              })}
            </div>
          </section>
        ))}
        {!busy && groups.length === 0 ? (
          <p className="work-list-empty">
            {query === ""
              ? "Your Colossus work will appear here."
              : "No work matches this search."}
          </p>
        ) : null}
      </nav>

      <div className="work-list-footer">
        {busy ? (
          <span className="loading-copy" role="status">
            <IconLoader2 className="spin-icon" size={15} aria-hidden="true" />
            Loading work
          </span>
        ) : null}
        {hasMore ? (
          <button
            className="text-button"
            type="button"
            disabled={busy}
            onClick={onLoadMore}
          >
            Load more
          </button>
        ) : null}
        {error !== "" ? (
          <p className="sidebar-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </aside>
  );
}
