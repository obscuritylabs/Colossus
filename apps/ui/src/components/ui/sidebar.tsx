// Adapted from shadcn/ui radix-nova (MIT). See assets/shadcn-LICENSE.txt.
// Colossus owns semantic styling and leaves persistence to the consuming host.
import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { IconLayoutSidebar } from "@tabler/icons-react";
import { cn } from "../../lib/utils.js";
import { useIsMobile } from "../../hooks/use-mobile.js";
import { Button } from "./button.js";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "./sheet.js";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip.js";

type SidebarContextProps = {
  state: "expanded" | "collapsed";
  open: boolean;
  setOpen: React.Dispatch<React.SetStateAction<boolean>>;
  openMobile: boolean;
  setOpenMobile: React.Dispatch<React.SetStateAction<boolean>>;
  isMobile: boolean;
  toggleSidebar: () => void;
};
const SidebarContext =
  /* @__PURE__ */ React.createContext<SidebarContextProps | null>(null);
export function useSidebar() {
  const context = React.useContext(SidebarContext);
  if (!context)
    throw new Error("useSidebar must be used within a SidebarProvider.");
  return context;
}
export function SidebarProvider({
  defaultOpen = true,
  open: controlledOpen,
  onOpenChange,
  className,
  style,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const isMobile = useIsMobile();
  const [localOpen, setLocalOpen] = React.useState(defaultOpen);
  const [openMobile, setOpenMobile] = React.useState(false);
  const open = controlledOpen ?? localOpen;
  const setOpen = React.useCallback<
    React.Dispatch<React.SetStateAction<boolean>>
  >(
    (next) => {
      const value = typeof next === "function" ? next(open) : next;
      if (onOpenChange) onOpenChange(value);
      else setLocalOpen(value);
    },
    [open, onOpenChange],
  );
  const toggleSidebar = React.useCallback(() => {
    if (isMobile) setOpenMobile((value) => !value);
    else setOpen((value) => !value);
  }, [isMobile, setOpen]);
  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        !event.isComposing &&
        event.key.toLowerCase() === "b" &&
        (event.metaKey || event.ctrlKey) &&
        !event.altKey
      ) {
        event.preventDefault();
        toggleSidebar();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleSidebar]);
  const value = React.useMemo<SidebarContextProps>(
    () => ({
      state: open ? "expanded" : "collapsed",
      open,
      setOpen,
      openMobile,
      setOpenMobile,
      isMobile,
      toggleSidebar,
    }),
    [open, setOpen, openMobile, isMobile, toggleSidebar],
  );
  return (
    <SidebarContext.Provider value={value}>
      <div
        data-slot="sidebar-wrapper"
        className={cn("ui-sidebar-provider", className)}
        style={{ "--sidebar-width": "20rem", ...style } as React.CSSProperties}
        {...props}
      >
        {children}
      </div>
    </SidebarContext.Provider>
  );
}
export function Sidebar({
  side = "left",
  collapsible = "offcanvas",
  className,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  side?: "left" | "right";
  collapsible?: "offcanvas" | "icon" | "none";
}) {
  const { isMobile, state, openMobile, setOpenMobile } = useSidebar();
  if (collapsible !== "none" && isMobile)
    return (
      <Sheet open={openMobile} onOpenChange={setOpenMobile}>
        <SheetContent side={side} className={cn("ui-sidebar-sheet", className)}>
          <SheetHeader className="shared-sr-only">
            <SheetTitle>Workspaces</SheetTitle>
            <SheetDescription>Workspace navigation</SheetDescription>
          </SheetHeader>
          <div {...props}>{children}</div>
        </SheetContent>
      </Sheet>
    );
  return (
    <div
      data-slot="sidebar"
      data-sidebar="sidebar"
      data-side={side}
      data-state={state}
      data-collapsible={
        collapsible === "none" ? "" : state === "collapsed" ? collapsible : ""
      }
      className={cn("ui-sidebar", className)}
      {...props}
    >
      {children}
    </div>
  );
}
export function SidebarTrigger({
  className,
  onClick,
  ...props
}: React.ComponentProps<typeof Button>) {
  const { toggleSidebar } = useSidebar();
  return (
    <Button
      variant="ghost"
      size="icon"
      data-slot="sidebar-trigger"
      className={className}
      onClick={(event) => {
        onClick?.(event);
        if (!event.defaultPrevented) toggleSidebar();
      }}
      {...props}
    >
      <IconLayoutSidebar size={18} aria-hidden="true" />
      <span className="shared-sr-only">Toggle Sidebar</span>
    </Button>
  );
}
export function SidebarInset({
  className,
  ...props
}: React.ComponentProps<"main">) {
  return (
    <main
      data-slot="sidebar-inset"
      className={cn("ui-sidebar-inset", className)}
      {...props}
    />
  );
}
export function SidebarHeader({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-header"
      className={cn("ui-sidebar-header", className)}
      {...props}
    />
  );
}
export function SidebarContent({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-content"
      className={cn("ui-sidebar-content", className)}
      {...props}
    />
  );
}
export function SidebarFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sidebar-footer"
      className={cn("ui-sidebar-footer", className)}
      {...props}
    />
  );
}
export function SidebarMenu({
  className,
  ...props
}: React.ComponentProps<"ul">) {
  return (
    <ul
      data-slot="sidebar-menu"
      className={cn("ui-sidebar-menu", className)}
      {...props}
    />
  );
}
export function SidebarMenuItem({
  className,
  ...props
}: React.ComponentProps<"li">) {
  return (
    <li
      data-slot="sidebar-menu-item"
      className={cn("ui-sidebar-menu-item", className)}
      {...props}
    />
  );
}
export function SidebarMenuButton({
  asChild = false,
  isActive = false,
  tooltip,
  className,
  ...props
}: React.ComponentProps<"button"> & {
  asChild?: boolean;
  isActive?: boolean;
  tooltip?: string;
}) {
  const Component = asChild ? Slot : "button";
  const button = (
    <Component
      type={asChild ? undefined : "button"}
      data-slot="sidebar-menu-button"
      data-active={isActive ? "true" : undefined}
      className={cn("ui-sidebar-menu-button", className)}
      {...props}
    />
  );
  return tooltip ? (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent side="right">{tooltip}</TooltipContent>
    </Tooltip>
  ) : (
    button
  );
}
export function SidebarMenuSubButton({
  asChild = false,
  isActive = false,
  className,
  ...props
}: React.ComponentProps<"a"> & { asChild?: boolean; isActive?: boolean }) {
  const Component = asChild ? Slot : "a";
  return (
    <Component
      data-slot="sidebar-menu-sub-button"
      data-active={isActive ? "true" : undefined}
      className={cn("ui-sidebar-menu-sub-button", className)}
      {...props}
    />
  );
}
