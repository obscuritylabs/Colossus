import {
  IconActivity,
  IconFolder,
  IconSearch,
  IconWorld,
} from "@tabler/icons-react";
import type { ReactNode, Ref } from "react";

/** Sidebar data, navigation, shortcuts, and workspace authority stay in the host. */
export function WorkspaceSidebarHeading({
  actions,
  id,
  className = "",
}: {
  actions?: ReactNode;
  id?: string;
  className?: string;
}) {
  return (
    <div className={`shared-workspace-sidebar-heading ${className}`}>
      <span id={id}>Workspaces</span>
      {actions}
    </div>
  );
}
export function WorkspaceSidebarSearch({
  value,
  onChange,
  inputRef,
  shortcut,
  className = "",
  maxLength,
}: {
  value: string;
  onChange: (value: string) => void;
  inputRef?: Ref<HTMLInputElement>;
  shortcut?: ReactNode;
  className?: string;
  maxLength?: number;
}) {
  return (
    <div className={`shared-workspace-search ${className}`}>
      <IconSearch size={17} stroke={1.7} aria-hidden="true" />
      <input
        ref={inputRef}
        type="search"
        aria-label="Search threads"
        placeholder="Search threads"
        value={value}
        maxLength={maxLength}
        onChange={(event) => onChange(event.target.value)}
      />
      {shortcut ? <kbd aria-hidden="true">{shortcut}</kbd> : null}
    </div>
  );
}
export type WorkspaceSidebarScopeValue = "all" | "workspace";
export function WorkspaceSidebarScope({
  value,
  onChange,
  workspaceDisabled = false,
  className = "",
}: {
  value: WorkspaceSidebarScopeValue;
  onChange: (value: WorkspaceSidebarScopeValue) => void;
  workspaceDisabled?: boolean;
  className?: string;
}) {
  return (
    <div
      className={`shared-workspace-scope ${className}`}
      role="group"
      aria-label="Thread search scope"
    >
      <button
        type="button"
        aria-pressed={value === "all"}
        onClick={() => onChange("all")}
      >
        <IconWorld size={13} stroke={1.8} aria-hidden="true" />
        All Workspaces
      </button>
      <button
        type="button"
        aria-pressed={value === "workspace"}
        disabled={workspaceDisabled}
        onClick={() => onChange("workspace")}
      >
        <IconFolder size={13} stroke={1.8} aria-hidden="true" />
        This Workspace
      </button>
    </div>
  );
}
export function WorkspaceSidebarWorkspace({
  identity,
  state,
  actions,
  className = "",
}: {
  identity: ReactNode;
  state?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`shared-workspace-row ${className}`}>
      {identity}
      {state}
      {actions}
    </div>
  );
}
export function WorkspaceSidebarGroupHeading({
  label,
  count,
  className = "",
  icon = <IconActivity size={14} stroke={1.6} aria-hidden="true" />,
  headingLevel = 3,
}: {
  label: string;
  count: number;
  className?: string;
  icon?: ReactNode;
  headingLevel?: 2 | 3;
}) {
  const Heading = headingLevel === 2 ? "h2" : "h3";
  return (
    <div className={`shared-workspace-group-heading ${className}`}>
      {icon}
      <Heading>{label}</Heading>
      <span>{count}</span>
    </div>
  );
}
export function WorkspaceSidebarThreadContent({
  title,
  metadata,
  status,
}: {
  title: string;
  metadata: ReactNode;
  status?: ReactNode;
}) {
  return (
    <>
      <span className="shared-workspace-thread-copy">
        <strong>{title}</strong>
        <span>{metadata}</span>
      </span>
      {status ? (
        <span className="shared-workspace-thread-state">{status}</span>
      ) : null}
    </>
  );
}
