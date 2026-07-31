import {
  IconActivity,
  IconAlertTriangle,
  IconArchive,
  IconArrowUpRight,
  IconCheck,
  IconChevronDown,
  IconCircle,
  IconDownload,
  IconFileText,
  IconFolder,
  IconPlus,
  IconPlugConnected,
  IconRefresh,
  IconRobot,
  IconShieldCheck,
  IconTerminal2,
  IconTopologyStar3,
  IconTrash,
} from "@tabler/icons-react";

import type { OperationalActivityItem, PresentedArtifact } from "../presenters";
import { agentRoleLabel, presentRunStatus } from "../presenters";
import type {
  ConnectionStatus,
  DesktopStatus,
  Run,
  TerminalKind,
} from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { AgentFlow } from "./AgentFlow";
import type { WorkspaceSurface } from "./ProductRail";

interface OperationsSurfaceProps {
  surface: Exclude<WorkspaceSurface, "work" | "terminal">;
  connection: ConnectionStatus;
  desktop: DesktopStatus;
  connecting: boolean;
  updateChecking: boolean;
  updateMessage: string;
  runs: readonly Run[];
  artifacts: readonly PresentedArtifact[];
  activity: readonly OperationalActivityItem[];
  demoParticipants: readonly AgentParticipant[] | null;
  onConnect: () => void;
  onOpenRun: (run: Run) => void;
  onSelectTarget: (targetId: string) => void;
  onAddExternalTarget: () => void;
  onRemoveExternalTarget: (targetId: string) => void;
  onChooseWorkspace: () => void;
  onConfigureManaged: () => void;
  onRestartManaged: () => void;
  onSetTerminalEnabled: (enabled: boolean) => void;
  onOpenTerminal: (kind: TerminalKind) => void;
  onExportDiagnostics: () => void;
  onCheckForUpdates: () => void;
  onInstallUpdate: () => void;
  onImportCaBundle: () => void;
  onRemoveCaBundle: () => void;
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
  desktop,
  demoParticipants,
  onOpenRun,
  onSelectTarget,
}: Pick<
  OperationsSurfaceProps,
  "runs" | "desktop" | "demoParticipants" | "onOpenRun" | "onSelectTarget"
