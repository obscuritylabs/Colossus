import {
  IconActivity,
  IconBriefcase2,
  IconLibrary,
  IconSettings,
  IconTopologyStar3,
} from "@tabler/icons-react";

import colossusMark from "../assets/colossus-mark.svg";
import type { ConnectionState } from "../types";

export type WorkspaceSurface =
  "work" | "fleet" | "library" | "activity" | "settings";

interface ProductRailProps {
  surface: WorkspaceSurface;
  attentionCount: number;
  connectionState: ConnectionState;
  onSelect: (surface: WorkspaceSurface) => void;
}

const MAIN_ITEMS = [
  { id: "work", label: "Work", Icon: IconBriefcase2 },
  { id: "fleet", label: "Fleet", Icon: IconTopologyStar3 },
  { id: "library", label: "Library", Icon: IconLibrary },
  { id: "activity", label: "Activity", Icon: IconActivity },
] as const;

export function ProductRail({
  surface,
  attentionCount,
  connectionState,
  onSelect,
}: ProductRailProps) {
  return (
    <aside className="product-rail" aria-label="Colossus navigation">
      <div className="product-mark-wrap">
        <img className="product-mark" src={colossusMark} alt="Colossus" />
      </div>

      <nav className="product-nav" aria-label="Workspace areas">
        {MAIN_ITEMS.map(({ id, label, Icon }) => (
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
