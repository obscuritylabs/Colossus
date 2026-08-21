import { beforeEach, describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as Array<{ onmessage: ((message: unknown) => void) | null }>,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: tauri.invoke,
  Channel: class {
    onmessage: ((message: unknown) => void) | null = null;

    constructor() {
      tauri.channels.push(this);
    }
  },
}));

import {
  addExternalTarget,
  applyRepositoryConfiguration,
  applySpaceConfiguration,
  applyManagedModelConfiguration,
  beginManagedMcpOAuth,
  archiveThread,
  cancelRun,
  checkDesktopUpdate,
  closeTerminal,
  codexAuthLogin,
  codexAuthLogout,
  codexAuthStatus,
  configureManagedRuntime,
  completeManagedMcpOAuth,
  connectColossus,
  createManagedCredential,
  createRun,
  desktopReleaseChannel,
  deleteManagedCredential,
  diagnoseManagedMcpServer,
  diagnoseManagedModel,
  diagnoseManagedProvider,
  diagnoseManagedSearch,
  diagnoseManagedTelemetry,
  getManagedExtensionInventory,
  getManagedConfiguration,
  getSessionMap,
  getThreadDelegate,
  getRun,
  installDesktopUpdate,
  inspectRepositoryConfiguration,
  logoutManagedMcpOAuth,
  managedMcpOAuthStatus,
  listWorkspaceDirectory,
  listRuns,
  openTerminal,
  resizeTerminal,
  rotateManagedCredential,
  restoreThread,
  removeExternalTarget,
  readWorkspaceFile,
  respondInteraction,
  runManagedSelfTest,
  selectTarget,
  setApprovalMode,
  setTerminalEnabled,
  saveGlobalDefaults,
  saveSpaceConfiguration,
  showTerminalWindow,
  signalTerminal,
  upsertGlobalMcpServer,
  upsertGlobalModel,
  upsertGlobalProvider,
  upsertGlobalSearchProvider,
  upsertGlobalTelemetryProfile,
  watchRun,
  writeTerminal,
} from "./api";
import type {
  ApplyManagedModelConfigurationRequest,
  CancelRunRequest,
  ConfigureManagedRuntimeRequest,
  CreateRunRequest,
  GetRunRequest,
  ListRunsRequest,
  RespondInteractionRequest,
  ThreadLifecycleRequest,
  WatchRunRequest,
} from "./types";