>) {
  const active = runs.filter((run) =>
    ["queued", "running", "waiting", "cancelling"].includes(run.status),
  );
  return (
    <>
      <SurfaceHeader
        eyebrow="Agents & workflows / Overview"
        title="Operational capabilities"
        description="Inspect only the orchestration features advertised by the connected runtime."
      />
      <div className="overview-scroll">
        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Authenticated discovery</p>
              <h3>Available orchestration</h3>
            </div>
          </div>
          <div className="target-grid">
            {desktop.capabilities.delegation ? (
              <article className="capability-summary-card">
                <span className="target-node-icon" aria-hidden="true">
                  <IconRobot size={20} stroke={1.6} />
                </span>
                <span>
                  <strong>Delegated agents</strong>
                  <small>
                    Child runs inherit the caller&apos;s exact tool ceiling and
                    cannot delegate recursively.
                  </small>
                </span>
                <span className="status-chip tone-success">Available</span>
              </article>
            ) : null}
            {desktop.capabilities.agentWorkflows ? (
              <article className="capability-summary-card">
                <span className="target-node-icon" aria-hidden="true">
                  <IconTopologyStar3 size={20} stroke={1.6} />
                </span>
                <span>
                  <strong>Durable workflows</strong>
                  <small>
                    Registered workflow definitions can run through the same
                    policy and approval gateway.
                  </small>
                </span>
                <span className="status-chip tone-success">Available</span>
              </article>
            ) : null}
            {desktop.capabilities.skills ? (
              <article className="capability-summary-card">
                <span className="target-node-icon" aria-hidden="true">
                  <IconArchive size={20} stroke={1.6} />
                </span>
                <span>
                  <strong>Declarative skills</strong>
                  <small>
                    Skill selection is enabled for this authenticated
                    application.
                  </small>
                </span>
                <span className="status-chip tone-success">Available</span>
              </article>
            ) : null}
          </div>
        </section>
        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Runtime targets</p>
              <h3>{desktop.targets.length} available nodes</h3>
            </div>
            <span className="status-chip tone-neutral">
              {
                desktop.targets.filter((target) => target.state === "ready")
                  .length
              }{" "}
              ready
            </span>
          </div>
          <div className="target-grid">
            {desktop.targets.map((target) => (
              <button
                type="button"
                key={target.targetId}
                className={target.selected ? "is-selected" : undefined}
                aria-pressed={target.selected}
                onClick={() => onSelectTarget(target.targetId)}
              >
                <span className="target-node-icon" aria-hidden="true">
                  {target.kind === "managed_local" ? (
                    <IconRobot size={20} stroke={1.6} />
                  ) : (
                    <IconTopologyStar3 size={20} stroke={1.6} />
                  )}
                </span>
                <span>
                  <strong>{target.label}</strong>
                  <small>
                    {target.kind === "managed_local"
                      ? (target.workspace?.displayName ?? "Local workspace")
                      : "External daemon"}
                  </small>
                </span>
                <span className={`target-state target-state-${target.state}`}>
                  {target.state.replace("_", " ")}
                </span>
              </button>
            ))}
            {desktop.targets.length === 0 ? (
              <p className="inline-empty">
                Configure Managed Local or connect an external daemon.
              </p>
            ) : null}
          </div>
        </section>

        {demoParticipants !== null ? (
          <section className="overview-section">
            <div className="section-heading">
              <div>
                <p className="eyebrow">Live topology</p>
                <h3>Advertised agent topology</h3>
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

function effectiveManagedConfiguration(desktop: DesktopStatus): string {
  const configuration = desktop.managedModelConfiguration;
  return JSON.stringify(
    {
      workspace:
        desktop.workspace === null
          ? null
          : {
              displayName: desktop.workspace.displayName,
              displayPath: desktop.workspace.displayPath,
            },
      accessProfile: desktop.accessProfile,
      terminalEnabled: desktop.terminalEnabled,
      additionalCaBundle: desktop.additionalCaBundle,
      providers: configuration.providers.map((provider) => ({
        profile: provider.profile,
        kind: provider.providerKind,
        baseUrl: provider.baseUrl,
        credential: provider.hasCredential
          ? "stored_in_native_keyring"
          : "not_configured",
        timeoutMs: provider.timeoutMs,
      })),
      models: configuration.models.map((model) => ({
        profile: model.profile,
        providerProfile: model.providerProfile,
        model: model.model,
        contextWindowTokens: model.contextWindowTokens,
        maxOutputTokens: model.maxOutputTokens,
        capabilities: model.capabilities,
      })),
      roles: configuration.roles,
    },
    null,
    2,
  );
}

function SettingsView({
  connection,
  desktop,
  connecting,
  onConnect,
  onSelectTarget,
  onAddExternalTarget,
  onRemoveExternalTarget,
  onChooseWorkspace,
  onConfigureManaged,
  onRestartManaged,
  onSetTerminalEnabled,
  onOpenTerminal,
  onExportDiagnostics,
  onCheckForUpdates,
  onInstallUpdate,
  updateChecking,
  updateMessage,
  onImportCaBundle,
  onRemoveCaBundle,
}: Pick<
  OperationsSurfaceProps,
  | "connection"
  | "desktop"
  | "connecting"
  | "onConnect"
  | "onSelectTarget"
  | "onAddExternalTarget"
  | "onRemoveExternalTarget"
  | "onChooseWorkspace"
  | "onConfigureManaged"
  | "onRestartManaged"
  | "onSetTerminalEnabled"
  | "onOpenTerminal"
  | "onExportDiagnostics"
  | "onCheckForUpdates"
  | "onInstallUpdate"
  | "updateChecking"
  | "updateMessage"
  | "onImportCaBundle"
  | "onRemoveCaBundle"
>) {
  const localTarget = desktop.targets.find(
    (target) => target.kind === "managed_local",
  );
  const selectedTarget = desktop.targets.find(
    (target) =>
      target.selected ||
      (connection.targetId !== null && target.targetId === connection.targetId),
  );
  const externalTargets = desktop.targets.filter(
    (target) => target.kind === "external_daemon",
  );
  const terminalAvailable = selectedTarget?.terminalAvailable === true;
  const shellAvailable = desktop.capabilities.shellTerminal;
  const hasManagedConfiguration =
    desktop.managedModelConfiguration.providers.length > 0 &&
    desktop.managedModelConfiguration.models.length > 0;
  const managedConfigurationState =
    desktop.managedState === "ready" && hasManagedConfiguration
      ? "Active"
      : hasManagedConfiguration
        ? "Saved"
        : "Not configured";
  return (
    <>
      <SurfaceHeader
        eyebrow="Settings / Runtime"
        title="Desktop runtime"
        description="Manage the local sidecar, external targets, and local-only terminal boundary."
      />
      <div className="overview-scroll settings-scroll">
        <section className="settings-card">
          <div className="settings-card-icon">
            <IconPlugConnected size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Selected target</p>
            <h3>
              {connection.state === "connected"
                ? (selectedTarget?.label ?? "Connected")
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
        <section className="settings-card settings-card-stack">
          <div className="settings-card-icon">
            <IconRefresh size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Signed channel</p>
            <h3>Desktop updates</h3>
            <p>
              Update checks run only when requested. Metadata and packages use
              the {desktop.releaseChannel.replaceAll("_", " ")} channel and the
              configured CA trust bundle.
            </p>
            {updateMessage ? (
              <p className="settings-inline-status" role="status">
                {updateMessage}
              </p>
            ) : null}
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              disabled={updateChecking}
              onClick={onCheckForUpdates}
            >
              <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
              {updateChecking ? "Checking…" : "Check for updates"}
            </button>
            {desktop.capabilities.updateAvailable ? (
              <button
                className="button primary"
                type="button"
                disabled={updateChecking}
                onClick={onInstallUpdate}
              >
                Install update
              </button>
            ) : null}
          </div>
        </section>
        <section className="settings-card settings-card-stack">
          <div className="settings-card-icon">
            <IconFolder size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Managed Local</p>
            <h3>{desktop.workspace?.displayName ?? "Choose a workspace"}</h3>
            <p>
              {desktop.workspace?.displayPath ??
                "Workspace authority has not been granted to this app yet."}
            </p>
            <div className="settings-inline-meta">
              <span>{desktop.provider.kind ?? "No provider"}</span>
              <span>{desktop.provider.model || "No model"}</span>
              <span>{desktop.accessProfile.replace("_", " ")}</span>
            </div>
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              disabled={connecting}
              onClick={onChooseWorkspace}
            >
              Choose folder
            </button>
            <button
              className="button secondary"
              type="button"
              disabled={connecting || desktop.workspace === null}
              onClick={onConfigureManaged}
            >
              Provider
            </button>
            <button
              className="button secondary"
              type="button"
              disabled={connecting || localTarget === undefined}
              onClick={onRestartManaged}
            >
              <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
              Restart
            </button>
          </div>
        </section>
        <section className="settings-card settings-card-stack effective-configuration-card">
          <div className="settings-card-icon">
            <IconFileText size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Effective configuration</p>
            <h3>Managed Local configuration</h3>
            <p>
              This read-only view reflects the configuration saved for Managed
              Local. Credentials, keyring labels, certificate paths, and private
              runtime paths remain native-only.
            </p>
          </div>
          <span
            className={`status-chip ${
              managedConfigurationState === "Active"
                ? "tone-success"
                : "tone-neutral"
            }`}
          >
            {managedConfigurationState}
          </span>
          <details className="effective-configuration-disclosure">
            <summary>
              <span className="configuration-collapsed-label">
                Show configuration
              </span>
              <span className="configuration-expanded-label">
                Hide configuration
              </span>
              <IconChevronDown size={17} stroke={1.8} aria-hidden="true" />
            </summary>
            <pre
              className="effective-configuration-code"
              aria-label="Effective Managed Local configuration"
            >
              <code>{effectiveManagedConfiguration(desktop)}</code>
            </pre>
          </details>
        </section>
        <section className="settings-card settings-card-stack external-targets-card">
          <div className="settings-card-icon">
            <IconTopologyStar3 size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Advanced daemon connections</p>
            <h3>External targets</h3>
            <p>
              Import an existing pinned-TLS connection file. Trust anchors and
              keyring lookup labels stay in native owner-private storage.
            </p>
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              disabled={connecting}
              onClick={onAddExternalTarget}
            >
              <IconPlus size={16} stroke={1.8} aria-hidden="true" />
              Add daemon
            </button>
          </div>
          <div className="external-target-settings-list">
            {externalTargets.map((target) => (
              <article key={target.targetId}>
                <div>
                  <strong>{target.label}</strong>
                  <span>{target.state.replace("_", " ")}</span>
                </div>
                <div className="external-target-row-actions">
                  <button
                    className="button secondary"
                    type="button"
                    disabled={connecting || target.selected}
                    onClick={() => onSelectTarget(target.targetId)}
                  >
                    {target.selected ? "Selected" : "Use"}
                  </button>
                  <button
                    className="icon-button danger-icon-button"
                    type="button"
                    disabled={connecting}
                    aria-label={`Remove ${target.label}`}
                    title={`Remove ${target.label}`}
                    onClick={() => onRemoveExternalTarget(target.targetId)}
                  >
                    <IconTrash size={17} stroke={1.7} aria-hidden="true" />
                  </button>
                </div>
              </article>
            ))}
            {externalTargets.length === 0 ? (
              <p className="inline-empty">No external daemons are saved.</p>
            ) : null}
          </div>
        </section>
        {desktop.capabilities.tui || shellAvailable ? (
          <section className="settings-card settings-card-stack terminal-settings-card">
            <div className="settings-card-icon">
              <IconTerminal2 size={23} stroke={1.6} aria-hidden="true" />
            </div>
            <div>
              <p className="eyebrow">Advanced local feature</p>
              <h3>Local terminal</h3>
              <p>
                The embedded shell runs as your macOS user outside Colossus
                policy and audit. The separate Colossus TUI tab uses the
                verified bundled CLI and retains normal policy and audit
                behavior.
              </p>
            </div>
            <div className="settings-actions terminal-settings-actions">
              <label className="terminal-consent-toggle">
                <input
                  type="checkbox"
                  checked={desktop.terminalEnabled}
                  disabled={
                    connecting ||
                    (!terminalAvailable && !shellAvailable) ||
                    desktop.workspace === null
                  }
                  onChange={(event) =>
                    onSetTerminalEnabled(event.target.checked)
                  }
                />
                <span>
                  {desktop.terminalEnabled
                    ? "Local terminal enabled"
                    : "I understand and want to enable it"}
                </span>
              </label>
              {shellAvailable ? (
                <button
                  className="button primary"
                  type="button"
                  disabled={!desktop.terminalEnabled}
                  onClick={() => onOpenTerminal("shell")}
                >
                  Open Shell
                </button>
              ) : null}
              {desktop.capabilities.tui ? (
                <button
                  className="button secondary"
                  type="button"
                  disabled={
                    !desktop.terminalEnabled ||
                    !terminalAvailable ||
                    localTarget === undefined ||
                    localTarget.state !== "ready"
                  }
                  onClick={() => onOpenTerminal("colossus_tui")}
                >
                  Open Colossus TUI
                </button>
              ) : null}
            </div>
          </section>
        ) : null}
        <section className="settings-card settings-card-stack">
          <div className="settings-card-icon">
            <IconShieldCheck size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Outbound TLS trust</p>
            <h3>Additional CA certificates</h3>
            <p>
              {desktop.additionalCaBundle.configured
                ? `${desktop.additionalCaBundle.certificateCount} additional certificate${desktop.additionalCaBundle.certificateCount === 1 ? "" : "s"} are trusted by Colossus networking.`
                : "No additional CA bundle is configured. Public system trust remains available."}
            </p>
            {desktop.additionalCaBundle.configured ? (
              <details className="ca-fingerprint-details">
                <summary>Certificate fingerprints</summary>
                <ul className="ca-fingerprint-list">
                  {desktop.additionalCaBundle.fingerprintsSha256.map(
                    (fingerprint) => (
                      <li key={fingerprint}>
                        <code>{fingerprint}</code>
                      </li>
                    ),
                  )}
                </ul>
              </details>
            ) : null}
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              disabled={connecting}
              onClick={onImportCaBundle}
            >
              Import PEM bundle
            </button>
            {desktop.additionalCaBundle.configured ? (
              <button
                className="button secondary"
                type="button"
                disabled={connecting}
                onClick={onRemoveCaBundle}
              >
                Remove bundle
              </button>
            ) : null}
          </div>
        </section>
        <section className="settings-card settings-card-stack">
          <div className="settings-card-icon">
            <IconDownload size={23} stroke={1.6} aria-hidden="true" />
          </div>
          <div>
            <p className="eyebrow">Local support</p>
            <h3>Diagnostics</h3>
            <p>
              Export version, platform, bundle status, and sanitized runtime
              health. Prompts, credentials, model output, and private paths are
              excluded.
            </p>
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              onClick={onExportDiagnostics}
            >
              <IconDownload size={16} stroke={1.8} aria-hidden="true" />
              Export diagnostics
            </button>
          </div>
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
          desktop={props.desktop}
          connecting={props.connecting}
          onConnect={props.onConnect}
          onSelectTarget={props.onSelectTarget}
          onAddExternalTarget={props.onAddExternalTarget}
          onRemoveExternalTarget={props.onRemoveExternalTarget}
          onChooseWorkspace={props.onChooseWorkspace}
          onConfigureManaged={props.onConfigureManaged}
          onRestartManaged={props.onRestartManaged}
          onSetTerminalEnabled={props.onSetTerminalEnabled}
          onOpenTerminal={props.onOpenTerminal}
          onExportDiagnostics={props.onExportDiagnostics}
          onCheckForUpdates={props.onCheckForUpdates}
          onInstallUpdate={props.onInstallUpdate}
          updateChecking={props.updateChecking}
          updateMessage={props.updateMessage}
          onImportCaBundle={props.onImportCaBundle}
          onRemoveCaBundle={props.onRemoveCaBundle}
        />
      ) : null}
    </main>
  );
}
