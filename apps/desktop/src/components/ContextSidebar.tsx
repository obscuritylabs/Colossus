import {
  IconActivity,
  IconArchive,
  IconCircleCheck,
  IconClock,
  IconPlugConnected,
  IconShieldCheck,
  IconTopologyStar3,
} from "@tabler/icons-react";

import type { WorkspaceSurface } from "./ProductRail";
import type { ConnectionStatus } from "../types";

interface ContextSidebarProps {
  surface: Exclude<WorkspaceSurface, "work">;
  connection: ConnectionStatus;
  runCount: number;
  activeCount: number;
  artifactCount: number;
  activityCount: number;
}

const COPY = {
  fleet: {
    kicker: "Command and control",
    title: "Fleet",
    description: "Operational workload and connected-agent topology.",
  },
  library: {
    kicker: "Released outputs",
    title: "Library",
    description: "Artifacts that crossed the public API release boundary.",
  },
  activity: {
    kicker: "Audit-friendly feed",
    title: "Activity",
    description: "Bounded operational events without hidden runtime details.",
  },
  settings: {
    kicker: "Desktop runtime",
    title: "Settings",
    description: "Connection health and security posture for this app.",
  },
} as const;

export function ContextSidebar({
  surface,
  connection,
  runCount,
  activeCount,
  artifactCount,
  activityCount,
}: ContextSidebarProps) {
  const copy = COPY[surface];
  return (
    <aside className="context-sidebar" aria-label={`${copy.title} summary`}>
      <header>
        <p>{copy.kicker}</p>
        <h1>{copy.title}</h1>
        <span>{copy.description}</span>
      </header>
      <dl className="context-metrics">
        <div>
          <dt>
            <IconTopologyStar3 size={16} stroke={1.7} aria-hidden="true" />
            Active work
          </dt>
          <dd>{activeCount}</dd>
        </div>
        <div>
          <dt>
            <IconCircleCheck size={16} stroke={1.7} aria-hidden="true" />
            Cached runs
          </dt>
          <dd>{runCount}</dd>
        </div>
        <div>
          <dt>
            <IconArchive size={16} stroke={1.7} aria-hidden="true" />
            Artifacts
          </dt>
          <dd>{artifactCount}</dd>
        </div>
        <div>
          <dt>
            <IconActivity size={16} stroke={1.7} aria-hidden="true" />
            Events
          </dt>
          <dd>{activityCount}</dd>
        </div>
      </dl>
      <div className="context-runtime">
        <p>
          <IconPlugConnected size={16} stroke={1.7} aria-hidden="true" />
          Runtime
        </p>
        <strong>
          {connection.state === "connected"
            ? "Local agent online"
            : "Agent offline"}
        </strong>
        <span>{connection.message}</span>
      </div>
      <div className="context-security">
        <IconShieldCheck size={18} stroke={1.7} aria-hidden="true" />
        <div>
          <strong>Renderer-safe view</strong>
          <span>
            Opaque IDs, hashes, credentials, and raw paths are not displayed.
          </span>
        </div>
      </div>
      <p className="context-updated">
        <IconClock size={14} stroke={1.7} aria-hidden="true" />
        Live state from this desktop session
      </p>
    </aside>
  );
}