describe("desktop API target routing", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.invoke.mockResolvedValue(undefined);
    tauri.channels.length = 0;
  });

  it("routes MCP diagnostics and OAuth through native commands", async () => {
    await diagnoseManagedMcpServer("space-1", "docs");
    await managedMcpOAuthStatus("space-1", "docs");
    await beginManagedMcpOAuth("space-1", "docs");
    await completeManagedMcpOAuth(
      "space-1",
      "docs",
      "http://127.0.0.1:8765/callback?code=opaque",
    );
    await logoutManagedMcpOAuth("space-1", "docs");
    await diagnoseManagedProvider("space-1", "openapi");
    await diagnoseManagedModel("space-1", "primary");
    await diagnoseManagedSearch("space-1", "research");
    await diagnoseManagedTelemetry("space-1", "production-otlp");
    await getManagedExtensionInventory("space-1");

    expect(tauri.invoke.mock.calls).toEqual([
      [
        "diagnose_managed_mcp_server",
        { request: { spaceId: "space-1", server: "docs" } },
      ],
      [
        "managed_mcp_oauth_status",
        { request: { spaceId: "space-1", server: "docs" } },
      ],
      [
        "begin_managed_mcp_oauth",
        { request: { spaceId: "space-1", server: "docs" } },
      ],
      [
        "complete_managed_mcp_oauth",
        {
          request: {
            spaceId: "space-1",
            server: "docs",
            callbackUrl: "http://127.0.0.1:8765/callback?code=opaque",
          },
        },
      ],
      [
        "logout_managed_mcp_oauth",
        { request: { spaceId: "space-1", server: "docs" } },
      ],
      [
        "diagnose_managed_provider",
        { request: { spaceId: "space-1", profile: "openapi" } },
      ],
      [
        "diagnose_managed_model",
        { request: { spaceId: "space-1", profile: "primary" } },
      ],
      [
        "diagnose_managed_search",
        { request: { spaceId: "space-1", role: "research" } },
      ],
      [
        "diagnose_managed_telemetry",
        { request: { spaceId: "space-1", profile: "production-otlp" } },
      ],
      ["get_managed_extension_inventory", { request: { spaceId: "space-1" } }],
    ]);
  });

  it("keeps repository inspection and credential mapping in native commands", async () => {
    await inspectRepositoryConfiguration("space-opaque-1");
    await applyRepositoryConfiguration({
      spaceId: "space-opaque-1",
      expectedSha256: "a".repeat(64),
      credentialMappings: {
        "env:OPENAI_API_KEY": "credential-opaque-1",
      },
      conflictDecisions: {
        "provider:primary": {
          action: "rename",
          renamedSourceId: "primary-imported",
        },
      },
    });

    expect(tauri.invoke.mock.calls).toEqual([
      [
        "inspect_repository_configuration",
        { request: { spaceId: "space-opaque-1" } },
      ],
      [
        "apply_repository_configuration",
        {
          request: {
            spaceId: "space-opaque-1",
            expectedSha256: "a".repeat(64),
            credentialMappings: {
              "env:OPENAI_API_KEY": "credential-opaque-1",
            },
            conflictDecisions: {
              "provider:primary": {
                action: "rename",
                renamedSourceId: "primary-imported",
              },
            },
          },
        },
      ],
    ]);
    expect(JSON.stringify(tauri.invoke.mock.calls)).not.toContain(
      "secretValue",
    );
  });

  it("passes an explicit target ID to every run mutation and read", async () => {
    const targetId = "target-opaque-7";
    const create: CreateRunRequest = {
      prompt: "Inspect this workspace",
      role: "primary",
      mode: "execute",
      maxTurns: 4,
      idempotencyKey: "create-key",
    };
    const get: GetRunRequest = { runId: "run-1" };
    const list: ListRunsRequest = { pageToken: "" };
    const cancel: CancelRunRequest = {
      runId: "run-1",
      idempotencyKey: "cancel-key",
    };
    const archive: ThreadLifecycleRequest = {
      runId: "run-1",
      idempotencyKey: "archive-key",
    };
    const restore: ThreadLifecycleRequest = {
      runId: "run-1",
      idempotencyKey: "restore-key",
    };
    const respond: RespondInteractionRequest = {
      runId: "run-1",
      interactionId: "interaction-1",
      etag: "etag-1",
      idempotencyKey: "response-key",
      response: {
        type: "approval",
        approved: false,
        requestHash: "request-hash",
      },
    };

    await createRun(targetId, create);
    await getRun(targetId, get);
    await listRuns(targetId, list);
    await cancelRun(targetId, cancel);
    await archiveThread(targetId, archive);
    await restoreThread(targetId, restore);
    await respondInteraction(targetId, respond);

    expect(tauri.invoke.mock.calls).toEqual([
      ["create_run", { targetId, request: create }],
      ["get_run", { targetId, request: get }],
      ["list_runs", { targetId, request: list }],
      ["cancel_run", { targetId, request: cancel }],
      ["archive_thread", { targetId, request: archive }],
      ["restore_thread", { targetId, request: restore }],
      ["respond_interaction", { targetId, request: respond }],
    ]);
  });

  it("routes a watch through the selected target and a private event channel", async () => {
    const request: WatchRunRequest = { runId: "run-9", afterSequence: 17 };
    const handleEvent = vi.fn();

    await watchRun("target-opaque-9", request, handleEvent);

    expect(tauri.channels).toHaveLength(1);
    expect(tauri.channels[0]?.onmessage).toBe(handleEvent);
    expect(tauri.invoke).toHaveBeenCalledWith("watch_run", {
      targetId: "target-opaque-9",
      request,
      onEvent: tauri.channels[0],
    });
  });

  it("uses null only for the intentional default-target connection path", async () => {
    await connectColossus();
    await connectColossus("external-opaque-1");

    expect(tauri.invoke.mock.calls).toEqual([
      ["connect_colossus", { targetId: null }],
      ["connect_colossus", { targetId: "external-opaque-1" }],
    ]);
  });

  it("keeps setup, target selection, and terminal consent arguments typed", async () => {
    const request: ConfigureManagedRuntimeRequest = {
      workspaceId: "workspace-opaque-2",
      providerKind: "openai_responses",
      model: "gpt-5",
      accessProfile: "development",
      executionBoundary: "workspace_isolated",
      replaceCredential: false,
    };

    await configureManagedRuntime(request);
    await runManagedSelfTest();
    await getSessionMap("run-session-map");
    await getThreadDelegate("run-parent", "agent-child");
    await selectTarget("managed-local");
    await setApprovalMode("risk_auto");
    await setTerminalEnabled(true);
    await showTerminalWindow("colossus_tui");
    await showTerminalWindow("shell");
    await showTerminalWindow("colossus_tui", {
      sessionId: "session-1",
      planId: "plan-1",
    });

    expect(tauri.invoke.mock.calls).toEqual([
      ["configure_managed_runtime", { request }],
      ["run_managed_self_test", undefined],
      ["get_session_map", { sourceRunId: "run-session-map" }],
      [
        "get_thread_delegate",
        { parentRunId: "run-parent", jobId: "agent-child" },
      ],
      ["select_target", { targetId: "managed-local" }],
      ["set_approval_mode", { approvalMode: "risk_auto" }],
      ["set_terminal_enabled", { enabled: true }],
      ["show_terminal_window", { request: { kind: "colossus_tui" } }],
      ["show_terminal_window", { request: { kind: "shell" } }],
      [
        "show_terminal_window",
        {
          request: {
            kind: "colossus_tui",
            sessionId: "session-1",
            planId: "plan-1",
          },
        },
      ],
    ]);
  });

  it("applies provider and model collections without renderer credential values", async () => {
    const request: ApplyManagedModelConfigurationRequest = {
      workspaceId: "workspace-opaque-2",
      providers: [
        {
          profile: "local-provider",
          providerKind: "openai_compatible",
          baseUrl: "http://127.0.0.1:11434/v1",
          timeoutMs: 30_000,
          credentialAction: "none",
        },
      ],
      models: [
        {
          profile: "primary",
          providerProfile: "local-provider",
          model: "local-model",
          contextWindowTokens: 32_768,
          maxOutputTokens: 4_096,
          reasoningEffort: null,
          capabilities: { toolCalls: false, streaming: false },
        },
      ],
      roles: { primary: "primary", context_summarizer: "primary" },
      accessProfile: "minimal",
      executionBoundary: "offline_isolated",
    };

    await applyManagedModelConfiguration(request);

    expect(tauri.invoke).toHaveBeenCalledWith(
      "apply_managed_model_configuration",
      { request },
    );
    expect(JSON.stringify(request)).not.toContain("apiKey");
    expect(JSON.stringify(request)).not.toContain("credentialId");
  });

  it("invokes only native Codex account commands without renderer credentials", async () => {
    await codexAuthStatus();
    await codexAuthLogin();
    await codexAuthLogout();

    expect(tauri.invoke.mock.calls.slice(-3)).toEqual([
      ["codex_auth_status", undefined],
      ["codex_auth_login", undefined],
      ["codex_auth_logout", undefined],
    ]);
  });

  it("routes managed settings revisions through native commands without secrets", async () => {
    const defaults = {
      expectedRevision: 4,
      accessProfile: "development" as const,
      executionBoundary: "workspace_isolated" as const,
      terminalEnabled: false,
      fieldOverrides: [{ fieldId: "runtime.maxTurns", value: 12 }],
    };
    const mcp = {
      expectedRevision: 5,
      resourceId: null,
      label: "docs",
      server: {
        name: "docs",
        transport: "streamable_http" as const,
        command: null,
        args: [],
        workingDirectory: null,
        environmentCredentials: {},
        url: "https://mcp.example.test",
        headers: {},
        credentialHeaders: {
          Authorization: {
            scheme: "Bearer",
            credentialId: "credential-opaque-1",
          },
        },
        allowStateless: false,
        oauth: null,
        allowedTools: ["search", "read_document"],
        researchTools: [],
        timeoutMs: 30_000,
        maxOutputBytes: 1_048_576,
      },
    };
    const space = {
      expectedGlobalRevision: 6,
      spaceId: "space-opaque-1",
      accessProfileOverride: null,
      executionBoundaryOverride: "offline_isolated" as const,
      terminalEnabledOverride: null,
      fieldOverrides: [],
      selectedProviderResourceIds: ["provider-opaque-1"],
      selectedModelResourceIds: ["model-opaque-1"],
      selectedMcpResourceIds: ["mcp-opaque-1"],
      selectedSearchResourceIds: ["search-opaque-1"],
      selectedTelemetryResourceId: "telemetry-opaque-1",
      searchRoles: { agent: "search-main", research: "search-main" },
      modelRoles: { primary: "primary" },
      credentialOverrides: {
        "credential-slot-1": "credential-opaque-1",
      },
    };

    await getManagedConfiguration();
    await saveGlobalDefaults(defaults);
    await upsertGlobalMcpServer(mcp);
    const provider = {
      expectedRevision: 6,
      resourceId: null,
      label: "OpenAPI",
      provider: {
        profile: "openapi",
        kind: "openai_compatible" as const,
        baseUrl: "https://llm.example.test/v1",
        credentialId: "credential-opaque-1",
        timeoutMs: 30_000,
      },
    };
    const model = {
      expectedRevision: 7,
      resourceId: null,
      label: "Primary",
      model: {
        profile: "primary",
        providerProfile: "openapi",
        model: "gpt-compatible",
        contextWindowTokens: 128_000,
        maxOutputTokens: 16_384,
        capabilities: { toolCalls: true, streaming: true },
        reasoningEffort: null,
      },
    };
    await upsertGlobalProvider(provider);
    await upsertGlobalModel(model);
    const search = {
      expectedRevision: 8,
      resourceId: null,
      label: "Search",
      search: {
        profile: "search-main",
        kind: "searxng" as const,
        endpoint: "https://search.example.test/search",
        credentialId: "credential-opaque-1",
        authHeader: "X-Search-Key",
        timeoutMs: 30_000,
      },
    };
    await upsertGlobalSearchProvider(search);
    const telemetry = {
      expectedRevision: 9,
      resourceId: null,
      label: "Local collector",
      telemetry: {
        name: "colossus-desktop",
        endpoint: "http://127.0.0.1:4317",
        protocol: "grpc" as const,
        timeoutMs: 10_000,
        tracesEnabled: true,
        traceSampleRatioMillionths: 100_000,
        metricsEnabled: true,
        metricExportIntervalMs: 60_000,
        logsOtlp: true,
        logsStdoutJson: false,
        journalPayloads: "metadata" as const,
        acknowledgeSensitiveContent: false,
        acknowledgeInsecureTransport: false,
        resourceAttributes: { "service.namespace": "colossus" },
      },
    };
    await upsertGlobalTelemetryProfile(telemetry);
    await saveSpaceConfiguration(space);
    await applySpaceConfiguration(space.spaceId);
    await createManagedCredential({
      expectedRevision: 6,
      label: "Docs token",
      kind: "bearer_token",
    });
    await rotateManagedCredential({
      expectedRevision: 7,
      credentialId: "credential-opaque-1",
    });
    await deleteManagedCredential({
      expectedRevision: 8,
      credentialId: "credential-opaque-1",
    });

    expect(tauri.invoke.mock.calls.slice(-12)).toEqual([
      ["get_managed_configuration", undefined],
      ["save_global_defaults", { request: defaults }],
      ["upsert_global_mcp_server", { request: mcp }],
      ["upsert_global_provider", { request: provider }],
      ["upsert_global_model", { request: model }],
      ["upsert_global_search_provider", { request: search }],
      ["upsert_global_telemetry_profile", { request: telemetry }],
      ["save_space_configuration", { request: space }],
      ["apply_space_configuration", { spaceId: space.spaceId }],
      [
        "create_managed_credential",
        {
          request: {
            expectedRevision: 6,
            label: "Docs token",
            kind: "bearer_token",
          },
        },
      ],
      [
        "rotate_managed_credential",
        {
          request: {
            expectedRevision: 7,
            credentialId: "credential-opaque-1",
          },
        },
      ],
      [
        "delete_managed_credential",
        {
          request: {
            expectedRevision: 8,
            credentialId: "credential-opaque-1",
          },
        },
      ],
    ]);

    const payload = JSON.stringify(tauri.invoke.mock.calls.slice(-12));
    expect(payload).not.toContain("secretValue");
    expect(payload).not.toContain("apiKey");
    expect(payload).not.toContain("clientSecret");
    expect(payload).not.toContain("accessToken");
  });

  it("reads the native compile-time release channel without renderer input", async () => {
    await desktopReleaseChannel();

    expect(tauri.invoke).toHaveBeenCalledWith(
      "desktop_release_channel",
      undefined,
    );
  });

  it("keeps update checks and installation behind explicit native commands", async () => {
    await checkDesktopUpdate();
    await installDesktopUpdate();

    expect(tauri.invoke.mock.calls).toEqual([
      ["check_desktop_update", undefined],
      ["install_desktop_update", undefined],
    ]);
  });

  it("enrolls and removes external targets through opaque native commands", async () => {
    await addExternalTarget();
    await removeExternalTarget("01968a3e-0ab3-7f10-bb27-4eadbd550007");

    expect(tauri.invoke.mock.calls).toEqual([
      ["add_external_target", undefined],
      [
        "remove_external_target",
        { targetId: "01968a3e-0ab3-7f10-bb27-4eadbd550007" },
      ],
    ]);
  });

  it("uses opaque session arguments for the fixed terminal command surface", async () => {
    tauri.invoke.mockResolvedValueOnce({ sessionId: "terminal-session-1" });
    const handleEvent = vi.fn();

    await expect(
      openTerminal(
        "workspace-opaque-1",
        7,
        "colossus_tui",
        24,
        80,
        handleEvent,
      ),
    ).resolves.toBe("terminal-session-1");
    await writeTerminal("terminal-session-1", "aGk=");
    await resizeTerminal("terminal-session-1", 30, 100);
    await signalTerminal("terminal-session-1", "interrupt");
    await closeTerminal("terminal-session-1");

    expect(tauri.channels[0]?.onmessage).toBe(handleEvent);
    expect(tauri.invoke.mock.calls).toEqual([
      [
        "open_terminal",
        {
          request: {
            workspaceId: "workspace-opaque-1",
            contextGeneration: 7,
            kind: "colossus_tui",
            rows: 24,
            cols: 80,
          },
          onEvent: tauri.channels[0],
        },
      ],
      [
        "write_terminal",
        {
          request: {
            sessionId: "terminal-session-1",
            dataBase64: "aGk=",
          },
        },
      ],
      [
        "resize_terminal",
        {
          request: { sessionId: "terminal-session-1", rows: 30, cols: 100 },
        },
      ],
      [
        "signal_terminal",
        {
          request: { sessionId: "terminal-session-1", signal: "interrupt" },
        },
      ],
      ["close_terminal", { request: { sessionId: "terminal-session-1" } }],
    ]);
  });

  it("keeps workspace browsing relative and bound to the opaque workspace ID", async () => {
    await listWorkspaceDirectory("workspace-opaque-1", "apps/desktop/src");
    await readWorkspaceFile("workspace-opaque-1", "apps/desktop/src/App.tsx");

    expect(tauri.invoke.mock.calls).toEqual([
      [
        "list_workspace_directory",
        {
          request: {
            workspaceId: "workspace-opaque-1",
            path: "apps/desktop/src",
          },
        },
      ],
      [
        "read_workspace_file",
        {
          request: {
            workspaceId: "workspace-opaque-1",
            path: "apps/desktop/src/App.tsx",
          },
        },
      ],
    ]);
    expect(JSON.stringify(tauri.invoke.mock.calls)).not.toContain(
      "/Users/alex",
    );
  });
});
