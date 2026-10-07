import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { IconCloud, IconRefresh, IconSettings } from "@tabler/icons-react";
import { controlPlaneProfiles } from "../api";
import type { ControlPlaneProfiles, SpaceSummary } from "../types";
import "./control-plane-connections.css";

export function controlPlaneState(status: string, unavailable = false) {
  if (unavailable) return { label: "Unknown", tone: "neutral" };
  switch (status) {
    case "connected":
      return { label: "Connected", tone: "success" };
    case "connecting":
      return { label: "Connecting", tone: "warning" };
    case "reconnecting":
      return { label: "Reconnecting", tone: "warning" };
    case "disconnected":
      return { label: "Disconnected", tone: "neutral" };
    case "revoked":
      return { label: "Revoked", tone: "danger" };
    default:
      return { label: "Unknown", tone: "neutral" };
  }
}

/** Display-only inventory. Navigation never selects or starts a runtime. */
export function ControlPlaneConnectionInventory({
  catalog,
  spaces,
  unavailable = false,
  disabled = false,
  onManageWorkspace,
  onManageProfiles,
}: {
  catalog: ControlPlaneProfiles;
  spaces: readonly Pick<SpaceSummary, "spaceId" | "targetId" | "displayName">[];
  unavailable?: boolean;
  disabled?: boolean;
  onManageWorkspace: (spaceId: string) => void;
  onManageProfiles: () => void;
}) {
  return (
    <>
      <div className="control-plane-connection-list">
        {catalog.connections.map((connection) => {
          const space =
            spaces.find((space) => space.spaceId === connection.targetId) ??
            spaces.find((space) => space.targetId === connection.targetId);
          const status = controlPlaneState(
            connection.status,
            unavailable || Boolean(catalog.connectionStatusUnavailable),
          );
          return (
            <article
              key={connection.targetId}
              className="control-plane-connection-row"
            >
              <div className="control-plane-connection-heading">
                <IconCloud size={18} aria-hidden="true" />
                <strong>{space?.displayName ?? "Unavailable workspace"}</strong>
                <span className={`status-chip tone-${status.tone}`}>
                  {status.label}
                </span>
                <Button
                  variant="secondary"
                  disabled={disabled || !space}
                  onClick={() => space && onManageWorkspace(space.spaceId)}
                  aria-label={`Manage Control Plane for ${space?.displayName ?? connection.targetId}`}
                >
                  <IconSettings size={15} aria-hidden="true" />
                  Manage
                </Button>
              </div>
              <dl>
                <dt>Project</dt>
                <dd>{connection.projectId ?? "Not reported"}</dd>
                <dt>Endpoint</dt>
                <dd>{connection.endpoint ?? "Not reported"}</dd>
              </dl>
              {!space ? (
                <p className="control-plane-connection-note">
                  This enrolled workspace is not in the current Desktop
                  inventory. No other workspace will be selected.
                </p>
              ) : null}
            </article>
          );
        })}
        {catalog.connections.length === 0 ? (
          <p className="inline-empty">
            No Control Plane enrollments are currently reported by this Desktop.
          </p>
        ) : null}
      </div>
      <div className="control-plane-bookmarks">
        <div className="section-heading">
          <div>
            <h4>Saved endpoints</h4>
            <p className="control-plane-connection-note">
              Bookmarks help with enrollment; they do not indicate a runtime
              connection.
            </p>
          </div>
          <Button
            variant="secondary"
            disabled={disabled}
            onClick={onManageProfiles}
          >
            Manage endpoints
          </Button>
        </div>
        {catalog.profiles.length ? (
          <ul>
            {catalog.profiles.map((profile) => (
              <li key={profile.id}>
                <strong>{profile.label}</strong>
                <span>{profile.endpoint}</span>
                {catalog.defaultProfile === profile.id ? (
                  <span className="status-chip tone-neutral">
                    Default bookmark
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="inline-empty">No saved endpoints.</p>
        )}
      </div>
    </>
  );
}

export function ControlPlaneConnections(
  props: Omit<
    Parameters<typeof ControlPlaneConnectionInventory>[0],
    "catalog" | "unavailable"
  >,
) {
  const [catalog, setCatalog] = useState<ControlPlaneProfiles | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  const [loading, setLoading] = useState(true);
  const alive = useRef(false),
    pending = useRef(false);
  async function refresh() {
    if (pending.current) return;
    pending.current = true;
    try {
      const next = await controlPlaneProfiles();
      if (alive.current) {
        setCatalog((current) => ({
          ...next,
          connections:
            next.connectionStatusUnavailable && !next.connections.length
              ? (current?.connections ?? [])
              : next.connections,
        }));
        setUnavailable(Boolean(next.connectionStatusUnavailable));
      }
    } catch {
      if (alive.current) setUnavailable(true);
    } finally {
      pending.current = false;
      if (alive.current) setLoading(false);
    }
  }
  useEffect(() => {
    alive.current = true;
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => {
      alive.current = false;
      clearInterval(timer);
    };
  }, []);
  return (
    <section
      className="overview-section"
      aria-label="Control Plane connections"
    >
      <div className="section-heading">
        <div>
          <p className="eyebrow">Enrolled workspace runtimes</p>
          <h3>Control Plane</h3>
        </div>
        <Button
          variant="secondary"
          onClick={() => void refresh()}
          disabled={loading}
        >
          <IconRefresh size={15} aria-hidden="true" />
          Refresh status
        </Button>
      </div>
      {loading && !catalog ? (
        <p className="inline-empty" role="status">
          Reading Control Plane enrollment status…
        </p>
      ) : null}
      {unavailable ? (
        <p className="managed-settings-notice error" role="alert">
          Control Plane status could not be refreshed. Any retained enrollment
          state is unknown until a successful read.
        </p>
      ) : null}
      {catalog ? (
        <ControlPlaneConnectionInventory
          {...props}
          catalog={catalog}
          unavailable={unavailable}
        />
      ) : unavailable ? (
        <Button
          variant="secondary"
          disabled={props.disabled}
          onClick={props.onManageProfiles}
        >
          Open Control Plane settings
        </Button>
      ) : null}
    </section>
  );
}
