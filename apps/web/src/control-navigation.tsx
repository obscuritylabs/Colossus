import {
  IconActivityHeartbeat,
  IconAdjustments,
  IconBell,
  IconCloud,
  IconCpu,
  IconFolder,
  IconHome,
  IconKey,
  IconMessageCircle,
  IconPalette,
  IconPlugConnected,
  IconSearch,
  IconServer,
  IconSettings,
  IconShield,
  IconUsers,
  IconVariable,
} from "@tabler/icons-react";
import type { ControlPlaneNavigation } from "@colossus/ui/components/ControlPlaneFrame";
import type { ReactNode } from "react";
import type { Me } from "./control-api";
import { projectPermissions } from "./control-api";
import {
  globalHref,
  personalSettingsHref,
  projectHref,
  projectSettingsHref,
  routeSurface,
  type AdminView,
  type ConnectionSettingsSection,
  type Route,
} from "./routes";

export const configurationSections: ReadonlyArray<{
  id: ConnectionSettingsSection;
  label: string;
  icon: ReactNode;
}> = [
  { id: "providers", label: "Providers", icon: <IconCloud size={19} /> },
  { id: "models", label: "Models", icon: <IconCpu size={19} /> },
  { id: "mcp", label: "MCP servers", icon: <IconPlugConnected size={19} /> },
  {
    id: "routing",
    label: "Model routing",
    icon: <IconAdjustments size={19} />,
  },
  { id: "search", label: "Search", icon: <IconSearch size={19} /> },
  {
    id: "telemetry",
    label: "Telemetry",
    icon: <IconActivityHeartbeat size={19} />,
  },
  { id: "credentials", label: "Secrets", icon: <IconKey size={19} /> },
  { id: "variables", label: "Variables", icon: <IconVariable size={19} /> },
];

/** Navigation reflects released authority; API authorization remains server-owned. */
export function controlNavigation(me: Me, route: Route, project: string) {
  const authorized = me.projects.some((item) => item.id === project);
  const selected = authorized ? project : undefined;
  const canAdminister =
    authorized && projectPermissions(me, project).includes("administer");
  const items: ControlPlaneNavigation[] = [
    {
      id: "home",
      group: "Workspace",
      label: "Home",
      href: globalHref("home", selected),
      icon: <IconHome size={21} />,
    },
    {
      id: "chat",
      group: "Workspace",
      label: "Chat",
      href: globalHref("chat", selected),
      icon: <IconMessageCircle size={21} />,
    },
    {
      id: "hosted",
      group: "Workspace",
      label: "Hosted",
      href: globalHref("hosted", selected),
      icon: <IconCloud size={21} />,
    },
    {
      id: "fleet",
      group: "Workspace",
      label: "Fleet",
      href: globalHref("fleet", selected),
      icon: <IconServer size={21} />,
    },
    {
      id: "projects",
      group: "Workspace",
      label: "Projects",
      href: globalHref("projects"),
      icon: <IconFolder size={21} />,
    },
  ];
  if (canAdminister) {
    items.push(
      {
        id: "project-members",
        group: "Project Administration",
        label: "Members",
        href: projectHref(project, "access"),
        icon: <IconUsers size={19} />,
      },
      ...configurationSections.map((section) => ({
        id: `project-${section.id}`,
        group: "Project Administration",
        label: section.label,
        href: projectSettingsHref(project, section.id),
        icon: section.icon,
      })),
      {
        id: "project-settings",
        group: "Project Administration",
        label: "General",
        href: projectHref(project, "settings"),
        icon: <IconSettings size={19} />,
      },
    );
  }
  if (me.user.is_admin) {
    const admin = (
      view: AdminView,
      label: string,
      icon: ReactNode,
    ): ControlPlaneNavigation => ({
      id: `admin-${view}`,
      group: "Control Plane Admin",
      label,
      href: globalHref("admin", undefined, view),
      icon,
    });
    items.push(
      admin("users", "Users", <IconUsers size={19} />),
      admin("projects", "Projects", <IconFolder size={19} />),
      ...configurationSections.map((section) =>
        admin(section.id, section.label, section.icon),
      ),
      admin("announcements", "Announcements", <IconBell size={19} />),
      admin("appearance", "Appearance", <IconPalette size={19} />),
      admin("settings", "Authentication & settings", <IconShield size={19} />),
    );
  }
  items.push({
    id: "settings",
    group: "Personal",
    label: "Settings",
    href: personalSettingsHref("appearance", selected),
    icon: <IconSettings size={21} />,
  });
  let current: string = routeSurface(route);
  if (route.kind === "project-settings") current = `project-${route.section}`;
  else if (route.kind === "project" && route.view === "access" && canAdminister)
    current = "project-members";
  else if (
    route.kind === "project" &&
    route.view === "settings" &&
    canAdminister
  )
    current = "project-settings";
  else if (route.kind === "global" && route.surface === "admin")
    current = `admin-${route.view ?? "users"}`;
  return { items, current };
}
