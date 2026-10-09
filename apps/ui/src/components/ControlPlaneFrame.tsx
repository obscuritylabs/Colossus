import { useEffect, useRef, type ReactNode } from "react";
import type { PanelImperativeHandle } from "react-resizable-panels";
import colossusMark from "../../assets/colossus-mark.svg";
import {
  Sidebar,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarTrigger,
  useSidebar,
} from "./ui/sidebar.js";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "./ui/resizable.js";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "./ui/sheet.js";
import { TooltipProvider } from "./ui/tooltip.js";

export interface ControlPlaneNavigation {
  id: string;
  label: string;
  shortLabel?: string;
  href?: string;
  icon: ReactNode;
  disabled?: boolean;
}
export interface ClassificationBanner {
  enabled: boolean;
  text: string;
  tone: "neutral" | "info" | "warning" | "danger";
  position: "top" | "top_and_bottom";
}
interface FrameProps {
  current: string;
  navigation: ControlPlaneNavigation[];
  onNavigate: (id: string) => void;
  sidebar?: ReactNode;
  header?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  classification?: ClassificationBanner | undefined;
  user?: ReactNode;
  activeScopeKey?: string;
}

/** Owned shadcn presentation; hosts retain routes, records and authority. */
export function ControlPlaneFrame(props: FrameProps) {
  const classification = props.classification;
  const banner = classification?.enabled ? (
    <div
      className={`control-classification classification-${classification.tone}`}
      role="note"
      aria-label="Environment classification"
    >
      {classification.text}
    </div>
  ) : null;
  return (
    <div
      className={`control-plane-frame ${props.sidebar ? "has-scope-sidebar" : ""}`}
    >
      <a href="#control-main" className="application-skip-link">
        Skip to content
      </a>
      {banner}
      <TooltipProvider delayDuration={300}>
        <SidebarProvider className="control-frame-layout">
          <ControlWorkbench {...props} />
        </SidebarProvider>
      </TooltipProvider>
      {classification?.enabled && classification.position === "top_and_bottom"
        ? banner
        : null}
    </div>
  );
}
function ControlWorkbench({
  current,
  navigation,
  onNavigate,
  sidebar,
  header,
  footer,
  children,
  user,
  activeScopeKey,
}: FrameProps) {
  const { open, setOpen, isMobile, openMobile, setOpenMobile } = useSidebar();
  const panel = useRef<PanelImperativeHandle | null>(null);
  useEffect(() => {
    if (!isMobile && sidebar) {
      if (open) panel.current?.expand();
      else panel.current?.collapse();
    }
  }, [open, isMobile, Boolean(sidebar)]);
  useEffect(() => {
    setOpenMobile(false);
  }, [activeScopeKey, setOpenMobile]);
  const workArea = (
    <SidebarInset
      className="control-frame-main"
      id="control-main"
      tabIndex={-1}
    >
      <header className="control-header">
        <div className="control-header-context">
          {sidebar ? (
            <SidebarTrigger
              aria-label="Toggle workspace sidebar"
              title="Toggle workspace sidebar (⌘/Ctrl B)"
              onClick={(event) => {
                if (!isMobile && panel.current) {
                  event.preventDefault();
                  setOpen(panel.current.isCollapsed());
                }
              }}
            />
          ) : null}
          <strong>
            {sidebar
              ? "Workspaces"
              : (navigation.find((item) => item.id === current)?.label ??
                "Control Plane")}
          </strong>
        </div>
        <div>{header}</div>
      </header>
      <div className="control-content">{children}</div>
    </SidebarInset>
  );
  return (
    <>
      <aside
        className="control-primary-rail"
        aria-label="Control Plane navigation"
      >
        <div className="control-brand" title="Colossus Control Plane">
          <img src={colossusMark} alt="Colossus" />
        </div>
        <nav>
          <SidebarMenu>
            {navigation.map((item) => (
              <SidebarMenuItem key={item.id}>
                <SidebarMenuButton
                  asChild
                  isActive={current === item.id}
                  className="control-rail-link"
                >
                  {item.href && !item.disabled ? (
                    <a
                      href={item.href}
                      aria-current={current === item.id ? "page" : undefined}
                      title={item.label}
                      aria-label={item.label}
                      onClick={(event) => {
                        if (
                          event.button ||
                          event.metaKey ||
                          event.ctrlKey ||
                          event.shiftKey ||
                          event.altKey
                        )
                          return;
                        event.preventDefault();
                        onNavigate(item.id);
                      }}
                    >
                      {item.icon}
                      <span>{item.shortLabel ?? item.label}</span>
                    </a>
                  ) : (
                    <button
                      type="button"
                      aria-current={current === item.id ? "page" : undefined}
                      disabled={item.disabled}
                      onClick={() => onNavigate(item.id)}
                      title={item.label}
                      aria-label={item.label}
                    >
                      {item.icon}
                      <span>{item.shortLabel ?? item.label}</span>
                    </button>
                  )}
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </nav>
        <div className="control-rail-footer">
          {user}
          {footer}
        </div>
      </aside>
      {sidebar && !isMobile ? (
        <ResizablePanelGroup
          className="control-workbench"
          orientation="horizontal"
          id="control-workbench"
          onLayoutChanged={(_layout, meta) => {
            // Responsive constraint normalization must not trigger another resize.
            if (meta.isUserInteraction && panel.current)
              setOpen(!panel.current.isCollapsed());
          }}
        >
          <ResizablePanel
            id="workspace-navigation"
            panelRef={panel}
            defaultSize="320px"
            minSize="240px"
            maxSize="480px"
            collapsible
            collapsedSize={0}
            className="control-scope-panel"
          >
            <Sidebar
              collapsible="none"
              className="control-scope-sidebar"
              role="complementary"
              aria-label="Selected scope"
            >
              {sidebar}
            </Sidebar>
          </ResizablePanel>
          <ResizableHandle
            className="control-sidebar-resize"
            aria-label="Resize workspace sidebar"
            disabled={!open}
          />
          <ResizablePanel
            id="workspace-content"
            minSize="320px"
            className="control-content-panel"
          >
            {workArea}
          </ResizablePanel>
        </ResizablePanelGroup>
      ) : (
        workArea
      )}
      {sidebar && isMobile ? (
        <Sheet open={openMobile} onOpenChange={setOpenMobile}>
          <SheetContent side="left" className="control-mobile-sidebar">
            <SheetHeader className="shared-sr-only">
              <SheetTitle>Workspaces</SheetTitle>
              <SheetDescription>
                Choose a workspace or conversation on this host.
              </SheetDescription>
            </SheetHeader>
            <div className="control-scope-sidebar">{sidebar}</div>
          </SheetContent>
        </Sheet>
      ) : null}
    </>
  );
}
