import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button, RadioGroup, TextInput } from "@colossus/ui";
import {
  IconCloud,
  IconPlugConnected,
  IconPlugConnectedX,
  IconRefresh,
  IconShieldLock,
} from "@tabler/icons-react";
import "./cloud-connection.css";
import type { ControlPlaneProfiles } from "./ControlPlaneSettingsPane";
interface CloudStatus {
  targetId: string;
  status:
    "connecting" | "connected" | "reconnecting" | "disconnected" | "revoked";
  nodeId: string | null;
  projectId: string | null;
  endpoint: string | null;
  hostId?: string | null;
  workspaceId?: string | null;
  sharedSessions?: boolean;
  sharedContinuation?: boolean;
  sharingSupported?: boolean;
}
export function CloudConnectionPane({ targetId }: { targetId: string }) {
  const [status, setStatus] = useState<CloudStatus | null>(null),
    [url, setUrl] = useState(""),
    [token, setToken] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [sharing, setSharing] = useState("private");
  const operation = useRef(0),
    busyRef = useRef(false);
  useEffect(() => {
    let alive = true;
    void invoke<ControlPlaneProfiles>("control_plane_profiles")
      .then((catalog) => {
        const profile = catalog.profiles.find(
          (item) => item.id === catalog.defaultProfile,
        );
        if (alive && profile)
          setUrl(
            (current) =>
              current || `${profile.endpoint.replace(/\/$/, "")}/api/enroll`,
          );
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [targetId]);
  useEffect(() => {
    let alive = true,
      refreshing = false;
    const refresh = () => {
      if (refreshing || busyRef.current) return;
      refreshing = true;
      const generation = operation.current;
      void invoke<CloudStatus>("cloud_status", { targetId })
        .then((status) => {
          if (alive && generation === operation.current) setStatus(status);
        })
        .catch((error) => {
          if (alive && generation === operation.current)
            setError(
              typeof error === "object" && error !== null && "message" in error
                ? String(error.message)
                : "Control Plane enrollment is unavailable.",
            );
        })
        .finally(() => {
          refreshing = false;
        });
    };
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      alive = false;
      operation.current++;
      clearInterval(timer);
    };
  }, [targetId]);
  async function action(
    command: string,
    parameters: Record<string, unknown> = { targetId },
  ) {
    if (busyRef.current) return;
    busyRef.current = true;
    const generation = ++operation.current;
    setBusy(true);
    setError("");
    // The native command owns this one-use value as soon as it is submitted.
    if (command === "cloud_enroll") setToken("");
    try {
      const next = await invoke<CloudStatus>(command, parameters);
      if (operation.current === generation) setStatus(next);
    } catch (error) {
      if (operation.current === generation)
        setError(
          typeof error === "object" && error !== null && "message" in error
            ? String(error.message)
            : "The Control Plane connection could not be changed.",
        );
    } finally {
      busyRef.current = false;
      if (operation.current === generation) setBusy(false);
    }
  }
  const active =
    status &&
    ["connected", "connecting", "reconnecting"].includes(status.status);
  useEffect(() => {
    setSharing(
      status?.sharedSessions
        ? status.sharedContinuation
          ? "continue"
          : "read"
        : "private",
    );
  }, [status?.sharedSessions, status?.sharedContinuation, targetId]);
  return (
    <section className="cloud-settings">
      <header>
        <div className="cloud-settings-icon">
          <IconCloud size={26} />
        </div>
        <div>
          <h2>Colossus Control Plane</h2>
          <p>Make this runtime available to your project's Control Plane.</p>
        </div>
      </header>
      {error && (
        <p className="cloud-settings-error" role="alert">
          {error}
        </p>
      )}
      <div className="cloud-settings-state" role="status" aria-live="polite">
        <span
          className={`cloud-state-dot ${status?.status === "connected" ? "connected" : ""}`}
        />
        <strong>
          {status
            ? status.status[0]?.toUpperCase() + status.status.slice(1)
            : error
              ? "Enrollment unavailable"
              : "Checking enrollment…"}
        </strong>
        {status?.projectId && <span>{status.projectId}</span>}
      </div>
      {!status && error && (
        <Button
          variant="secondary"
          disabled={busy}
          onClick={() => void action("cloud_status")}
        >
          Retry status
        </Button>
      )}
      {status?.nodeId ? (
        <>
          <dl>
            <dt>Control Plane endpoint</dt>
            <dd>{status.endpoint}</dd>
            <dt>Node identity</dt>
            <dd>{status.nodeId}</dd>
            {status.hostId && (
              <>
                <dt>Host identity</dt>
                <dd>{status.hostId}</dd>
              </>
            )}
          </dl>
          <div className="cloud-settings-actions">
            {active ? (
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => void action("cloud_disconnect")}
              >
                <IconPlugConnectedX size={17} />
                Disconnect
              </Button>
            ) : (
              <Button
                variant="primary"
                disabled={busy || status.status === "revoked"}
                onClick={() => void action("cloud_connect")}
              >
                <IconRefresh size={17} />
                Reconnect runtime
              </Button>
            )}
            <Button
              variant="danger"
              disabled={busy || status.status === "revoked"}
              onClick={() => void action("cloud_revoke")}
            >
              Revoke enrollment
            </Button>
            <Button
              variant="secondary"
              disabled={busy || !!active}
              onClick={() => void action("cloud_forget")}
            >
              Forget enrollment
            </Button>
          </div>
          <p className="cloud-settings-note">
            {status.status === "revoked"
              ? "Forget this enrollment, then use a new invitation to connect again. Accepted tasks remain under local runtime policy."
              : "Disconnecting leaves accepted tasks running locally. Revocation removes this runtime's Control Plane authority until it is enrolled again."}
          </p>
          {status.sharingSupported && (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void action("cloud_set_workspace_sharing", {
                  targetId,
                  enabled: sharing !== "private",
                  allowContinuation: sharing === "continue",
                });
              }}
            >
              <div>
                <p>Desktop conversation sharing</p>
                <RadioGroup
                  value={sharing}
                  onValueChange={setSharing}
                  disabled={busy || status.status === "revoked"}
                  aria-label="Desktop conversation sharing"
                  options={[
                    {
                      value: "private",
                      label: "Control Plane conversations only",
                    },
                    {
                      value: "read",
                      label: "Share Desktop history for viewing",
                    },
                    {
                      value: "continue",
                      label: "Share history and allow continuation",
                    },
                  ]}
                />
              </div>
              <p className="cloud-settings-note">
                Sharing includes existing and future conversations in this
                workspace. Enabling sharing requires local confirmation.
                Disabling it stops future synchronization; history already
                synchronized remains in the Control Plane.
              </p>
              <Button
                type="submit"
                variant="secondary"
                disabled={busy || status.status === "revoked"}
              >
                Save conversation sharing
              </Button>
            </form>
          )}
        </>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void action("cloud_enroll", {
              request: { targetId, enrollmentUrl: url, token },
            });
          }}
        >
          <label>
            Enrollment URL
            <TextInput
              type="url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              placeholder="https://control-plane.example.com/api/enroll"
              required
              disabled={busy || !status}
            />
          </label>
          <label>
            One-use invitation
            <TextInput
              type="password"
              value={token}
              onChange={(event) => setToken(event.target.value)}
              placeholder="Paste the invitation from Runtime fleet"
              required
              maxLength={64}
              pattern="[a-fA-F0-9]{64}"
              autoComplete="off"
              spellCheck={false}
              disabled={busy || !status}
            />
          </label>
          <Button
            type="submit"
            variant="primary"
            disabled={busy || !status || !url || token.length !== 64}
          >
            <IconPlugConnected size={17} />
            {busy ? "Enrolling runtime…" : "Enroll and connect"}
          </Button>
          {error && (
            <Button
              type="button"
              variant="secondary"
              disabled={busy}
              onClick={() => void action("cloud_forget")}
            >
              Reset local enrollment
            </Button>
          )}
        </form>
      )}
      <aside>
        <IconShieldLock size={19} />
        <p>
          Only authorized project members can submit tasks. This runtime's
          roles, tools, policy, and approval rules still apply. Managed Local
          work depends on Desktop staying open; use a CLI daemon for persistent
          execution.
        </p>
      </aside>
    </section>
  );
}
