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
  if (canAdminister)
    items.push({
      id: "project-admin",
      group: "Administration",
      label: "Project Admin",
      href: projectHref(project, "access"),
      icon: <IconSettings size={21} />,
    });
  if (me.user.is_admin)
    items.push({
      id: "admin",
      group: "Administration",
      label: "Control Plane Admin",
      href: globalHref("admin", undefined, "users"),
      icon: <IconShield size={21} />,
    });
  items.push({
    id: "settings",
    group: "Personal",
    label: "Settings",
    href: personalSettingsHref("appearance", selected),
    icon: <IconSettings size={21} />,
  });
  const projectAdministration =
    canAdminister &&
    ((route.kind === "project-settings" && route.project === project) ||
      (route.kind === "project" &&
        route.project === project &&
        (route.view === "access" || route.view === "settings")));
  if (projectAdministration) {
    const current =
      route.kind === "project-settings"
        ? `project-${route.section}`
        : route.view === "access"
          ? "project-members"
          : "project-settings";
    const sections: ControlPlaneNavigation[] = [
      {
        id: "project-members",
        label: "Members",
        href: projectHref(project, "access"),
        icon: <IconUsers size={19} />,
      },
      ...configurationSections.map((section) => ({
        id: `project-${section.id}`,
        label: section.label,
        href: projectSettingsHref(project, section.id),
        icon: section.icon,
      })),
      {
        id: "project-settings",
        label: "General",
        href: projectHref(project, "settings"),
        icon: <IconSettings size={19} />,
      },
    ];
    return {
      items,
      current: "project-admin",
      secondary: {
        id: `project:${project}`,
        title: "Project Admin",
        context: me.projects.find((item) => item.id === project)!.name,
        items: sections,
        current,
      },
    };
  }
  if (
    me.user.is_admin &&
    route.kind === "global" &&
    route.surface === "admin"
  ) {
    const admin = (
      view: AdminView,
      label: string,
      icon: ReactNode,
    ): ControlPlaneNavigation => ({
      id: `admin-${view}`,
      label,
      href: globalHref("admin", undefined, view),
      icon,
    });
    const sections: ControlPlaneNavigation[] = [
      admin("users", "Users", <IconUsers size={19} />),
      admin("projects", "Projects", <IconFolder size={19} />),
      ...configurationSections.map((section) =>
        admin(section.id, section.label, section.icon),
      ),
      admin("announcements", "Announcements", <IconBell size={19} />),
      admin("appearance", "Appearance", <IconPalette size={19} />),
      admin("settings", "Authentication & settings", <IconShield size={19} />),
    ];
    return {
      items,
      current: "admin",
      secondary: {
        id: "control-plane",
        title: "Control Plane Admin",
        context: "Instance-wide settings",
        items: sections,
        current: `admin-${route.view ?? "users"}`,
      },
    };
  }
  return { items, current: routeSurface(route), secondary: undefined };
}
