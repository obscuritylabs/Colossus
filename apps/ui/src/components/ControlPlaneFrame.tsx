import { useEffect, useRef, useState, type ReactNode } from "react";
import { IconListDetails } from "@tabler/icons-react";
import colossusMark from "../../assets/colossus-mark.svg";
import { Button } from "./ui/button.js";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "./ui/sheet.js";
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
import { TooltipProvider } from "./ui/tooltip.js";

export interface ControlPlaneNavigation {
  id: string;
  label: string;
  shortLabel?: string;
  href?: string;
  icon: ReactNode;
  disabled?: boolean;
  group?: string;
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
  sidebarHeader?: ReactNode;
  title?: ReactNode;
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
  sidebarHeader,
  title,
}: FrameProps) {
  const { setOpenMobile, openMobile, open, isMobile } = useSidebar();
  const appTrigger = useRef<HTMLButtonElement>(null);
  const [scopeOpen, setScopeOpen] = useState(true);
  const [scopeOpenMobile, setScopeOpenMobile] = useState(false);
  const [compactScope, setCompactScope] = useState(() =>
    typeof window !== "undefined" && window.matchMedia
      ? window.matchMedia("(max-width: 1100px)").matches
      : false,
  );
  useEffect(() => {
    const query = window.matchMedia("(max-width: 1100px)");
    const changed = () => setCompactScope(query.matches);
    query.addEventListener("change", changed);
    changed();
    return () => query.removeEventListener("change", changed);
  }, []);
  useEffect(() => {
    setOpenMobile(false);
    setScopeOpenMobile(false);
  }, [activeScopeKey, compactScope, setOpenMobile]);
  const scopeNavigation = sidebar ? (
    <aside
      id="control-conversation-navigation"
      className="control-scope-sidebar"
      aria-label="Conversation navigation"
      hidden={!compactScope && !scopeOpen}
    >
      {sidebar}
    </aside>
  ) : null;
  const scopeToggle = (
    <Button
      variant="ghost"
      size="icon"
      className="control-scope-trigger"
      aria-label="Toggle conversation list"
      aria-expanded={compactScope ? scopeOpenMobile : scopeOpen}
      aria-controls="control-conversation-navigation"
      title="Toggle conversation list"
      onClick={compactScope ? undefined : () => setScopeOpen((value) => !value)}
    >
      <IconListDetails size={18} aria-hidden="true" />
    </Button>
  );
  const workArea = (
    <SidebarInset
      className="control-frame-main"
      id="control-main"
      tabIndex={-1}
    >
      <header className="control-header">
        <div className="control-header-context">
          <SidebarTrigger
            ref={appTrigger}
            aria-label="Toggle app sidebar"
            aria-expanded={isMobile ? openMobile : open}
            aria-controls="control-app-navigation"
            title="Toggle app sidebar (⌘/Ctrl B)"
          />
          {sidebar ? (
            compactScope ? (
              <Sheet open={scopeOpenMobile} onOpenChange={setScopeOpenMobile}>
                <SheetTrigger asChild>{scopeToggle}</SheetTrigger>
                <SheetContent side="left" className="control-mobile-sidebar">
                  <SheetHeader className="shared-sr-only">
                    <SheetTitle>Conversation navigation</SheetTitle>
                    <SheetDescription>
                      Choose a workspace or conversation.
                    </SheetDescription>
                  </SheetHeader>
                  {scopeNavigation}
                </SheetContent>
              </Sheet>
            ) : (
              scopeToggle
            )
          ) : null}
          <strong>
            {title ??
              navigation.find((item) => item.id === current)?.label ??
              "Colossus"}
          </strong>
        </div>
        <div>{header}</div>
      </header>
      <div className="control-workbench">
        {compactScope ? null : scopeNavigation}
        <div className="control-content">{children}</div>
      </div>
    </SidebarInset>
  );
  return (
    <>
      <Sidebar
        collapsible="icon"
        className="control-app-sidebar"
        id="control-app-navigation"
        aria-label="App sidebar"
        mobileTitle="Navigation"
        mobileDescription="Choose a project or application page."
        onMobileCloseAutoFocus={(event) => {
          event.preventDefault();
          appTrigger.current?.focus();
        }}
      >
        <div className="control-brand" title="Colossus">
          <img src={colossusMark} alt="" />
          <div>
            <strong>Colossus</strong>
            <small>Your agent workspace</small>
          </div>
        </div>
        {sidebarHeader ? (
          <div className="control-sidebar-context">{sidebarHeader}</div>
        ) : null}
        <nav>
          {[...new Set(navigation.map((item) => item.group ?? ""))].map(
            (group) => (
              <section
                className="control-navigation-group"
                key={group}
                aria-label={group || undefined}
              >
                {group ? (
                  <h2 className="control-navigation-heading">{group}</h2>
                ) : null}
                <SidebarMenu>
                  {navigation
                    .filter((item) => (item.group ?? "") === group)
                    .map((item) => (
                      <SidebarMenuItem key={item.id}>
                        <SidebarMenuButton
                          asChild
                          isActive={current === item.id}
                          className="control-app-link"
                        >
                          {item.href && !item.disabled ? (
                            <a
                              href={item.href}
                              aria-current={
                                current === item.id ? "page" : undefined
                              }
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
                              <span>{item.label}</span>
                            </a>
                          ) : (
                            <button
                              type="button"
                              aria-current={
                                current === item.id ? "page" : undefined
                              }
                              disabled={item.disabled}
                              onClick={() => onNavigate(item.id)}
                              title={item.label}
                              aria-label={item.label}
                            >
                              {item.icon}
                              <span>{item.label}</span>
                            </button>
                          )}
                        </SidebarMenuButton>
                      </SidebarMenuItem>
                    ))}
                </SidebarMenu>
              </section>
            ),
          )}
        </nav>
        <div className="control-rail-footer">
          {user}
          {footer}
        </div>
      </Sidebar>
      {workArea}
    </>
  );
}
