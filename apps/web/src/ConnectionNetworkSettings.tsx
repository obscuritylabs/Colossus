import { useEffect, useId, useRef, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import { ProviderSetupSection } from "@colossus/ui/components/ProviderSetupSection";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import type {
  ConnectionAvailability,
  ConnectionAvailabilitySnapshot,
  ConnectionNetworkPolicy,
} from "./chat-settings";
import {
  SettingsEditorPage,
  type SettingsEditorNavigation,
} from "./settings-editor";
import "./connection-network.css";

const defaultPolicy: ConnectionNetworkPolicy = {
  mode: "allow_all",
  allowed_origins: [],
};

/** Match the server's origin-only contract; a list never grants transport or credentials. */
export function connectionNetworkPolicy(
  mode: ConnectionNetworkPolicy["mode"],
  text: string,
): ConnectionNetworkPolicy {
  if (mode === "allow_all") return { mode, allowed_origins: [] };
  const origins = [
    ...new Set(
      text
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean),
    ),
  ];
  if (origins.length > 256)
    throw new Error("Use at most 256 distinct origins.");
  for (const origin of origins) {
    let url: URL;
    try {
      url = new URL(origin);
    } catch {
      throw new Error(
        `Invalid origin: ${origin}. Use an exact HTTP or HTTPS origin, such as https://api.example.com.`,
      );
    }
    if (
      origin.length > 2048 ||
      !["http:", "https:"].includes(url.protocol) ||
      !url.hostname ||
      url.username ||
      url.password ||
      origin.includes("*") ||
      origin !== url.origin
    ) {
      throw new Error(
        `Invalid origin: ${origin}. Use only a canonical HTTP or HTTPS origin without a path, trailing slash, credentials, query, fragment, or wildcard.`,
      );
    }
  }
  return { mode, allowed_origins: origins };
}

export function ConnectionNetworkStatus({
  value,
}: {
  value?: ConnectionAvailability | undefined;
}) {
  if (!value || value.enabled) return null;
  return (
    <small className="connection-network-blocked" role="status">
      {value.reason ?? "Blocked by this project's Network access policy."}
    </small>
  );
}

function blockedConnections(availability?: ConnectionAvailabilitySnapshot) {
  const groups = {
    providers: "Provider",
    models: "Model",
    mcp: "MCP server",
    search: "Search service",
    roles: "Route",
  } as const;
  return Object.entries(groups).flatMap(([kind, label]) =>
    Object.entries(
      availability?.[kind as keyof ConnectionAvailabilitySnapshot] ?? {},
    )
      .filter(([, value]) => !value.enabled)
      .map(([id, value]) => ({
        id: `${kind}:${id}`,
        label: id,
        kind: label,
        reason:
          value.reason ?? "Blocked by the project's Network access policy.",
      })),
  );
}

