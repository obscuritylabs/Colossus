import { CapabilitiesSurface } from "./CapabilitiesSurface";

import { WorkspaceSurfaceHeader as SurfaceHeader } from "@colossus/ui";
import {
  IconChevronDown,
  IconDownload,
  IconFileText,
  IconFolder,
  IconMenu2,
  IconPlus,
  IconPlugConnected,
  IconRefresh,
  IconShieldCheck,
  IconTerminal2,
  IconTopologyStar3,
  IconTrash,
} from "@tabler/icons-react";
import { useEffect, useRef } from "react";

import type { PresentedArtifact } from "../presenters";
import type {
  ConnectionStatus,
  DesktopStatus,
  Run,
  TerminalKind,
} from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { ManagedSettingsPane } from "./ManagedSettingsPane";
import { ControlPlaneConnections } from "./ControlPlaneConnections";
import { PluginsSurface } from "./PluginsSurface";
import { WorkflowsSurface } from "./WorkflowsSurface";
import { SchedulesSurface } from "./SchedulesSurface";
import type { ScheduleRunAttempt } from "../workflows";
import type { WorkspaceSurface } from "./ProductRail";

interface OperationsSurfaceProps {
  scheduleRunAttempts: Map<string, ScheduleRunAttempt>;
  onCreateWithAgent: (prompt: string) => void;
  agentStarting: boolean;
  scheduleInspection?:
    { scheduleId: string; showRun: boolean } | null | undefined;
  initialSettingsTab?:
    | "runtime"
    | "providers"
    | "models"
    | "plugins"
    | "terminal"
    | "dictation"
    | "cloud"
    | "control-plane"
    | undefined;
  initialSettingsSpaceId?: string | undefined;
  onManageControlPlaneWorkspace: (spaceId: string) => void;
  onManageControlPlaneProfiles: () => void;
  onConfigurePluginConnection?: () => void;
  pluginSelections?: readonly string[];
  onUsePluginSkill?: (id: string) => void;
  surface: Exclude<WorkspaceSurface, "work" | "terminal">;
  connection: ConnectionStatus;
  desktop: DesktopStatus;
  connecting: boolean;
  updateChecking: boolean;
  updateMessage: string;
  runs: readonly Run[];
  artifacts: readonly PresentedArtifact[];
  demoParticipants: readonly AgentParticipant[] | null;
  workNavigationOpen: boolean;
  onOpenWorkNavigation: () => void;
  onReturnToWork?: (() => void) | undefined;
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
  onImportClientIdentity: () => void;
  onRemoveClientIdentity: () => void;
}

