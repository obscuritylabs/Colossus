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
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarInset,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from "./ui/sidebar.js";
import { TooltipProvider } from "./ui/tooltip.js";
import {
  classificationColorVariables,
  type ClassificationColors,
} from "../lib/classification-colors";
import {
  ControlPlaneNavigationMenu,
  type ControlPlaneNavigation,
} from "./ControlPlaneNavigationMenu.js";
import {
  ControlPlaneNavigationBreadcrumb,
  ControlPlaneSecondaryNavigation,
  type ControlPlaneSecondaryNavigationProps,
} from "./ControlPlaneSecondaryNavigation.js";
export type { ControlPlaneNavigation } from "./ControlPlaneNavigationMenu.js";
export type { ControlPlaneSecondaryNavigationProps } from "./ControlPlaneSecondaryNavigation.js";
export interface ClassificationBanner {
  enabled: boolean;
  text: string;
  tone: "neutral" | "info" | "warning" | "danger";
  position: "top" | "top_and_bottom";
  colors?: ClassificationColors | null;
}
interface FrameProps {
  current: string;
  navigation: ControlPlaneNavigation[];
  collapsibleNavigationGroups?: readonly string[];
  onNavigate: (id: string) => void;
  sidebar?: ReactNode;
  secondaryNavigation?: ControlPlaneSecondaryNavigationProps | undefined;
  header?: ReactNode;
  sidebarHeader?: ReactNode;
  title?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
  classification?: ClassificationBanner | undefined;
  announcement?: ReactNode;
  user?: ReactNode;
  activeScopeKey?: string;
}

/** Owned shadcn presentation; hosts retain routes, records and authority. */
export function ControlPlaneFrame(props: FrameProps) {
  const classification = props.classification;
  const classificationStyle = classificationColorVariables(
    classification?.colors,
  );
  const banner = classification?.enabled ? (
    <div
      className={`control-classification classification-${classification.tone}${classificationStyle ? " classification-custom-colors" : ""}`}
      style={classificationStyle}
      role="note"
      aria-label="Environment classification"
    >
      {classification.text}
    </div>
  ) : null;
  return (
    <div
      className={`control-plane-frame ${props.sidebar || props.secondaryNavigation ? "has-scope-sidebar" : ""}`}
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
  collapsibleNavigationGroups,
  onNavigate,
  sidebar,
  secondaryNavigation,
  header,
  footer,
  children,
  user,
  activeScopeKey,
  sidebarHeader,
  title,
  announcement,
}: FrameProps) {
  const { setOpenMobile, openMobile, open, isMobile } = useSidebar();
  const appTrigger = useRef<HTMLButtonElement>(null);
  const scopeTrigger = useRef<HTMLButtonElement>(null);
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
  }, [
    activeScopeKey,
    compactScope,
    secondaryNavigation?.current,
    setOpenMobile,
  ]);
  const secondaryScope = secondaryNavigation?.id ?? secondaryNavigation?.title;
  useEffect(() => {
    if (secondaryScope) setScopeOpen(true);
  }, [secondaryScope]);
  const scopeLabel = secondaryNavigation
    ? `${secondaryNavigation.title} navigation`
    : "Conversation navigation";
  const scopeId = secondaryNavigation
    ? "control-administration-navigation"
    : "control-conversation-navigation";
  const secondaryNavigate = (id: string) => {
    setScopeOpenMobile(false);
    secondaryNavigation?.onNavigate(id);
  };
  const scopeContent = secondaryNavigation ? (
    <ControlPlaneSecondaryNavigation
      {...secondaryNavigation}
      onNavigate={secondaryNavigate}
    />
  ) : (
    sidebar
  );
  const scopeNavigation = scopeContent ? (
    <aside
      id={scopeId}
      className={`control-scope-sidebar${secondaryNavigation ? " control-administration-sidebar" : ""}`}
      aria-label={scopeLabel}
      hidden={!compactScope && !scopeOpen}
    >
      {scopeContent}
    </aside>
  ) : null;
  const scopeToggle = (
    <Button
      variant="ghost"
      size="icon"
      ref={scopeTrigger}
      className="control-scope-trigger"
      aria-label={
        secondaryNavigation
          ? "Toggle administration navigation"
          : "Toggle conversation list"
      }
      aria-expanded={compactScope ? scopeOpenMobile : scopeOpen}
      aria-controls={scopeId}
      title={
        secondaryNavigation
          ? "Toggle administration navigation"
          : "Toggle conversation list"
      }
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
          <span className="control-header-separator" aria-hidden="true" />
          {scopeContent ? (
            compactScope ? (
              <Sheet open={scopeOpenMobile} onOpenChange={setScopeOpenMobile}>
                <SheetTrigger asChild>{scopeToggle}</SheetTrigger>
                <SheetContent
                  side="left"
                  className="control-mobile-sidebar"
                  onCloseAutoFocus={(event) => {
                    event.preventDefault();
                    scopeTrigger.current?.focus();
                  }}
                >
                  <SheetHeader className="shared-sr-only">
                    <SheetTitle>{scopeLabel}</SheetTitle>
                    <SheetDescription>
                      {secondaryNavigation
                        ? "Choose an administration section. Close to return to the current page."
                        : "Choose a workspace or conversation."}
                    </SheetDescription>
                  </SheetHeader>
                  {scopeNavigation}
                </SheetContent>
              </Sheet>
            ) : (
              scopeToggle
            )
          ) : null}
          {secondaryNavigation ? (
            <ControlPlaneNavigationBreadcrumb
              {...secondaryNavigation}
              onNavigate={secondaryNavigate}
            />
          ) : (
            <strong>
              {title ??
                navigation.find((item) => item.id === current)?.label ??
                "Colossus"}
            </strong>
          )}
        </div>
        <div>{header}</div>
      </header>
      {announcement}
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
        <SidebarHeader>
          {sidebarHeader ?? (
            <div className="control-brand" title="Colossus">
              <img src={colossusMark} alt="" />
              <div>
                <strong>Colossus</strong>
                <small>Your agent workspace</small>
              </div>
            </div>
          )}
        </SidebarHeader>
        <SidebarContent>
          <ControlPlaneNavigationMenu
            current={current}
            navigation={navigation}
            onNavigate={onNavigate}
            {...(collapsibleNavigationGroups
              ? { collapsibleNavigationGroups }
              : {})}
          />
          {footer ? (
            <div className="control-sidebar-utilities">{footer}</div>
          ) : null}
        </SidebarContent>
        {user ? <SidebarFooter>{user}</SidebarFooter> : null}
        <SidebarRail aria-label="Toggle sidebar at edge" />
      </Sidebar>
      {workArea}
    </>
  );
}
