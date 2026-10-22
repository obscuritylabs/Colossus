import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { usePanelRef } from "react-resizable-panels";
import { useWorkspaceComposerAlignment } from "../hooks/useWorkspaceComposerAlignment.js";
import {
  WorkspacePaneHeader,
  type WorkspacePaneTab,
} from "./WorkspacePaneHeader.js";
import { createPortal } from "react-dom";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./ui/resizable.js";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetTitle,
} from "./ui/sheet.js";

/** The host owns selection and content. The main surface stays mounted in every layout. */
export function InspectorLayout({
  children,
  inspector,
  title,
  onClose,
  open,
  tabs,
  activeTab,
  onTabChange,
  onTabClose,
}: {
  children: ReactNode;
  inspector: ReactNode;
  title: string;
  onClose: () => void;
  open: boolean;
  tabs?: readonly WorkspacePaneTab[];
  activeTab?: string | undefined;
  onTabChange?: (id: string) => void;
  onTabClose?: (id: string) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const parked = useRef<HTMLDivElement>(null);
  const paneHeading = useRef<HTMLHeadingElement>(null);
  const [contentRoot] = useState(() => {
    const element = document.createElement("div");
    element.className = "shared-inspector-portal";
    return element;
  });
  const origin = useRef<HTMLElement | null>(null);
  const wasOpen = useRef(false);
  const [width, setWidth] = useState(0);
  const paneWidth = useRef(360);
  const resizing = useRef(false);
  const mainPanel = usePanelRef();
  const inspectorPanel = usePanelRef();
  const [expanded, setExpanded] = useState(false);
  const heading = useId();
  const wide = width >= 690;
  const paneTabs = tabs ?? [{ id: "inspector", label: title }];
  const selectedTab = activeTab ?? paneTabs[0]?.id ?? "inspector";
  useWorkspaceComposerAlignment(root, wide && open && !expanded);
  useEffect(() => {
    // Conditional panels register with their group after mounting. Resize only
    // after that registration has committed, including split/Sheet transitions.
    const frame = requestAnimationFrame(() => {
      if (wide && open) {
        if (expanded) mainPanel.current?.resize(0);
        else
          inspectorPanel.current?.resize(
            Math.min(paneWidth.current, width - 390),
          );
      }
      resizing.current = false;
    });
    return () => cancelAnimationFrame(frame);
  }, [expanded, wide, open, mainPanel, inspectorPanel]);
  useEffect(() => {
    if (!wide || !open || !expanded) return;
    const frame = requestAnimationFrame(() => mainPanel.current?.resize(0));
    return () => cancelAnimationFrame(frame);
  }, [width, wide, open, expanded, mainPanel]);
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const next = element.getBoundingClientRect().width;
        setWidth((current) => (current === next ? current : next));
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);
  useEffect(() => {
    if (open && !wasOpen.current) {
      origin.current = document.activeElement as HTMLElement | null;
      if (wide) {
        const tab = root.current?.querySelector<HTMLButtonElement>(
          '.shared-inspector [role="tab"][aria-selected="true"]',
        );
        if (tab) tab.focus();
        else paneHeading.current?.focus();
      }
    }
    if (!open && parked.current && contentRoot.parentElement !== parked.current)
      parked.current.appendChild(contentRoot);
    if (!open && wasOpen.current && origin.current?.isConnected)
      origin.current.focus();
    wasOpen.current = open;
  }, [open, wide, contentRoot]);
  const close = () => {
    setExpanded(false);
    onClose();
  };
  const paneHeader = (
    <WorkspacePaneHeader
      tabs={paneTabs}
      activeTab={selectedTab}
      onTabChange={onTabChange ?? (() => {})}
      onTabClose={onTabClose}
      title={title}
      expanded={expanded}
      onToggleExpanded={() => {
        resizing.current = true;
        setExpanded((value) => !value);
      }}
      onClose={close}
      closeLabel={`Close ${title.toLowerCase()}`}
      panelId={`${heading}-${wide ? "wide" : "sheet"}-panel`}
    />
  );
  return (
    <div
      className={`shared-inspector-layout${expanded && open ? " is-expanded" : ""}`}
      ref={root}
      onKeyDown={(event) => {
        if (
          wide &&
          open &&
          event.key === "Escape" &&
          !event.defaultPrevented &&
          contentRoot.contains(event.target as Node)
        ) {
          event.preventDefault();
          close();
        }
      }}
    >
      <ResizablePanelGroup orientation="horizontal">
        <ResizablePanel
          id={`${heading}-main`}
          panelRef={mainPanel}
          minSize={wide && open && !expanded ? 390 : 0}
          aria-hidden={wide && open && expanded ? true : undefined}
          inert={wide && open && expanded}
        >
          <div className="shared-inspector-main">{children}</div>
        </ResizablePanel>
        {wide && open ? (
          <>
            <ResizableHandle aria-label="Resize inspector" hidden={expanded} />
            <ResizablePanel
              id={`${heading}-inspector`}
              panelRef={inspectorPanel}
              defaultSize={Math.min(paneWidth.current, width - 390)}
              minSize={280}
              maxSize={expanded ? width : width - 390}
              groupResizeBehavior="preserve-pixel-size"
              onResize={(size) => {
                if (!expanded && !resizing.current)
                  paneWidth.current = size.inPixels;
              }}
            >
              <aside
                className="shared-inspector"
                aria-labelledby={heading}
                onKeyDown={(event) => {
                  if (event.key === "Escape" && !event.defaultPrevented) {
                    event.preventDefault();
                    close();
                  }
                }}
              >
                <div className="shared-inspector-header">
                  <h2
                    id={heading}
                    ref={paneHeading}
                    className="shared-sr-only"
                    tabIndex={-1}
                  >
                    {title}
                  </h2>
                  {paneHeader}
                </div>
                <div
                  className="shared-inspector-content"
                  id={`${heading}-wide-panel`}
                  role="tabpanel"
                  aria-label={title}
                  ref={(element) => {
                    if (element && contentRoot.parentElement !== element)
                      element.appendChild(contentRoot);
                  }}
                />
              </aside>
            </ResizablePanel>
          </>
        ) : null}
      </ResizablePanelGroup>
      <Sheet
        open={!wide && open}
        onOpenChange={(next) => {
          if (!next) close();
        }}
      >
        <SheetContent
          side="right"
          className={`shared-inspector-sheet${expanded ? " is-expanded" : ""}`}
          showCloseButton={false}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (open) {
              if (wide)
                requestAnimationFrame(() =>
                  root.current
                    ?.querySelector<HTMLButtonElement>(
                      '.shared-inspector [role="tab"][aria-selected="true"]',
                    )
                    ?.focus(),
                );
              return;
            }
            if (origin.current?.isConnected) origin.current.focus();
          }}
        >
          <div className="shared-inspector-header">
            <SheetTitle className="shared-sr-only">{title}</SheetTitle>
            <SheetDescription className="shared-sr-only">
              Inspect alongside your conversation.
            </SheetDescription>
            {paneHeader}
          </div>
          <div
            className="shared-inspector-content"
            id={`${heading}-sheet-panel`}
            role="tabpanel"
            aria-label={title}
            ref={(element) => {
              if (
                element &&
                !wide &&
                open &&
                contentRoot.parentElement !== element
              )
                element.appendChild(contentRoot);
            }}
          />
        </SheetContent>
      </Sheet>
      <div hidden ref={parked} />
      {createPortal(inspector, contentRoot)}
    </div>
  );
}