function ConnectionsView({
  desktop,
  connecting,
  onSelectTarget,
  onAddExternalTarget,
  onRemoveExternalTarget,
  onManageControlPlaneWorkspace,
  onManageControlPlaneProfiles,
}: Pick<
  OperationsSurfaceProps,
  | "desktop"
  | "connecting"
  | "onSelectTarget"
  | "onAddExternalTarget"
  | "onRemoveExternalTarget"
  | "onManageControlPlaneWorkspace"
  | "onManageControlPlaneProfiles"
>) {
  const externalTargets = desktop.targets.filter(
    (target) => target.kind === "external_daemon",
  );
  return (
    <>
      <SurfaceHeader
        eyebrow="Connections / Runtime routing"
        title="Connections"
        description="Workspaces own local sidecars. External daemons remain shared connection targets."
      />
      <div className="overview-scroll" tabIndex={0}>
        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Folder-backed runtimes</p>
              <h3>
                {desktop.spaces.filter((space) => !space.archived).length}{" "}
                Workspaces
              </h3>
            </div>
            <span className="status-chip tone-neutral">Up to 4 live</span>
          </div>
          <div className="target-grid">
            {desktop.spaces
              .filter((space) => !space.archived)
              .map((space) => (
                <button
                  type="button"
                  key={space.spaceId}
                  className={space.selected ? "is-selected" : undefined}
                  aria-pressed={space.selected}
                  disabled={connecting || space.selected}
                  onClick={() => onSelectTarget(space.targetId)}
                >
                  <span className="target-node-icon" aria-hidden="true">
                    <IconFolder size={20} stroke={1.6} />
                  </span>
                  <span>
                    <strong>{space.displayName}</strong>
                    <small>{space.message}</small>
                  </span>
                  <span className={`target-state target-state-${space.state}`}>
                    {space.state.replaceAll("_", " ")}
                  </span>
                </button>
              ))}
          </div>
        </section>
        <ControlPlaneConnections
          spaces={desktop.spaces}
          disabled={connecting}
          onManageWorkspace={onManageControlPlaneWorkspace}
          onManageProfiles={onManageControlPlaneProfiles}
        />
        <section className="overview-section">
          <div className="section-heading">
            <div>
              <p className="eyebrow">Advanced connections</p>
              <h3>External daemons</h3>
            </div>
            <button
              className="button secondary compact"
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
                  <span>{target.state.replaceAll("_", " ")}</span>
                </div>
                <div className="external-target-row-actions">
                  <button
                    className="button secondary compact"
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
      executionBoundary: desktop.executionBoundary,
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
        effectiveTimeoutMs: provider.effectiveTimeoutMs,
      })),
      models: configuration.models.map((model) => ({
        profile: model.profile,
        providerProfile: model.providerProfile,
        model: model.model,
        contextWindowTokens: model.contextWindowTokens,
        maxOutputTokens: model.maxOutputTokens,
        reasoningEffort: model.reasoningEffort,
        capabilities: model.capabilities,
      })),
      roles: configuration.roles,
    },
    null,
    2,
  );
}

