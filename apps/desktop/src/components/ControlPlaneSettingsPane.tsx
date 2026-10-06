import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { IconCloud } from "@tabler/icons-react";
import { Button, CatalogInventory, TextInput } from "@colossus/ui";

interface Profile {
  id: string;
  label: string;
  endpoint: string;
}
export interface ControlPlaneProfiles {
  revision: number;
  profiles: Profile[];
  defaultProfile: string | null;
  connections: {
    targetId: string;
    status: string;
    projectId: string | null;
    endpoint: string | null;
    sharedSessions: boolean;
  }[];
}

export function ControlPlaneSettingsPane() {
  const [catalog, setCatalog] = useState<ControlPlaneProfiles | null>(null);
  const [label, setLabel] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    const refresh = () => {
      if (busyRef.current) return;
      void invoke<ControlPlaneProfiles>("control_plane_profiles")
        .then((value) => {
          if (alive)
            setCatalog((current) =>
              current && current.revision > value.revision ? current : value,
            );
        })
        .catch(() => {
          if (alive) setError("Control Plane settings are unavailable.");
        });
    };
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
  async function save(profiles: Profile[], defaultProfile: string | null) {
    if (!catalog || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setError("");
    try {
      const next = await invoke<ControlPlaneProfiles>(
        "save_control_plane_profiles",
        {
          catalog: { revision: catalog.revision, profiles, defaultProfile },
        },
      );
      setCatalog(next);
      setEditorOpen(false);
      setEditing(null);
      setLabel("");
      setEndpoint("");
    } catch (failure) {
      setError(
        typeof failure === "object" && failure !== null && "message" in failure
          ? String(failure.message)
          : "Connection settings could not be saved. Reload and try again.",
      );
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }
  const profiles = catalog?.profiles ?? [];
  if (!catalog)
    return (
      <p role={error ? "alert" : "status"}>
        {error || "Loading Control Plane connections…"}
      </p>
    );
  return (
    <CatalogInventory
      kind={{
        title: "Control Plane",
        singular: "connection",
        plural: "connections",
        column: "Connection",
        description:
          "Save endpoints for your workspaces. Enrollment and sharing are managed in each workspace.",
        connectionLabel: "Default",
        usageLabel: "Enrollment",
        Icon: IconCloud,
        className: "providers-settings control-plane-settings",
      }}
      summary={`${profiles.length} saved ${profiles.length === 1 ? "connection" : "connections"}`}
      busy={busy}
      editing={editorOpen}
      onAdd={() => {
        setEditorOpen(true);
        setEditing(null);
        setLabel("");
        setEndpoint("");
      }}
      emptyDescription="Add an endpoint, then enroll a workspace from its Control Plane settings."
      rows={profiles.map((profile) => ({
        id: profile.id,
        label: profile.label,
        name: profile.label,
        description: profile.endpoint,
        searchText: `${profile.label} ${profile.endpoint}`,
        connection:
          catalog.defaultProfile === profile.id ? (
            "Default"
          ) : (
            <Button
              type="button"
              disabled={busy}
              onClick={() => void save(profiles, profile.id)}
            >
              Use by default
            </Button>
          ),
        usage: "Per workspace",
        details: (
          <p>
            Workspace runtimes keep their own enrollment and sharing choice.
          </p>
        ),
        onEdit: () => {
          setEditing(profile.id);
          setEditorOpen(true);
          setLabel(profile.label);
          setEndpoint(profile.endpoint);
        },
        onDelete: () =>
          void save(
            profiles.filter((item) => item.id !== profile.id),
            catalog.defaultProfile === profile.id
              ? null
              : catalog.defaultProfile,
          ),
      }))}
      footer={
        catalog.connections.length ? (
          <section>
            <h4>Connected workspace runtimes</h4>
            {catalog.connections.map((connection) => (
              <p key={connection.targetId}>
                <strong>{connection.projectId ?? "Workspace"}</strong> ·{" "}
                {connection.status} ·{" "}
                {connection.sharedSessions
                  ? "Workspace history shared"
                  : "Local history private"}
              </p>
            ))}
          </section>
        ) : null
      }
    >
      {error ? (
        <p className="managed-settings-notice error" role="alert">
          {error}
        </p>
      ) : null}
      {editorOpen ? (
        <form
          className="mcp-editor catalog-editor provider-editor"
          onSubmit={(event) => {
            event.preventDefault();
            const id = editing ?? crypto.randomUUID();
            void save(
              [
                ...profiles.filter((profile) => profile.id !== id),
                { id, label: label.trim(), endpoint: endpoint.trim() },
              ],
              catalog.defaultProfile ?? id,
            );
          }}
        >
          <div className="provider-editor-heading">
            <IconCloud size={24} aria-hidden="true" />
            <div>
              <h4>{editing ? "Edit connection" : "Add connection"}</h4>
              <p>
                Use HTTPS, or HTTP on loopback for local development. Workspace
                enrollments are managed separately.
              </p>
            </div>
          </div>
          <div className="provider-editor-section">
            <div className="provider-editor-grid">
              <label>
                Connection name
                <TextInput
                  value={label}
                  onChange={(event) => setLabel(event.target.value)}
                  maxLength={128}
                  placeholder="Team Control Plane"
                  disabled={busy}
                  required
                  autoFocus
                />
              </label>
              <label>
                Web endpoint
                <TextInput
                  value={endpoint}
                  onChange={(event) => setEndpoint(event.target.value)}
                  maxLength={2048}
                  placeholder="https://control-plane.example.com"
                  disabled={busy}
                  required
                />
              </label>
            </div>
          </div>
          <div className="mcp-editor-actions">
            <Button
              type="submit"
              disabled={busy || !label.trim() || !endpoint.trim()}
            >
              {busy ? "Saving…" : "Save connection"}
            </Button>
            <Button
              type="button"
              disabled={busy}
              onClick={() => {
                setEditorOpen(false);
                setEditing(null);
                setLabel("");
                setEndpoint("");
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      ) : null}
    </CatalogInventory>
  );
}
