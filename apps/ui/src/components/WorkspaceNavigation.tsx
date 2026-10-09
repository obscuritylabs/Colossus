import type { ReactNode } from "react";
import {
  IconBriefcase2,
  IconTopologyStar3,
  IconPlugConnected,
  IconCalendarTime,
  IconLibrary,
  IconSettings,
} from "@tabler/icons-react";
export const WORKSPACE_DESTINATIONS = [
  { id: "work", label: "Work", Icon: IconBriefcase2 },
  { id: "fleet", label: "Capabilities", Icon: IconTopologyStar3 },
  { id: "plugins", label: "Plugins", Icon: IconPlugConnected },
  { id: "workflows", label: "Workflows", Icon: IconTopologyStar3 },
  { id: "schedules", label: "Schedules", Icon: IconCalendarTime },
  { id: "library", label: "Library", Icon: IconLibrary },
  { id: "connections", label: "Connections", Icon: IconPlugConnected },
  { id: "settings", label: "Settings", Icon: IconSettings },
] as const;
export type WorkspaceDestination = (typeof WORKSPACE_DESTINATIONS)[number];
export type WorkspaceDestinationId = WorkspaceDestination["id"];
/** One native navigation hierarchy; hosts supply links, selection and attention. */
export function WorkspaceNavigation({
  active,
  onSelect,
  renderItem,
  workAttention = 0,
}: {
  active: WorkspaceDestinationId;
  onSelect?: (id: WorkspaceDestinationId) => void;
  renderItem?: (item: WorkspaceDestination, children: ReactNode) => ReactNode;
  workAttention?: number;
}) {
  return (
    <nav className="space-destinations" aria-label="Workspace destinations">
      {WORKSPACE_DESTINATIONS.map((item) => {
        const { id, label, Icon } = item;
        const children = (
          <>
            <Icon size={17} stroke={1.7} aria-hidden="true" />
            <span>{label}</span>
            {id === "work" && workAttention > 0 ? (
              <span className="space-attention-badge">
                {Math.min(workAttention, 99)}
              </span>
            ) : null}
          </>
        );
        return renderItem ? (
          <span key={id} className="workspace-destination-item">
            {renderItem(item, children)}
          </span>
        ) : (
          <button
            key={id}
            type="button"
            className="sidebar-nav-item"
            aria-label={label}
            aria-current={active === id ? "page" : undefined}
            onClick={() => onSelect?.(id)}
          >
            {children}
          </button>
        );
      })}
    </nav>
  );
}