export function ConnectionNetworkSettings({
  policy = defaultPolicy,
  availability,
  busy = false,
  navigationBusy = busy,
  readOnly = false,
  onSave,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: SettingsEditorNavigation & {
  policy?: ConnectionNetworkPolicy | undefined;
  availability?: ConnectionAvailabilitySnapshot | undefined;
  busy?: boolean;
  navigationBusy?: boolean;
  readOnly?: boolean;
  onSave: (policy: ConnectionNetworkPolicy) => void;
}) {
  const [inlineEditor, setInlineEditor] = useState(false);
  const editing = Boolean(editorLocation) || inlineEditor;
  const [mode, setMode] = useState(policy.mode);
  const [origins, setOrigins] = useState(policy.allowed_origins.join("\n"));
  const [error, setError] = useState("");
  const initial = useRef(
    JSON.stringify([policy.mode, policy.allowed_origins.join("\n")]),
  );
  const dirty = editing && JSON.stringify([mode, origins]) !== initial.current;
  const help = useId();
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);
  function close() {
    if (editorLocation) {
      onCloseEditor?.();
      return;
    }
    setInlineEditor(false);
    setMode(policy.mode);
    setOrigins(policy.allowed_origins.join("\n"));
    setError("");
    onCloseEditor?.();
  }
  const blocked = blockedConnections(availability);
  const columns: DataTableColumn<(typeof blocked)[number]>[] = [
    { id: "name", label: "Definition", value: (row) => row.label },
    { id: "kind", label: "Type", value: (row) => row.kind },
    { id: "reason", label: "Unavailable because", value: (row) => row.reason },
  ];
  if (!editing)
    return (
      <section
        className="connection-network-settings"
        aria-label="Network access"
      >
        <header className="catalog-heading">
          <div>
            <h2>Network access</h2>
            <p>
              Choose which configured provider, MCP, and search origins this
              project can use.
            </p>
          </div>
          <Button
            disabled={busy || readOnly}
            onClick={() =>
              onOpenEditor
                ? onOpenEditor({ action: "edit" })
                : setInlineEditor(true)
            }
          >
            Configure network access
          </Button>
        </header>
        <section
          className="connection-network-summary"
          aria-label="Saved network policy"
        >
          <h3>
            {policy.mode === "allow_all"
              ? "Allow all configured connections"
              : "Restricted origins"}
          </h3>
          <p>
            {policy.mode === "allow_all"
              ? "This project can use all configured provider, MCP, and search connections, including inherited Control Plane definitions."
              : "Only configured connections with an origin in this list can be used. Other definitions remain available for administration."}
          </p>
          {policy.mode === "allow_list" ? (
            policy.allowed_origins.length ? (
              <ul>
                {policy.allowed_origins.map((origin) => (
                  <li key={origin}>
                    <code>{origin}</code>
                  </li>
                ))}
              </ul>
            ) : (
              <p role="status">
                No origins are allowed. Connections that require an origin are
                blocked.
              </p>
            )
          ) : null}
          <p className="field-help">
            Transport requirements, credentials, and tool network permissions
            are managed separately. This setting does not test connectivity.
          </p>
        </section>
        {blocked.length ? (
          <section aria-label="Unavailable connections">
            <h3>Unavailable connections and routes</h3>
            <p className="field-help">
              Review this project's origin list or choose an available
              connection in its settings page. Blocking a provider also makes
              its models and routes unavailable.
            </p>
            <DataTable
              data={blocked}
              columns={columns}
              getRowId={(row) => row.id}
              label="Unavailable connections and routes"
              search={{
                columnId: "name",
                label: "Search unavailable definitions",
              }}
            />
          </section>
        ) : null}
      </section>
    );
  return (
    <SettingsEditorPage
      title="Edit network access"
      description="Changes affect every user and chat in this project. Control Plane catalog definitions remain available to other projects."
      backLabel="Network access"
      busy={navigationBusy}
      onBack={close}
    >
      <form
        className="connection-network-editor"
        onSubmit={(event) => {
          event.preventDefault();
          if (busy || readOnly || !dirty) return;
          try {
            const next = connectionNetworkPolicy(mode, origins);
            setError("");
            onSave(next);
          } catch (cause) {
            setError(
              cause instanceof Error ? cause.message : "Check the origin list.",
            );
          }
        }}
      >
        {error ? (
          <p role="alert" className="alert">
            {error}
          </p>
        ) : null}
        <fieldset
          disabled={busy || readOnly}
          className="settings-editor-fields"
        >
          <ProviderSetupSection
            title="Connection policy"
            description="Applies to configured provider, MCP, and search endpoints. It does not grant credentials or additional tool network access."
          >
            <label>
              Access mode
              <DropdownSelect
                aria-label="Connection access mode"
                value={mode}
                onChange={(event) =>
                  setMode(event.target.value as ConnectionNetworkPolicy["mode"])
                }
              >
                <option value="allow_all">
                  Allow all configured connections
                </option>
                <option value="allow_list">Restrict to an origin list</option>
              </DropdownSelect>
            </label>
          </ProviderSetupSection>
          {mode === "allow_list" ? (
            <ProviderSetupSection
              title="Allowed origins"
              description="Enter one exact origin per line, including required OAuth issuer and token origins. An empty list blocks every connection that requires an origin."
            >
              <label>
                Origins
                <textarea
                  aria-label="Allowed connection origins"
                  aria-describedby={help}
                  rows={7}
                  value={origins}
                  spellCheck={false}
                  autoCapitalize="off"
                  autoCorrect="off"
                  onChange={(event) => setOrigins(event.target.value)}
                />
              </label>
              <p id={help} className="field-help">
                Use a scheme, host, and optional port, such as
                https://api.example.com or https://mcp.example.com:8443. No
                paths, trailing slashes, credentials, queries, fragments, or
                wildcards. HTTP entries do not override the deployment's
                transport requirements.
              </p>
            </ProviderSetupSection>
          ) : null}
        </fieldset>
        <div className="chat-settings-actions">
          <Button disabled={navigationBusy} onClick={close}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            disabled={busy || readOnly || !dirty}
          >
            {navigationBusy ? "Saving…" : "Save network access"}
          </Button>
        </div>
      </form>
    </SettingsEditorPage>
  );
}