function SettingsView({
  initialSettingsTab,
  initialSettingsSpaceId,
  onReturnToWork,
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
  onImportClientIdentity,
  onRemoveClientIdentity,
}: Pick<
  OperationsSurfaceProps,
  | "initialSettingsTab"
  | "initialSettingsSpaceId"
  | "onReturnToWork"
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
  | "onImportClientIdentity"
  | "onRemoveClientIdentity"
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
      <div className="overview-scroll settings-scroll" tabIndex={0}>
        <ManagedSettingsPane
          initialSpaceTab={initialSettingsTab}
          initialSpaceId={initialSettingsSpaceId}
          onReturnToWork={onReturnToWork}
          desktop={desktop}
          connecting={connecting}
          updateChecking={updateChecking}
          updateMessage={updateMessage}
          onChooseWorkspace={onChooseWorkspace}
          onConfigureManaged={onConfigureManaged}
          onRestartManaged={onRestartManaged}
          onAddExternalTarget={onAddExternalTarget}
          onRemoveExternalTarget={onRemoveExternalTarget}
          onSetTerminalEnabled={onSetTerminalEnabled}
          onOpenTerminal={onOpenTerminal}
          onCheckForUpdates={onCheckForUpdates}
          onInstallUpdate={onInstallUpdate}
          onImportCaBundle={onImportCaBundle}
          onRemoveCaBundle={onRemoveCaBundle}
          onImportClientIdentity={onImportClientIdentity}
          onRemoveClientIdentity={onRemoveClientIdentity}
          onExportDiagnostics={onExportDiagnostics}
        />
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
            <h3>{desktop.workspace?.displayName ?? "Add a Workspace"}</h3>
            <p>
              {desktop.workspace?.displayPath ??
                "Workspace authority has not been granted to this app yet."}
            </p>
            <div className="settings-inline-meta">
              <span>{desktop.provider.kind ?? "No provider"}</span>
              <span>{desktop.provider.model || "No model"}</span>
              <span>{desktop.accessProfile.replace("_", " ")}</span>
              <span>
                {desktop.executionBoundary === "full_access"
                  ? "Unsafe: full access"
                  : desktop.executionBoundary.replaceAll("_", " ")}
              </span>
            </div>
            {desktop.executionBoundary === "full_access" ? (
              <p className="unsafe-settings-label" role="alert">
                Full access disables Colossus filesystem and network isolation.
                Approval mode remains a separate setting.
              </p>
            ) : null}
          </div>
          <div className="settings-actions">
            <button
              className="button secondary"
              type="button"
              disabled={connecting}
              onClick={onChooseWorkspace}
            >
              Add Workspace
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
                The embedded shell runs as your signed-in OS user outside
                Colossus policy and audit. The separate Colossus TUI tab uses
                the verified bundled CLI and retains normal policy and audit
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
  const navigationTriggerRef = useRef<HTMLButtonElement>(null);
  const previousNavigationOpen = useRef(props.workNavigationOpen);

  useEffect(() => {
    if (previousNavigationOpen.current && !props.workNavigationOpen) {
      requestAnimationFrame(() => navigationTriggerRef.current?.focus());
    }
    previousNavigationOpen.current = props.workNavigationOpen;
  }, [props.workNavigationOpen]);

  return (
    <main className="operations-surface" id="primary-workspace" tabIndex={-1}>
      {props.surface !== "settings" ? (
        <button
          ref={navigationTriggerRef}
          className="button secondary compact work-navigation-button operations-navigation-button"
          type="button"
          aria-label="Open Workspace navigation"
          aria-controls="work-navigation"
          aria-expanded={props.workNavigationOpen}
          onClick={props.onOpenWorkNavigation}
        >
          <IconMenu2 size={16} stroke={1.8} aria-hidden="true" />
          <span className="compact-action-copy">Workspaces</span>
        </button>
      ) : null}
      {props.surface === "fleet" ? (
        <CapabilitiesSurface desktop={props.desktop} />
      ) : null}
      {props.surface === "workflows" ? (
        <WorkflowsSurface
          onCreateWithAgent={props.onCreateWithAgent}
          agentStarting={props.agentStarting}
          key={props.desktop.selectedTargetId}
          targetId={props.desktop.selectedTargetId}
          workspaceName={
            props.desktop.workspace?.displayName || "Selected runtime"
          }
          runtimeReady={
            props.connection.state === "connected" &&
            props.connection.targetId === props.desktop.selectedTargetId
          }
        />
      ) : null}
      {props.surface === "schedules" ? (
        <SchedulesSurface
          runAttempts={props.scheduleRunAttempts}
          attemptScope={JSON.stringify([
            props.desktop.selectedTargetId,
            props.desktop.workspace?.workspaceId,
            props.desktop.selectedSpaceId,
          ])}
          onCreateWithAgent={props.onCreateWithAgent}
          agentStarting={props.agentStarting}
          key={props.desktop.selectedTargetId}
          targetId={props.desktop.selectedTargetId}
          workspaceName={
            props.desktop.workspace?.displayName || "Selected runtime"
          }
          runtimeReady={
            props.connection.state === "connected" &&
            props.connection.targetId === props.desktop.selectedTargetId
          }
          initialInspection={props.scheduleInspection}
        />
      ) : null}
      {props.surface === "plugins" ? (
        <PluginsSurface
          key={props.desktop.selectedTargetId}
          targetId={props.desktop.selectedTargetId}
          spaceId={props.desktop.selectedSpaceId}
          onConfigureConnection={props.onConfigurePluginConnection}
          supported={props.desktop.capabilities.plugins}
          selections={props.pluginSelections}
          onUseSkill={
            props.desktop.capabilities.pluginSkillSelection
              ? props.onUsePluginSkill
              : undefined
          }
        />
      ) : null}

      {props.surface === "connections" ? <ConnectionsView {...props} /> : null}
      {props.surface === "settings" ? (
        <SettingsView
          initialSettingsTab={props.initialSettingsTab}
          initialSettingsSpaceId={props.initialSettingsSpaceId}
          onReturnToWork={props.onReturnToWork}
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
          onImportClientIdentity={props.onImportClientIdentity}
          onRemoveClientIdentity={props.onRemoveClientIdentity}
        />
      ) : null}
    </main>
  );
}
