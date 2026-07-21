import {
  IconActivity,
  IconAlertTriangle,
  IconArchive,
  IconArrowUpRight,
  IconCheck,
  IconCircle,
  IconFileText,
  IconPlugConnected,
  IconRefresh,
  IconRobot,
  IconShieldCheck,
  IconTopologyStar3,
} from "@tabler/icons-react";

import type { OperationalActivityItem, PresentedArtifact } from "../presenters";
import { agentRoleLabel, presentRunStatus } from "../presenters";
import type { Run } from "../types";
import type { ConnectionStatus } from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { AgentFlow } from "./AgentFlow";
import type { WorkspaceSurface } from "./ProductRail";

interface OperationsSurfaceProps {
  surface: Exclude<WorkspaceSurface, "work">;
  connection: ConnectionStatus;
  connecting: boolean;
  runs: readonly Run[];
  artifacts: readonly PresentedArtifact[];
  activity: readonly OperationalActivityItem[];
  demoParticipants: readonly AgentParticipant[] | null;
  onConnect: () => void;
  onOpenRun: (run: Run) => void;
}

function SurfaceHeader({
  eyebrow,
  title,
  description,
}: {
  eyebrow: string;
  title: string;
  description: string;
}) {
  return (
    <header className="surface-header overview-header">
      <div className="surface-title-copy">
        <p className="surface-breadcrumb">{eyebrow}</p>
        <h2>{title}</h2>
        <span>{description}</span>
      </div>
    </header>
  );
}

function FleetView({
  runs,
  demoParticipants,
  onOpenRun,
}: Pick<OperationsSurfaceProps, "runs" | "demoParticipants" | "onOpenRun">) {
  const active = runs.filter((run) =>
    ["queued", "running", "waiting", "cancelling"].includes(run.status),
  );
  return (
    <>
      <SurfaceHeader
        eyebrow="Fleet / Overview"
        title="Agent fleet"
        description="Coordinate active work while keeping every handoff visible."
      />
      <div className="overview-scroll">
        {demoParticipants !== null ? (
          <section className="overview-section">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Live topology</p>
                <h3>Desktop hardening squad</h3>
              </div>
              <span className="status-chip tone-progress">4 connected</span>
            </div>
            <AgentFlow participants={demoParticipants} />
          </section>
        ) : (
          <section className="honest-empty">
            <IconTopologyStar3 size={28} stroke={1.4} aria-hidden="true" />
            <div>
              <strong>Agent identities are not exposed by this API yet</strong>
              <p>
                Current run roles are workload labels, not stable
                connected-agent identities. Fleet topology will activate when
                the public API exposes an authorized agent inventory.
              </p>
            </div>
          </section>
        )}

        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Current workload</p>
              <h3>{active.length} active work items</h3>
            </div>
          </div>
          <div className="workload-grid">
            {active.map((run) => {
              const status = presentRunStatus(run.status);
              return (
                <button
                  type="button"
                  key={run.runId}
                  onClick={() => onOpenRun(run)}
                >
                  <span className={`workload-icon tone-${status.tone}`}>
                    <IconRobot size={19} stroke={1.7} aria-hidden="true" />
                  </span>
                  <span>
                    <strong>{agentRoleLabel(run.role)}</strong>
                    <small>{status.copy}</small>
                  </span>
                  <IconArrowUpRight size={17} stroke={1.6} aria-hidden="true" />
                </button>
              );
            })}
            {active.length === 0 ? (
              <p className="inline-empty">
                No active work is cached in this session.
              </p>
            ) : null}
          </div>
        </section>
      </div>
    </>
  );
}

