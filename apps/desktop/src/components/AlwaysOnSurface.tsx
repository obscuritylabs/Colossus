import { Button, WorkspaceSurfaceHeader } from "@colossus/ui";
import { IconArrowUpRight, IconClock, IconRefresh } from "@tabler/icons-react";
import { useState } from "react";
import { shortDateLabel } from "../presenters";
import type { AlwaysOnWorkspace } from "../types";
import "./always-on-surface.css";

export interface AlwaysOnSurfaceProps {
  rows: readonly AlwaysOnWorkspace[] | null;
  failed: boolean;
  disabled: boolean;
  onRefresh: () => void;
  onOpenConversation: (spaceId: string, sessionId: string) => Promise<boolean>;
  onSelectWorkspace: (spaceId: string) => void;
}

export function AlwaysOnSurface({
  rows,
  failed,
  disabled,
  onRefresh,
  onOpenConversation,
  onSelectWorkspace,
}: AlwaysOnSurfaceProps) {
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const sorted = [...(rows ?? [])].sort((left, right) =>
    left.displayName.localeCompare(right.displayName),
  );
  const enrolled = sorted.filter((row) => row.state === "enrolled");
  const unavailable = sorted.filter((row) => row.state === "unavailable");

  async function open(spaceId: string, sessionId: string, name: string) {
    setOpening(true);
    setOpenError(null);
    try {
      if (!(await onOpenConversation(spaceId, sessionId))) {
        setOpenError(`The enrolled conversation in ${name} is unavailable.`);
      }
    } finally {
      setOpening(false);
    }
  }

  return (
    <>
      <WorkspaceSurfaceHeader
        eyebrow="Workspaces"
        title="Always On Agents"
        description="Monitor scheduled check-ins and open the conversations that receive them."
      />
      <div className="overview-scroll always-on-inventory" tabIndex={0}>
        <div className="always-on-inventory-toolbar">
          {rows !== null ? (
            <span>
              {enrolled.length} {enrolled.length === 1 ? "agent" : "agents"}
              {unavailable.length > 0
                ? ` · ${unavailable.length} ${unavailable.length === 1 ? "Workspace" : "Workspaces"} unavailable`
                : ""}
            </span>
          ) : null}
          <Button variant="secondary" onClick={onRefresh}>
            <IconRefresh size={16} aria-hidden="true" />
            Refresh
          </Button>
        </div>
        {failed ? (
          <p role="alert">
            Always On Agents could not be loaded. Try refreshing.
          </p>
        ) : rows === null ? (
          <p role="status">Checking Always On Agents…</p>
        ) : enrolled.length === 0 ? (
          <p className="inline-empty">
            {unavailable.length === 0
              ? "No Always On Agents are enrolled."
              : "No Always On Agents were found in the available Workspaces."}{" "}
            To enroll an agent, open a completed Execute conversation in a
            Managed Local Workspace and choose Always On.
          </p>
        ) : null}
        {enrolled.length > 0 ? (
          <ul className="always-on-workspace-list" aria-label="Enrolled agents">
            {enrolled.map(({ spaceId, displayName, profile }) => (
              <li key={spaceId}>
                <div className="always-on-workspace-identity">
                  <IconClock size={18} aria-hidden="true" />
                  <h3>{displayName}</h3>
                </div>
                <div className="always-on-workspace-status">
                  <span
                    className={`status-chip ${profile.enabled ? "tone-success" : "tone-neutral"}`}
                  >
                    {profile.enabled ? "Enabled" : "Paused"}
                  </span>
                  {profile.pending !== null ? (
                    <span>
                      {profile.pending.trigger === "manual"
                        ? "Manual"
                        : "Scheduled"}{" "}
                      wake pending
                    </span>
                  ) : null}
                  {profile.enabled ? (
                    <span>
                      Next wake{" "}
                      <time dateTime={profile.next_wake_at}>
                        {shortDateLabel(profile.next_wake_at)}
                      </time>
                    </span>
                  ) : null}
                  {profile.last_wake !== null ? (
                    <span>
                      Last wake {profile.last_wake.status.replaceAll("_", " ")}
                      {" · "}
                      <time dateTime={profile.last_wake.completed_at}>
                        {shortDateLabel(profile.last_wake.completed_at)}
                      </time>
                    </span>
                  ) : null}
                </div>
                <Button
                  variant="secondary"
                  aria-label={`Open conversation in ${displayName}`}
                  disabled={disabled || opening}
                  onClick={() =>
                    void open(spaceId, profile.session_id, displayName)
                  }
                >
                  Open conversation
                  <IconArrowUpRight size={16} aria-hidden="true" />
                </Button>
              </li>
            ))}
          </ul>
        ) : null}
        {openError !== null ? <p role="alert">{openError}</p> : null}
        {unavailable.length > 0 ? (
          <section
            className="always-on-unavailable"
            aria-labelledby="always-on-unavailable"
          >
            <h3 id="always-on-unavailable">Unavailable Workspaces</h3>
            <p>Open a Workspace to check whether it has an Always On Agent.</p>
            <ul className="always-on-workspace-list">
              {unavailable.map(({ spaceId, displayName }) => (
                <li key={spaceId}>
                  <strong>{displayName}</strong>
                  <Button
                    variant="secondary"
                    aria-label={`Open Workspace ${displayName}`}
                    disabled={disabled || opening}
                    onClick={() => onSelectWorkspace(spaceId)}
                  >
                    Open Workspace
                  </Button>
                </li>
              ))}
            </ul>
          </section>
        ) : null}
      </div>
    </>
  );
}
