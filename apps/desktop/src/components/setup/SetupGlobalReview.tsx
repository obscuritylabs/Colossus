import type { ReactNode } from "react";
import { IconChevronDown } from "@tabler/icons-react";
import type { SetupGlobalSettings } from "../../setupPackages";

export function SetupGlobalReview({
  settings,
  applyDefaults,
  onApplyDefaults,
  disabled,
}: {
  settings: SetupGlobalSettings | undefined;
  applyDefaults: boolean;
  onApplyDefaults: (value: boolean) => void;
  disabled: boolean;
}) {
  if (!settings) return null;
  const { defaults, mcpServers, searchProviders, telemetryProfiles } = settings;
  function group(title: string, children: ReactNode, count: number) {
    return count ? (
      <section className="setup-global-group" aria-label={title}>
        <h3>
          {title} <span>{count}</span>
        </h3>
        {children}
      </section>
    ) : null;
  }
  function details(id: string, label: string, summary: string, value: unknown) {
    return (
      <details key={id} className="setup-global-resource">
        <summary>
          <strong>{label}</strong>
          <span>{summary}</span>
          <IconChevronDown size={14} aria-hidden="true" />
        </summary>
        <pre>{JSON.stringify(value, null, 2)}</pre>
      </details>
    );
  }
  return (
    <div className="setup-global-review">
      {defaults ? (
        <section className="setup-global-group" aria-label="Global defaults">
          <h3>Global defaults</h3>
          <dl className="setup-detail-grid">
            <div>
              <dt>Access profile</dt>
              <dd>
                {defaults.accessProfile
                  ? {
                      minimal: "Minimal",
                      pinned: "Pinned",
                      development: "Development",
                      allow_all: "Allow all",
                    }[defaults.accessProfile]
                  : "Built-in default"}
              </dd>
            </div>
            <div>
              <dt>Sandbox boundary</dt>
              <dd>
                {defaults.executionBoundary
                  ? {
                      full_access: "Full access",
                      workspace_isolated: "Workspace isolation",
                      offline_isolated: "Offline isolation",
                    }[defaults.executionBoundary]
                  : "Built-in default"}
              </dd>
            </div>
            <div>
              <dt>Terminal</dt>
              <dd>
                {defaults.terminalEnabled === null
                  ? "Built-in default"
                  : defaults.terminalEnabled
                    ? "Enabled"
                    : "Disabled"}
              </dd>
            </div>
          </dl>
          {defaults.fieldOverrides.length ? (
            <details>
              <summary>
                View {defaults.fieldOverrides.length} limits and settings
              </summary>
              <dl className="setup-global-fields">
                {defaults.fieldOverrides.map((field) => (
                  <div key={field.fieldId}>
                    <dt>{field.fieldId}</dt>
                    <dd>
                      <code>{JSON.stringify(field.value)}</code>
                    </dd>
                  </div>
                ))}
              </dl>
            </details>
          ) : null}
          <label className="setup-replace">
            <input
              type="checkbox"
              checked={applyDefaults}
              disabled={disabled}
              onChange={(event) => onApplyDefaults(event.target.checked)}
            />{" "}
            Use included global defaults
          </label>
          <p>
            Replaces the current global defaults for new workspaces. Existing
            workspaces keep their settings until you review and apply the
            update.
          </p>
        </section>
      ) : null}
      {group(
        "MCP servers",
        mcpServers.map(({ id, label, configuration: server }) =>
          details(
            id,
            label,
            server.url ?? [server.command, ...server.args].join(" "),
            server,
          ),
        ),
        mcpServers.length,
      )}
      {group(
        "Search providers",
        searchProviders.map(({ id, label, configuration: search }) =>
          details(id, label, search.endpoint, search),
        ),
        searchProviders.length,
      )}
      {group(
        "Telemetry profiles",
        telemetryProfiles.map(({ id, label, configuration: telemetry }) =>
          details(
            id,
            label,
            `${telemetry.endpoint ?? "Local only"} · ${
              [
                telemetry.tracesEnabled && "traces",
                telemetry.metricsEnabled && "metrics",
                (telemetry.logsOtlp || telemetry.logsStdoutJson) && "logs",
              ]
                .filter(Boolean)
                .join(", ") || "export disabled"
            }`,
            telemetry,
          ),
        ),
        telemetryProfiles.length,
      )}
      {mcpServers.length + searchProviders.length + telemetryProfiles.length ? (
        <p>
          These entries will be available in MCP, Search, and Telemetry
          settings. Select them for a workspace when ready. Credential
          placeholders need a local secret in Credentials settings.
        </p>
      ) : null}
    </div>
  );
}
