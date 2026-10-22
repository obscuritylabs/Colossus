// Adapted from shadcn/ui sidebar-07 (MIT). See assets/shadcn-LICENSE.txt.
import {
  IconCheck,
  IconSelector,
  IconFolder,
  IconLogout,
  IconSettings,
} from "@tabler/icons-react";
import colossusMark from "../../assets/colossus-mark.svg";
import { useSidebarDropdown } from "../hooks/useSidebarDropdown.js";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu.js";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "./ui/sidebar.js";

export interface ControlPlaneProjectSwitcherProps {
  projects: { id: string; name: string }[];
  value: string;
  onValueChange: (id: string) => void;
  onManageProjects: () => void;
}

/** Hosts supply authorized projects and retain selection and navigation. */
export function ControlPlaneProjectSwitcher({
  projects,
  value,
  onValueChange,
  onManageProjects,
}: ControlPlaneProjectSwitcherProps) {
  const { isMobile } = useSidebar();
  const menu = useSidebarDropdown();
  const selectedProject = projects.find((project) => project.id === value);
  return (
    <SidebarMenu className="control-project-switcher">
      <SidebarMenuItem>
        <DropdownMenu
          open={menu.open}
          onOpenChange={menu.onOpenChange}
          modal={menu.modal}
        >
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton
              ref={menu.triggerRef}
              className="control-project-switcher-trigger control-sidebar-menu-trigger"
              aria-label="Select project"
              title={selectedProject?.name ?? "Select project"}
            >
              <span
                className="control-project-switcher-mark"
                aria-hidden="true"
              >
                <img src={colossusMark} alt="" />
              </span>
              <span className="control-sidebar-menu-copy">
                <strong>{selectedProject?.name ?? "Select project"}</strong>
                <small>Colossus</small>
              </span>
              <IconSelector
                className="control-sidebar-menu-chevron"
                size={16}
                aria-hidden="true"
              />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            ref={menu.contentRef}
            container={menu.portalContainer}
            className="control-sidebar-dropdown control-project-switcher-dropdown"
            side={isMobile ? "bottom" : "right"}
            align="start"
            sideOffset={4}
            collisionPadding={8}
          >
            <DropdownMenuLabel className="control-sidebar-dropdown-heading">
              Projects
            </DropdownMenuLabel>
            {projects.map((project) => (
              <DropdownMenuItem
                className="control-sidebar-dropdown-item"
                key={project.id}
                aria-current={project.id === value ? "true" : undefined}
                onSelect={() => onValueChange(project.id)}
              >
                <span
                  className="control-project-switcher-initial"
                  aria-hidden="true"
                >
                  {initials(project.name)}
                </span>
                <span className="control-sidebar-dropdown-item-copy">
                  {project.name}
                </span>
                {project.id === value ? (
                  <IconCheck size={16} aria-hidden="true" />
                ) : null}
              </DropdownMenuItem>
            ))}
            <div
              className="control-sidebar-dropdown-separator"
              role="separator"
            />
            <DropdownMenuItem
              className="control-sidebar-dropdown-item"
              onSelect={onManageProjects}
            >
              <IconFolder size={18} aria-hidden="true" />
              <span className="control-sidebar-dropdown-item-copy">
                Manage projects
              </span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

export interface ControlPlaneAccountMenuProps {
  name: string;
  detail?: string;
  onSettings: () => void;
  onSignOut: () => void;
  signingOut?: boolean;
}

/** Account actions are callbacks; the host retains authentication authority. */
export function ControlPlaneAccountMenu({
  name,
  detail,
  onSettings,
  onSignOut,
  signingOut = false,
}: ControlPlaneAccountMenuProps) {
  const { isMobile } = useSidebar();
  const menu = useSidebarDropdown();
  const account = (
    <>
      <span className="control-account-menu-avatar" aria-hidden="true">
        {initials(name)}
      </span>
      <span className="control-sidebar-menu-copy">
        <strong>{name}</strong>
        {detail ? <small>{detail}</small> : null}
      </span>
    </>
  );
  return (
    <SidebarMenu className="control-account-menu">
      <SidebarMenuItem>
        <DropdownMenu
          open={menu.open}
          onOpenChange={menu.onOpenChange}
          modal={menu.modal}
        >
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton
              ref={menu.triggerRef}
              className="control-account-menu-trigger control-sidebar-menu-trigger"
              aria-label="Account menu"
              title={name}
            >
              {account}
              <IconSelector
                className="control-sidebar-menu-chevron"
                size={16}
                aria-hidden="true"
              />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            ref={menu.contentRef}
            container={menu.portalContainer}
            className="control-sidebar-dropdown control-account-menu-dropdown"
            side={isMobile ? "bottom" : "right"}
            align="end"
            sideOffset={4}
            collisionPadding={8}
          >
            <DropdownMenuLabel className="control-account-menu-label">
              {account}
            </DropdownMenuLabel>
            <div
              className="control-sidebar-dropdown-separator"
              role="separator"
            />
            <DropdownMenuItem
              className="control-sidebar-dropdown-item"
              onSelect={onSettings}
            >
              <IconSettings size={18} aria-hidden="true" />
              <span className="control-sidebar-dropdown-item-copy">
                Settings
              </span>
            </DropdownMenuItem>
            <DropdownMenuItem
              className="control-sidebar-dropdown-item"
              onSelect={onSignOut}
              disabled={signingOut}
              aria-busy={signingOut || undefined}
            >
              <IconLogout size={18} aria-hidden="true" />
              <span className="control-sidebar-dropdown-item-copy">
                {signingOut ? "Signing out…" : "Sign out"}
              </span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function initials(name: string): string {
  return (
    name
      .trim()
      .split(/\s+/)
      .slice(0, 2)
      .map((part) => Array.from(part)[0] ?? "")
      .join("")
      .toUpperCase() || "C"
  );
}