function LibraryView({ artifacts }: Pick<OperationsSurfaceProps, "artifacts">) {
  return (
    <>
      <SurfaceHeader
        eyebrow="Library / Released artifacts"
        title="Artifact library"
        description="Safe metadata for files and outputs released through run messages."
      />
      <div className="overview-scroll">
        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Available in this session</p>
              <h3>{artifacts.length} released artifacts</h3>
            </div>
          </div>
          <div className="artifact-library-list">
            {artifacts.map((artifact) => (
              <article key={artifact.key}>
                <span className="library-file-icon" aria-hidden="true">
                  <IconFileText size={20} stroke={1.6} />
                </span>
                <div>
                  <strong>{artifact.fileName}</strong>
                  <span>
                    {artifact.typeLabel} · {artifact.sizeLabel} ·{" "}
                    {artifact.purposeLabel}
                  </span>
                </div>
                <span
                  className={`status-chip tone-${artifact.canOpen ? "success" : "attention"}`}
                >
                  {artifact.stateLabel}
                </span>
              </article>
            ))}
            {artifacts.length === 0 ? (
              <div className="honest-empty compact-empty">
                <IconArchive size={25} stroke={1.4} aria-hidden="true" />
                <div>
                  <strong>No released artifacts yet</strong>
                  <p>
                    Run outputs will appear after they cross the public release
                    boundary.
                  </p>
                </div>
              </div>
            ) : null}
          </div>
        </section>
      </div>
    </>
  );
}

function ActivityView({ activity }: Pick<OperationsSurfaceProps, "activity">) {
  return (
    <>
      <SurfaceHeader
        eyebrow="Activity / Operational feed"
        title="What the system is doing"
        description="A bounded, newest-first feed of released operational events."
      />
      <div className="overview-scroll">
        <section className="overview-section activity-section">
          <div className="activity-list">
            {activity.map((item) => (
              <article key={item.key}>
                <span
                  className={`activity-marker tone-${item.tone}`}
                  aria-hidden="true"
                >
                  {item.tone === "success" ? (
                    <IconCheck size={16} stroke={2} />
                  ) : item.tone === "danger" || item.tone === "attention" ? (
                    <IconAlertTriangle size={16} stroke={1.8} />
                  ) : item.kind === "tool" ? (
                    <IconActivity size={16} stroke={1.7} />
                  ) : (
                    <IconCircle size={13} stroke={1.7} />
                  )}
                </span>
                <div>
                  <header>
                    <strong>{item.title}</strong>
                    <time dateTime={item.createdAt}>{item.createdLabel}</time>
                  </header>
                  {item.detail !== null ? <p>{item.detail}</p> : null}
                  <span>{item.agentLabel}</span>
                </div>
                {item.stateLabel !== null ? (
                  <span className={`status-chip tone-${item.tone}`}>
                    {item.stateLabel}
                  </span>
                ) : null}
              </article>
            ))}
            {activity.length === 0 ? (
              <p className="inline-empty">
                No released activity is cached yet.
              </p>
            ) : null}
          </div>
        </section>
      </div>
    </>
  );
}

function SettingsView({
  connection,
  connecting,
  onConnect,
}: Pick<OperationsSurfaceProps, "connection" | "connecting" | "onConnect">) {
  return (
    <>
      <SurfaceHeader
        eyebrow="Settings / Runtime"
        title="Desktop connection"
        description="Inspect the connection state without exposing native credentials."
      />
      <div className="overview-scroll settings-scroll">
        <section className="settings-card">
          <div className="settings-card-icon">
            <IconPlugConnected size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Local agent</p>
            <h3>
              {connection.state === "connected"
                ? "Connected"
                : "Connection required"}
            </h3>
            <p>{connection.message}</p>
          </div>
          <button
            className="button secondary"
            type="button"
            disabled={connecting}
            onClick={onConnect}
          >
            <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
            {connecting ? "Connecting…" : "Reconnect"}
          </button>
        </section>
        <section className="settings-card security-settings-card">
          <div className="settings-card-icon">
            <IconShieldCheck size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Security boundary</p>
            <h3>Native-owned credentials and IPC</h3>
            <p>
              Enrollment material remains in the OS keyring. The renderer
              receives typed command results only, under the Tauri capability
              and CSP boundary.
            </p>
          </div>
        </section>
      </div>
    </>
  );
}

export function OperationsSurface(props: OperationsSurfaceProps) {
  return (
    <main className="operations-surface" id="primary-workspace" tabIndex={-1}>
      {props.surface === "fleet" ? <FleetView {...props} /> : null}
      {props.surface === "library" ? (
        <LibraryView artifacts={props.artifacts} />
      ) : null}
      {props.surface === "activity" ? (
        <ActivityView activity={props.activity} />
      ) : null}
      {props.surface === "settings" ? (
        <SettingsView
          connection={props.connection}
          connecting={props.connecting}
          onConnect={props.onConnect}
        />
      ) : null}
    </main>
  );
}
