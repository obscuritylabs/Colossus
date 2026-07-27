import {
  IconActivity,
  IconBriefcase2,
  IconLibrary,
  IconSettings,
  IconTerminal2,
  IconTopologyStar3,
} from "@tabler/icons-react";

import colossusMark from "../assets/colossus-mark.svg";
import type { ConnectionState, DesktopCapabilities } from "../types";

export type WorkspaceSurface =
  "work" | "fleet" | "library" | "activity" | "settings";

interface ProductRailProps {
  surface: WorkspaceSurface;
  attentionCount: number;
  connectionState: ConnectionState;
  terminalEnabled: boolean;
  terminalAvailable: boolean;
  capabilities: DesktopCapabilities;
  onSelect: (surface: WorkspaceSurface) => void;
  onOpenTerminal: () => void;
}

const MAIN_ITEMS = [
  { id: "work", label: "Work", Icon: IconBriefcase2 },
  { id: "fleet", label: "Agents", Icon: IconTopologyStar3 },
  { id: "library", label: "Library", Icon: IconLibrary },
  { id: "activity", label: "Activity", Icon: IconActivity },
] as const;

export function ProductRail({
  surface,
  attentionCount,
  connectionState,
  terminalEnabled,
  terminalAvailable,
  capabilities,
  onSelect,
  onOpenTerminal,
}: ProductRailProps) {
  return (
    <aside className="product-rail" aria-label="Colossus navigation">
      <div className="product-mark-wrap">
        <img className="product-mark" src={colossusMark} alt="Colossus" />
      </div>

      <nav className="product-nav" aria-label="Workspace areas">
        {MAIN_ITEMS.filter(({ id }) => {
          if (id === "fleet") {
            return (
              capabilities.delegation ||
              capabilities.agentWorkflows ||
              capabilities.skills
            );
          }
          if (id === "library") {
            return capabilities.artifacts;
          }
          return true;
        }).map(({ id, label, Icon }) => (
          <button
            className="product-nav-item"
            type="button"
            key={id}
            aria-current={surface === id ? "page" : undefined}
            onClick={() => onSelect(id)}
          >
            <span className="product-nav-icon" aria-hidden="true">
              <Icon size={21} stroke={1.7} />
              {id === "fleet" && attentionCount > 0 ? (
                <span className="nav-count">
                  {Math.min(attentionCount, 99)}
                </span>
              ) : null}
            </span>
            <span>{label}</span>
          </button>
        ))}
        {capabilities.tui ? (
          <button
            className="product-nav-item"
            type="button"
            disabled={!terminalAvailable || !terminalEnabled}
            title={
              !terminalAvailable
                ? "Terminal is available only for the selected Managed Local target"
                : terminalEnabled
                  ? "Open the authenticated local Colossus TUI"
                  : "Enable the local Colossus TUI in Settings"
            }
            onClick={onOpenTerminal}
          >
            <span className="product-nav-icon" aria-hidden="true">
              <IconTerminal2 size={21} stroke={1.7} />
            </span>
            <span>TUI</span>
          </button>
        ) : null}
      </nav>

      <div className="product-rail-footer">
        <button
          className="product-nav-item"
          type="button"
          aria-current={surface === "settings" ? "page" : undefined}
          onClick={() => onSelect("settings")}
        >
          <span className="product-nav-icon" aria-hidden="true">
            <IconSettings size={21} stroke={1.7} />
          </span>
          <span>Settings</span>
        </button>
        <div className="rail-identity" title="Local Colossus runtime">
          <img className="identity-avatar" src={colossusMark} alt="" />
          <span className="identity-copy">
            <strong>Colossus</strong>
            <span>
              <i
                className={`connection-dot connection-${connectionState}`}
                aria-hidden="true"
              />
              {connectionState === "connected" ? "Online" : "Offline"}
            </span>
          </span>
        </div>
      </div>
    </aside>
  );
}
