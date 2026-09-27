import {
  IconActivityHeartbeat,
  IconAdjustments,
  IconArrowLeft,
  IconBox,
  IconCloud,
  IconCpu,
  IconDeviceDesktop,
  IconFileCode,
  IconFlask,
  IconFolder,
  IconKey,
  IconPlugConnected,
  IconSearch,
  IconSettings,
  IconShield,
  IconWorld,
  IconX,
} from "@tabler/icons-react";
import type { ReactNode } from "react";
import colossusMark from "../assets/colossus-mark.svg";

const sectionIcons = {
  runtime: IconSettings,
  providers: IconCloud,
  models: IconCpu,
  credentials: IconKey,
  mcp: IconPlugConnected,
  plugins: IconPlugConnected,
  access: IconShield,
  sandbox: IconBox,
  search: IconSearch,
  telemetry: IconActivityHeartbeat,
  research: IconFlask,
  advanced: IconAdjustments,
  effective: IconFileCode,
  defaults: IconAdjustments,
  desktop: IconDeviceDesktop,
};

export function SettingsFrame({
  scope,
  onScopeChange,
  query,
  onQueryChange,
  tabs,
  activeTab,
  onTabChange,
  workspaceContext,
  onReturnToWork,
  children,
}: {
  scope: "global" | "space";
  onScopeChange: (scope: "global" | "space") => void;
  query: string;
  onQueryChange: (query: string) => void;
  tabs: ReadonlyArray<{ id: string; label: string }>;
  activeTab: string;
  onTabChange: (tab: string) => void;
  workspaceContext: ReactNode;
  onReturnToWork?: (() => void) | undefined;
  children: ReactNode;
}) {
  return (
    <div className="managed-settings-shell">
      <header className="managed-settings-header">
        <div className="settings-brand">
          <img src={colossusMark} alt="" />
          <strong>Colossus</strong>
          <h2>Settings</h2>
        </div>
        <label className="managed-settings-search">
          <IconSearch size={18} aria-hidden="true" />
          <span className="sr-only">Search settings</span>
          <input
            type="search"
            value={query}
            placeholder="Search all settings"
            onChange={(event) => onQueryChange(event.target.value)}
          />
          {query ? (
            <button
              type="button"
              aria-label="Clear settings search"
              onClick={() => onQueryChange("")}
            >
              <IconX size={16} aria-hidden="true" />
            </button>
          ) : null}
        </label>
      </header>
      <div className="settings-frame-layout">
        <aside className="settings-sidebar" aria-label="Settings navigation">
          <div className="settings-sidebar-content">
            <div
              className="managed-scope-switch"
              role="group"
              aria-label="Configuration scope"
            >
              <button
                type="button"
                aria-label="Global"
                aria-describedby="settings-global-description"
                aria-pressed={scope === "global"}
                className="sidebar-nav-item"
                onClick={() => onScopeChange("global")}
              >
                <IconWorld size={17} stroke={1.7} aria-hidden="true" />
                <span>
                  <strong>Global</strong>
                  <small id="settings-global-description">
                    Shared resources &amp; defaults
                  </small>
                </span>
              </button>
              <button
                type="button"
                aria-label="Workspace"
                aria-describedby="settings-workspace-description"
                aria-pressed={scope === "space"}
                className="sidebar-nav-item"
                onClick={() => onScopeChange("space")}
              >
                <IconFolder size={17} stroke={1.7} aria-hidden="true" />
                <span>
                  <strong>Workspace</strong>
                  <small id="settings-workspace-description">
                    Settings for one workspace
                  </small>
                </span>
              </button>
            </div>
            <div className="settings-scope-context">
              <div
                className={`settings-workspace-context${scope === "global" ? " is-inactive" : ""}`}
                inert={scope === "global"}
                aria-hidden={scope === "global"}
              >
                {workspaceContext}
              </div>
              {scope === "global" ? (
                <p>Manage shared resources and defaults for your workspaces.</p>
              ) : null}
            </div>
            <nav
              className="managed-settings-tabs"
              aria-label="Settings sections"
            >
              {tabs.map((tab) => {
                const Icon =
                  sectionIcons[tab.id as keyof typeof sectionIcons] ??
                  IconSettings;
                return (
                  <button
                    key={tab.id}
                    type="button"
                    className="sidebar-nav-item"
                    aria-current={
                      !query && activeTab === tab.id ? "page" : undefined
                    }
                    onClick={() => onTabChange(tab.id)}
                  >
                    <Icon size={17} stroke={1.7} aria-hidden="true" />
                    {tab.label}
                  </button>
                );
              })}
            </nav>
          </div>
          {onReturnToWork ? (
            <div className="settings-sidebar-footer">
              <button
                type="button"
                className="sidebar-nav-item"
                onClick={onReturnToWork}
              >
                <IconArrowLeft size={17} stroke={1.7} aria-hidden="true" />
                Back to work
              </button>
            </div>
          ) : null}
        </aside>
        <div
          className="settings-main"
          tabIndex={0}
          aria-label="Settings content"
        >
          {children}
        </div>
      </div>
    </div>
  );
}
