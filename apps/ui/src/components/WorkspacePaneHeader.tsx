import {
  useId,
  useLayoutEffect,
  useRef,
  type ReactNode,
  type Ref,
} from "react";
import {
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconX,
} from "@tabler/icons-react";
import { Button } from "./ui/button.js";

export interface WorkspacePaneTab {
  id: string;
  label: string;
}

/** Presentation only: hosts own opened tools, selection, closing and expansion. */
export function WorkspacePaneHeader({
  tabs,
  activeTab,
  onTabChange,
  onTabClose,
  expanded = false,
  onToggleExpanded,
  onClose,
  closeRef,
  actions,
  title = "Workspace",
  panelId,
  expandLabel = "Expand right pane",
  restoreLabel = "Restore right pane",
  closeLabel = "Close right pane",
}: {
  tabs: readonly WorkspacePaneTab[];
  activeTab: string;
  onTabChange: (id: string) => void;
  onTabClose?: ((id: string) => void) | undefined;
  expanded?: boolean;
  onToggleExpanded?: (() => void) | undefined;
  onClose?: (() => void) | undefined;
  closeRef?: Ref<HTMLButtonElement>;
  actions?: ReactNode;
  title?: string;
  panelId?: string;
  expandLabel?: string;
  restoreLabel?: string;
  closeLabel?: string;
}) {
  const id = useId();
  const root = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const strip = root.current?.querySelector(".shared-workspace-pane-tabs");
    const revealSelection = () =>
      root.current
        ?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
    revealSelection();
    let frame = 0;
    const resize = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(revealSelection);
    });
    if (strip) resize.observe(strip);
    return () => {
      cancelAnimationFrame(frame);
      resize.disconnect();
    };
  }, [activeTab, tabs.length, expanded]);
  const select = (next: string) => {
    onTabChange(next);
    [
      ...(root.current?.querySelectorAll<HTMLButtonElement>(
        "[data-workspace-tab]",
      ) ?? []),
    ]
      .find((button) => button.dataset.workspaceTab === next)
      ?.focus();
  };
  return (
    <header className="shared-workspace-pane-header" ref={root}>
      <div className="shared-workspace-pane-tabs">
        <div
          className="shared-workspace-pane-tablist"
          role="tablist"
          aria-label="Workspace tabs"
          onKeyDown={(event) => {
            if (
              !(event.target instanceof HTMLElement) ||
              event.target.getAttribute("role") !== "tab"
            )
              return;
            const current = tabs.findIndex((tab) => tab.id === activeTab);
            let next = current;
            if (event.key === "ArrowRight") next = (current + 1) % tabs.length;
            else if (event.key === "ArrowLeft")
              next = (current + tabs.length - 1) % tabs.length;
            else if (event.key === "Home") next = 0;
            else if (event.key === "End") next = tabs.length - 1;
            else if (event.key === "Delete" && onTabClose) {
              event.preventDefault();
              onTabClose(activeTab);
              requestAnimationFrame(() =>
                root.current
                  ?.querySelector<HTMLButtonElement>(
                    '[role="tab"][aria-selected="true"]',
                  )
                  ?.focus(),
              );
              return;
            } else return;
            event.preventDefault();
            const nextTab = tabs[next];
            if (nextTab) select(nextTab.id);
          }}
        >
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              className={`shared-workspace-pane-tab${activeTab === tab.id ? " is-active" : ""}${onTabClose ? " has-close" : ""}`}
              style={{ order: index * 2 }}
              id={`${id}-tab-${index}`}
              type="button"
              role="tab"
              aria-selected={activeTab === tab.id}
              aria-controls={panelId}
              tabIndex={activeTab === tab.id ? 0 : -1}
              data-workspace-tab={tab.id}
              title={tab.label}
              onClick={() => onTabChange(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        {onTabClose
          ? tabs.map((tab, index) => (
              <button
                key={tab.id}
                type="button"
                className={`shared-workspace-tab-close${activeTab === tab.id ? " is-active" : ""}`}
                style={{ order: index * 2 + 1 }}
                aria-label={`Close ${tab.label} tab`}
                onClick={() => {
                  onTabClose(tab.id);
                  requestAnimationFrame(() =>
                    root.current
                      ?.querySelector<HTMLButtonElement>(
                        '[role="tab"][aria-selected="true"]',
                      )
                      ?.focus(),
                  );
                }}
              >
                <span>
                  <IconX size={12} aria-hidden="true" />
                </span>
              </button>
            ))
          : null}
      </div>
      <span className="shared-sr-only">{title}</span>
      <div className="shared-workspace-pane-actions">
        {actions}
        {onToggleExpanded ? (
          <Button
            variant="ghost"
            size="icon"
            aria-label={expanded ? restoreLabel : expandLabel}
            aria-pressed={expanded}
            onClick={onToggleExpanded}
          >
            {expanded ? (
              <IconArrowsMinimize size={16} aria-hidden="true" />
            ) : (
              <IconArrowsMaximize size={16} aria-hidden="true" />
            )}
          </Button>
        ) : null}
        {onClose ? (
          <Button
            ref={closeRef}
            variant="ghost"
            size="icon"
            aria-label={closeLabel}
            onClick={onClose}
          >
            <IconX size={17} aria-hidden="true" />
          </Button>
        ) : null}
      </div>
    </header>
  );
}
