import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type {
  DesktopStatus,
  CatalogEntry,
  ManagedCredentialMetadata,
  ManagedExtensionInventory,
  ManagedMcpServer,
  ManagedModelCatalogValue,
  ManagedProviderCatalogValue,
  ManagedTelemetryProfile,
  RepositoryConfigurationProposal,
  SpaceSummary,
} from "../types";
import {
  advancedSectionContainsField,
  buildManagedSettingsFixture,
  ExtensionCatalog,
  FieldGrid,
  managedModel,
  managedModelConsumers,
  managedCredentialConsumers,
  managedProviderConsumers,
  managedSearchProfileConsumers,
  managedTelemetryConsumers,
  managedProvider,
  managedTelemetry,
  managedFieldDestination,
  ManagedSettingsPane,
  managedMcpServer,
  McpEditor,
  mcpDraft,
  modelDraft,
  providerDraft,
  RepositoryImportDialog,
  SettingsActionBar,
  SpaceSettingsBody,
  spaceDraft,
  telemetryDraft,
  updateMcpDraftName,
} from "./ManagedSettingsPane";

const space: SpaceSummary = {
  spaceId: "space-colossus",
  targetId: "managed-local",
  displayName: "Colossus",
  displayPath: "D:\\tools\\Colossus",
  archived: false,
  lastOpenedAtMs: 1_721_490_000_000,
  state: "ready",
  message: "Managed runtime ready.",
  selected: true,
  attentionCount: 0,
  lastActivityAt: null,
  providerConfigured: true,
};

function desktop(): DesktopStatus {
  return {
    releaseChannel: "development",
    connection: {
      state: "connected",
      message: "Connected securely.",
      targetId: "managed-local",
    },
    targets: [
      {
        targetId: "managed-local",
        kind: "managed_local",
        label: "Managed Local",
        state: "ready",
        message: "Managed runtime ready.",
        selected: true,
        terminalAvailable: true,
        workspace: {
          workspaceId: "workspace-colossus",
          displayName: "Colossus",
          displayPath: "D:\\tools\\Colossus",
        },
        failureCode: null,
      },
    ],
    selectedTargetId: "managed-local",
    spaces: [space],
    selectedSpaceId: space.spaceId,
    managedState: "ready",
    workspace: {
      workspaceId: "workspace-colossus",
      displayName: "Colossus",
      displayPath: "D:\\tools\\Colossus",
    },
    provider: {
      configured: true,
      kind: "openai_compatible",
      model: "colossus-primary",
    },
    codexAuth: {
      state: "signed_out",
      message: "Sign in with ChatGPT to use Codex.",
    },
    managedModelConfiguration: {
      providers: [
        {
          profile: "openapi",
          providerKind: "openai_compatible",
          baseUrl: "https://llm.example.test/v1",
          hasCredential: true,
          timeoutMs: 30_000,
          effectiveTimeoutMs: 30_000,
        },
      ],
      models: [
        {
          profile: "primary",
          providerProfile: "openapi",
          model: "colossus-primary",
          contextWindowTokens: 128_000,
          maxOutputTokens: 16_384,
          capabilities: {
            toolCalls: true,
            streaming: true,
            imageInputs: false,
          },
          reasoningEffort: null,
        },
      ],
      roles: { primary: "primary" },
    },
    accessProfile: "development",
    executionBoundary: "workspace_isolated",
    approvalMode: "ask",
    terminalEnabled: false,
    additionalCaBundle: {
      configured: false,
      certificateCount: 0,
      fingerprintsSha256: [],
    },
    capabilities: {
      research: true,
      delegation: true,
      skills: true,
      tui: true,
      shellTerminal: true,
      files: true,
      artifacts: true,
      planContinuation: true,
      updateAvailable: false,
      agentWorkflows: true,
      attachments: true,
    },
  };
}

function renderPane(): string {
  return renderToStaticMarkup(
    createElement(ManagedSettingsPane, {
      desktop: desktop(),
      connecting: false,
      updateChecking: false,
      updateMessage: "",
      onChooseWorkspace: vi.fn(),
      onConfigureManaged: vi.fn(),
      onRestartManaged: vi.fn(),
      onAddExternalTarget: vi.fn(),
      onRemoveExternalTarget: vi.fn(),
      onSetTerminalEnabled: vi.fn(),
      onOpenTerminal: vi.fn(),
      onCheckForUpdates: vi.fn(),
      onInstallUpdate: vi.fn(),
      onImportCaBundle: vi.fn(),
      onRemoveCaBundle: vi.fn(),
    }),
  );
}

const importProposal: RepositoryConfigurationProposal = {
  spaceId: space.spaceId,
  relativePath: ".colossus/config.yaml",
  sha256: "a".repeat(64),
  previousSha256: "b".repeat(64),
  changedSinceImport: true,
  resources: [
    {
      kind: "provider",
      sourceId: "openapi",
      label: "OpenAPI",
      detail: "open ai compatible",
      conflict: true,
      existingResourceId: "provider-existing",
    },
    {
      kind: "mcp",
      sourceId: "docs",
      label: "Documentation",
      detail: "streamable http",
      conflict: false,
      existingResourceId: null,
    },
  ],
  credentialSlots: [
    {
      slotId: "env:OPENAI_API_KEY",
      label: "OPENAI_API_KEY",
      consumers: ["runtime.providers.profiles.openapi.credentialReference"],
    },
  ],
  fieldOverrides: ["agent.maxTurns"],
  lockedFields: ["storage.location"],
  warnings: ["Static MCP headers are not imported."],
};

const importCredentials: ManagedCredentialMetadata[] = [
  {
    id: "credential-openapi",
    label: "OpenAPI production",
    kind: "api_key",
    backend: "desktop",
    createdAtMs: 1,
  },
];

function renderImport(
  stage: number,
  mappings: Record<string, string> = {
    "env:OPENAI_API_KEY": "credential-openapi",
  },
  conflicts = {
    "provider:openapi": {
      action: "rename" as const,
      renamedSourceId: "openapi-imported",
    },
  },
): string {
  return renderToStaticMarkup(
    createElement(RepositoryImportDialog, {
      proposal: importProposal,
      stage,
      credentials: importCredentials,
      mappings,
      conflicts,
      busy: false,
      onStageChange: vi.fn(),
      onMappingsChange: vi.fn(),
      onConflictsChange: vi.fn(),
      onApply: vi.fn(),
      onClose: vi.fn(),
    }),
  );
}

describe("ManagedSettingsPane", () => {
  it("builds a revisioned, renderer-safe snapshot from Desktop status", () => {
    const snapshot = buildManagedSettingsFixture(desktop());

    expect(snapshot.globalConfiguration.revision).toBe(4);
    expect(snapshot.globalConfiguration.providers[0]?.label).toBe("openapi");
    expect(snapshot.globalConfiguration.models[0]?.label).toBe("primary");
    expect(snapshot.spaces[0]?.configuration.acceptedGlobalRevision).toBe(4);
    expect(snapshot.spaces[0]?.effectiveYaml).toContain(
      "workspaceIdentity: <desktop-managed>",
    );
    expect(snapshot.spaces[0]?.effectiveYaml).toContain("workspace:");
    expect(snapshot.spaces[0]?.effectiveYaml).not.toContain("\nspace:");
    expect(snapshot.lockedInvariants.map((entry) => entry.id)).toContain(
      "runtime.bootstrapAuthentication",
    );

    const serialized = JSON.stringify(snapshot);
    expect(serialized).not.toContain("apiKey");
    expect(serialized).not.toContain("secretValue");
    expect(serialized).not.toContain("accessToken");
  });

  it("reports active credential consumers from renderer-safe metadata", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const credentialId = "credential-openapi";
    snapshot.globalConfiguration.credentials.push({
      id: credentialId,
      label: "OpenAPI production",
      kind: "api_key",
      backend: "desktop",
      createdAtMs: 1,
    });
    snapshot.globalConfiguration.providers[0]!.revisions[0]!.value.credentialId =
      credentialId;
    snapshot.spaces[0]!.configuration.credentialOverrides.OPENAI_API_KEY =
      credentialId;

    expect(managedCredentialConsumers(snapshot, credentialId)).toEqual([
      "Provider · openapi",
      "Workspace · Colossus",
    ]);

    snapshot.globalConfiguration.providers[0]!.archived = true;
    expect(managedCredentialConsumers(snapshot, credentialId)).toEqual([
      "Workspace · Colossus",
    ]);
  });

  it("reports active search profile consumers from pinned workspace metadata", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const search = {
      id: "search-engineering",
      label: "Engineering search",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            profile: "engineering-search",
            kind: "searxng" as const,
            endpoint: "https://search.example.test/search",
            credentialId: null,
            authHeader: null,
            timeoutMs: 30_000,
          },
        },
      ],
    };
    snapshot.globalConfiguration.searchProviders.push(search);
    snapshot.spaces[0]!.configuration.catalogRevisions[`search:${search.id}`] =
      {
        resourceId: search.id,
        revision: search.currentRevision,
      };
    snapshot.spaces[0]!.configuration.searchRoles.agent = "engineering-search";

    expect(managedSearchProfileConsumers(snapshot, search.id)).toEqual([
      "Colossus",
    ]);

    snapshot.spaces[0]!.archived = true;
    expect(managedSearchProfileConsumers(snapshot, search.id)).toEqual([]);

    snapshot.spaces[0]!.archived = false;
    search.archived = true;
    expect(managedSearchProfileConsumers(snapshot, search.id)).toEqual([]);
  });

  it("reports active model consumers from pinned workspace metadata", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const model = snapshot.globalConfiguration.models[0]!;
    const profile = model.revisions[0]!.value.profile;
    snapshot.spaces[0]!.configuration.catalogRevisions[`model:${model.id}`] = {
      resourceId: model.id,
      revision: model.currentRevision,
    };
    snapshot.spaces[0]!.configuration.modelRoles.primary = profile;

    expect(managedModelConsumers(snapshot, model.id)).toEqual(["Colossus"]);

    snapshot.spaces[0]!.archived = true;
    expect(managedModelConsumers(snapshot, model.id)).toEqual([]);

    snapshot.spaces[0]!.archived = false;
    model.archived = true;
    expect(managedModelConsumers(snapshot, model.id)).toEqual([]);
  });

  it("reports active provider consumers from the model catalog", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const provider = snapshot.globalConfiguration.providers[0]!;
    const model = snapshot.globalConfiguration.models[0]!;

    expect(managedProviderConsumers(snapshot, provider.id)).toEqual([
      model.label,
    ]);

    model.archived = true;
    expect(managedProviderConsumers(snapshot, provider.id)).toEqual([]);

    model.archived = false;
    provider.archived = true;
    expect(managedProviderConsumers(snapshot, provider.id)).toEqual([]);
  });

  it("reports active telemetry consumers from workspace assignments", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const telemetry: CatalogEntry<ManagedTelemetryProfile> = {
      id: "telemetry-local",
      label: "Local collector",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "colossus-desktop",
            endpoint: "http://127.0.0.1:4317",
            protocol: "grpc",
            timeoutMs: 10_000,
            tracesEnabled: true,
            traceSampleRatioMillionths: 100_000,
            metricsEnabled: true,
            metricExportIntervalMs: 60_000,
            logsOtlp: true,
            logsStdoutJson: false,
            journalPayloads: "metadata",
            acknowledgeSensitiveContent: false,
            acknowledgeInsecureTransport: false,
            resourceAttributes: {},
          },
        },
      ],
    };
    snapshot.globalConfiguration.telemetryProfiles.push(telemetry);
    snapshot.spaces[0]!.configuration.catalogRevisions[
      `telemetry:${telemetry.id}`
    ] = { resourceId: telemetry.id, revision: 1 };

    expect(managedTelemetryConsumers(snapshot, telemetry.id)).toEqual([
      "Colossus",
    ]);

    snapshot.spaces[0]!.archived = true;
    expect(managedTelemetryConsumers(snapshot, telemetry.id)).toEqual([]);

    snapshot.spaces[0]!.archived = false;
    telemetry.archived = true;
    expect(managedTelemetryConsumers(snapshot, telemetry.id)).toEqual([]);
  });

  it("renders scope, provenance, lifecycle, and dirty-state controls", () => {
    const markup = renderPane();

    expect(markup).toContain('aria-label="Configuration scope"');
    expect(markup).toContain("Global");
    expect(markup).toContain("Workspace");
    expect(markup).not.toContain(">space<");
    expect(markup).toContain("Runtime defaults");
    expect(markup).toContain('role="combobox"');
    expect(markup).not.toContain("<select");
    expect(markup).toContain("built in");
    expect(markup).toContain("Authority summary");
    expect(markup).toContain("No local changes");
    expect(markup).toContain('disabled=""');
  });

  it("renders the live OTLP diagnostic only for an active selected profile", () => {
    const snapshot = buildManagedSettingsFixture(desktop());
    const selectedSpace = snapshot.spaces[0]!;
    const draft = spaceDraft(selectedSpace);
    snapshot.globalConfiguration.telemetryProfiles.push({
      id: "telemetry-local",
      label: "Local collector",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "colossus-desktop",
            endpoint: "http://127.0.0.1:4317",
            protocol: "grpc",
            timeoutMs: 10_000,
            tracesEnabled: true,
            traceSampleRatioMillionths: 100_000,
            metricsEnabled: true,
            metricExportIntervalMs: 60_000,
            logsOtlp: true,
            logsStdoutJson: false,
            journalPayloads: "metadata",
            acknowledgeSensitiveContent: false,
            acknowledgeInsecureTransport: false,
            resourceAttributes: {},
          },
        },
      ],
    });
    draft.selectedTelemetry = "telemetry-local";

    const markup = renderToStaticMarkup(
      createElement(SpaceSettingsBody, {
        tab: "telemetry",
        snapshot,
        selectedSpace,
        draft,
        setDraft: vi.fn(),
        descriptors: snapshot.fieldDescriptors,
        effective: new Map(
          selectedSpace.effectiveValues.map((value) => [value.fieldId, value]),
        ),
        focusedFieldId: null,
        expandedAdvancedSections: new Set<string>(),
        onAdvancedSectionToggle: vi.fn(),
        busy: false,
        mcpDiagnostics: {},
        mcpOauthStatuses: {},
        mcpOauthLogins: {},
        mcpOauthCallbacks: {},
        onMcpOauthCallback: vi.fn(),
        onTestMcp: vi.fn(),
        onLoadMcpOAuthStatus: vi.fn(),
        onLoginMcpOAuth: vi.fn(),
        onCompleteMcpOAuth: vi.fn(),
        onLogoutMcpOAuth: vi.fn(),
        runtimeDiagnostics: {},
        onTestRuntimeProfile: vi.fn(),
        onTestSearchRole: vi.fn(),
        onTestTelemetry: vi.fn(),
        extensionInventory: null,
        extensionInventoryBusy: false,
        onRefreshExtensionInventory: vi.fn(),
      }),
    );

    expect(markup).toContain("Test OTLP exporters");
    expect(markup).not.toContain('disabled=""');
    expect(markup).not.toContain("secretValue");
    expect(markup).not.toContain("authorization");
  });

  it("routes field search results to the owning tab and disclosure", () => {
    const descriptors = buildManagedSettingsFixture(desktop()).fieldDescriptors;
    const semantic = descriptors.find(
      (descriptor) => descriptor.id === "memory.semantic",
    )!;
    const sandbox = descriptors.find(
      (descriptor) => descriptor.id === "sandbox.maxMemoryBytes",
    )!;

    expect(managedFieldDestination(semantic)).toEqual({
      tab: "advanced",
      section: "Memory",
    });
    expect(managedFieldDestination(sandbox)).toEqual({
      tab: "sandbox",
      section: null,
    });
    expect(
      advancedSectionContainsField(
        descriptors.filter(({ section }) => section === "Memory"),
        semantic.id,
      ),
    ).toBe(true);
  });

  it("keeps all renderer markup free of secret inputs and values", () => {
    const markup = renderPane();

    expect(markup).not.toContain('type="password"');
    expect(markup).not.toContain("apiKey");
    expect(markup).not.toContain("clientSecret");
    expect(markup).not.toContain("credentialValue");
  });

  it("preserves the explicit stateless HTTP opt-in across MCP revisions", () => {
    const draft = mcpDraft({
      id: "mcp-cloudflare",
      label: "Cloudflare docs",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "cloudflare",
            transport: "streamable_http",
            command: null,
            args: [],
            workingDirectory: null,
            environmentCredentials: {},
            url: "https://docs.mcp.cloudflare.com/mcp",
            headers: {},
            credentialHeaders: {},
            allowStateless: true,
            oauth: null,
            allowedTools: ["search_cloudflare_documentation"],
            researchTools: [],
            timeoutMs: 30_000,
            maxOutputBytes: 1_048_576,
          },
        },
      ],
    });

    expect(draft.allowStateless).toBe(true);
    expect(managedMcpServer(draft).allowStateless).toBe(true);
    expect(
      managedMcpServer({ ...draft, transport: "stdio" }).allowStateless,
    ).toBe(false);

    const markup = renderToStaticMarkup(
      createElement(McpEditor, {
        draft,
        credentials: [],
        busy: false,
        onChange: vi.fn(),
        onCancel: vi.fn(),
        onSave: vi.fn(),
      }),
    );
    expect(markup).toContain("Allow stateless HTTP");
    expect(markup).toContain('type="checkbox" checked=""');
  });

  it("uses the server name as the catalog label until a custom label is set", () => {
    const base = mcpDraft({
      id: "mcp-splunk",
      label: "splunk",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "splunk",
            transport: "streamable_http",
            command: null,
            args: [],
            workingDirectory: null,
            environmentCredentials: {},
            url: "http://127.0.0.1:18000/mcp",
            headers: {},
            credentialHeaders: {},
            allowStateless: true,
            oauth: null,
            allowedTools: ["*"],
            researchTools: [],
            timeoutMs: null,
            maxOutputBytes: null,
          },
        },
      ],
    });

    expect(updateMcpDraftName(base, "splunk-local")).toMatchObject({
      name: "splunk-local",
      label: "splunk-local",
    });
    expect(
      updateMcpDraftName(
        { ...base, label: "Splunk Production" },
        "splunk-local",
      ),
    ).toMatchObject({
      name: "splunk-local",
      label: "Splunk Production",
    });
  });

  it("groups the primary MCP workflow and keeps catalog metadata advanced", () => {
    const draft = mcpDraft({
      id: "mcp-local-tools",
      label: "Local tools",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "local-tools",
            transport: "stdio",
            command: "mcp-local-tools",
            args: [],
            workingDirectory: null,
            environmentCredentials: {},
            url: null,
            headers: {},
            credentialHeaders: {},
            allowStateless: false,
            oauth: null,
            allowedTools: ["*"],
            researchTools: [],
            timeoutMs: null,
            maxOutputBytes: null,
          },
        },
      ],
    });
    const markup = renderToStaticMarkup(
      createElement(McpEditor, {
        draft,
        credentials: [],
        busy: false,
        onChange: vi.fn(),
        onCancel: vi.fn(),
        onSave: vi.fn(),
      }),
    );

    expect(markup).toContain("Connection");
    expect(markup).toContain("Access");
    expect(markup).toContain("Advanced settings");
    expect(markup).toContain("Display label (optional)");
    expect(markup).toContain('<details class="mcp-editor-advanced">');
    expect(markup).not.toContain("<span>Label</span>");
  });

  it("drops hidden stdio arguments when switching an MCP server to HTTP", () => {
    const draft = mcpDraft({
      id: "mcp-local-tools",
      label: "Local tools",
      currentRevision: 1,
      archived: false,
      revisions: [
        {
          revision: 1,
          value: {
            name: "local-tools",
            transport: "stdio",
            command: "/opt/colossus/bin/mcp-server",
            args: ["--stdio", "--label=one,two"],
            workingDirectory: null,
            environmentCredentials: {},
            url: null,
            headers: {},
            credentialHeaders: {},
            allowStateless: false,
            oauth: null,
            allowedTools: ["search"],
            researchTools: [],
            timeoutMs: null,
            maxOutputBytes: null,
          },
        },
      ],
    });

    const switched = managedMcpServer({
      ...draft,
      transport: "streamable_http",
      commandOrUrl: "https://mcp.example.test/rpc",
    });

    expect(draft.argsText).toBe("--stdio\n--label=one,two");
    expect(switched.args).toEqual([]);
    expect(switched.url).toBe("https://mcp.example.test/rpc");
  });

  it.each([
    {
      name: "local-tools",
      transport: "stdio" as const,
      command: "/opt/colossus/bin/mcp-server",
      args: ["--label=one,two", "--exact value"],
      workingDirectory: "/workspace/services/docs",
      environmentCredentials: {
        GITHUB_TOKEN: "credential-github",
        SPLUNK_TOKEN: "credential-splunk",
      },
      url: null,
      headers: {},
      credentialHeaders: {},
      allowStateless: false,
      oauth: null,
      allowedTools: ["search,exact", "read_document"],
      researchTools: [
        {
          tool: "search,exact",
          title: "Search docs",
          arguments: { limit: 8, nested: { enabled: true } },
        },
      ],
      timeoutMs: null,
      maxOutputBytes: 2_097_152,
    },
    {
      name: "remote-tools",
      transport: "streamable_http" as const,
      command: null,
      args: [],
      workingDirectory: null,
      environmentCredentials: {},
      url: "https://mcp.example.test/rpc",
      headers: { "X-Workspace": "engineering", Accept: "application/json" },
      credentialHeaders: {
        Authorization: {
          scheme: "Bearer",
          credentialId: "credential-bearer",
        },
        "X-Api-Key": { scheme: null, credentialId: "credential-api" },
      },
      allowStateless: true,
      oauth: {
        clientId: "desktop-client",
        clientSecretCredentialId: "credential-oauth",
        callbackPort: 8787,
        scopes: ["read:tools", "execute:tools"],
      },
      allowedTools: ["search", "read"],
      researchTools: [],
      timeoutMs: 45_000,
      maxOutputBytes: null,
    },
  ])("round-trips every durable $transport MCP setting", (server) => {
    const entry: CatalogEntry<ManagedMcpServer> = {
      id: `mcp-${server.name}`,
      label: server.name,
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: server }],
    };

    const draft = mcpDraft(entry);

    expect(managedMcpServer(draft)).toEqual(server);
    expect(draft.argsText).toContain(server.args[0] ?? "");
  });

  it("renders every MCP setting without secret-value inputs", () => {
    const server: ManagedMcpServer = {
      name: "remote-tools",
      transport: "streamable_http",
      command: null,
      args: [],
      workingDirectory: null,
      environmentCredentials: {},
      url: "https://mcp.example.test/rpc",
      headers: { "X-Workspace": "engineering" },
      credentialHeaders: {
        Authorization: {
          scheme: "Bearer",
          credentialId: "credential-bearer",
        },
      },
      allowStateless: true,
      oauth: {
        clientId: "desktop-client",
        clientSecretCredentialId: "credential-bearer",
        callbackPort: 8787,
        scopes: ["read:tools"],
      },
      allowedTools: ["search"],
      researchTools: [
        { tool: "search", title: "Search", arguments: { limit: 4 } },
      ],
      timeoutMs: null,
      maxOutputBytes: null,
    };
    const draft = mcpDraft({
      id: "mcp-remote",
      label: "Remote",
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: server }],
    });
    const markup = renderToStaticMarkup(
      createElement(McpEditor, {
        draft,
        credentials: [
          {
            id: "credential-bearer",
            label: "Bearer token",
            kind: "bearer_token",
            backend: "desktop",
            createdAtMs: 1,
          },
        ],
        busy: false,
        onChange: vi.fn(),
        onCancel: vi.fn(),
        onSave: vi.fn(),
      }),
    );

    for (const label of [
      "Static headers",
      "Credential headers",
      "OAuth client configuration",
      "Client ID",
      "Client secret credential",
      "Callback port",
      "Scopes",
      "Allowed tools",
      "Research tool projections",
      "Timeout (ms)",
      "Maximum output (bytes)",
    ]) {
      expect(markup).toContain(label);
    }
    expect(markup).not.toContain('type="password"');
  });

  it("round-trips provider defaults, model reasoning, and telemetry acknowledgments", () => {
    const provider: ManagedProviderCatalogValue = {
      profile: "openrouter",
      kind: "openai_compatible",
      baseUrl: "https://openrouter.ai/api/v1",
      credentialId: "credential-openrouter",
      timeoutMs: null,
    };
    const providerEntry: CatalogEntry<ManagedProviderCatalogValue> = {
      id: "provider-openrouter",
      label: "OpenRouter",
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: provider }],
    };
    expect(managedProvider(providerDraft(providerEntry))).toEqual(provider);

    const model: ManagedModelCatalogValue = {
      profile: "reasoning",
      providerProfile: "openrouter",
      model: "example/reasoning",
      contextWindowTokens: 200_000,
      maxOutputTokens: 32_000,
      capabilities: {
        toolCalls: true,
        streaming: false,
        imageInputs: true,
      },
      reasoningEffort: "xhigh",
    };
    const modelEntry: CatalogEntry<ManagedModelCatalogValue> = {
      id: "model-reasoning",
      label: "Reasoning",
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: model }],
    };
    expect(managedModel(modelDraft(modelEntry))).toEqual(model);

    const telemetry: ManagedTelemetryProfile = {
      name: "colossus-desktop",
      endpoint: "http://collector.example.test:4318",
      protocol: "http_protobuf",
      timeoutMs: 11_000,
      tracesEnabled: true,
      traceSampleRatioMillionths: 250_000,
      metricsEnabled: false,
      metricExportIntervalMs: 75_000,
      logsOtlp: true,
      logsStdoutJson: true,
      journalPayloads: "full",
      acknowledgeSensitiveContent: true,
      acknowledgeInsecureTransport: true,
      resourceAttributes: {
        "service.namespace": "colossus",
        "deployment.environment": "preview",
      },
    };
    const telemetryEntry: CatalogEntry<ManagedTelemetryProfile> = {
      id: "telemetry-preview",
      label: "Preview collector",
      currentRevision: 1,
      archived: false,
      revisions: [{ revision: 1, value: telemetry }],
    };
    expect(managedTelemetry(telemetryDraft(telemetryEntry))).toEqual(telemetry);
  });

  it("renders a control and help text for every managed field descriptor", () => {
    const descriptors = buildManagedSettingsFixture(desktop()).fieldDescriptors;
    const markup = renderToStaticMarkup(
      createElement(FieldGrid, {
        descriptors,
        values: {},
        effective: new Map(),
        scope: "global",
        onChange: vi.fn(),
        onInherit: vi.fn(),
      }),
    );

    for (const descriptor of descriptors) {
      expect(markup).toContain(`id="managed-setting-${descriptor.id}"`);
      expect(markup).toContain(descriptor.description);
      expect(markup).toContain(`aria-label="${descriptor.title}"`);
    }
  });

  it("keeps apply failures visible in the sticky action bar", () => {
    const markup = renderToStaticMarkup(
      createElement(SettingsActionBar, {
        dirty: true,
        busy: false,
        failure: "The previous Colossus connection did not close cleanly.",
        label: "Apply Workspace changes",
        onDiscard: vi.fn(),
        onApply: vi.fn(),
      }),
    );

    expect(markup).toContain('role="alert"');
    expect(markup).toContain("did not close cleanly");
    expect(markup).toContain("Apply Workspace changes");
  });

  it.each([
    [0, "OpenAPI"],
    [1, "OPENAI_API_KEY"],
    [2, "Rename imported"],
    [3, "storage.location"],
    [4, "Apply import"],
  ])("renders repository import stage %i", (stage, expected) => {
    const markup = renderImport(stage as number);

    expect(markup).toContain(expected);
    expect(markup).toContain("SHA-256 aaaaaaaaaaaa");
    expect(markup).not.toContain('type="password"');
    expect(markup).not.toContain("must-not-cross-renderer");
  });

  it("blocks import progression until native credentials and valid renames are mapped", () => {
    const unmapped = renderImport(1, { "env:OPENAI_API_KEY": "" });
    const missingCredential = renderImport(1, {
      "env:OPENAI_API_KEY": "credential-missing",
    });
    const invalidRename = renderImport(2, undefined, {
      "provider:openapi": {
        action: "rename",
        renamedSourceId: "openapi imported",
      },
    });
    const valid = renderImport(2);

    expect(unmapped).toContain('disabled=""');
    expect(missingCredential).toContain('disabled=""');
    expect(invalidRename).toContain('disabled=""');
    expect(valid).not.toContain(
      'class="button primary" type="button" disabled',
    );
  });

  it.each(["Skills", "Packs", "Workflows"])(
    "renders the live %s catalog without private paths or payloads",
    (section) => {
      const inventory: ManagedExtensionInventory = {
        skills: [
          {
            name: "incident-response",
            version: "1.0.0",
            description: "Incident triage",
            source: "repository:incident-response",
            offlineCompatible: true,
          },
        ],
        packs: [
          {
            name: "engineering-tools",
            version: "2.0.0",
            publisher: "Obscurity Labs",
            status: "enabled",
            manifestSha256: "a".repeat(64),
            trusted: true,
          },
        ],
        workflows: [
          {
            name: "release",
            version: "3.0.0",
            status: "registered",
            updatedAt: "2026-08-20T12:00:00Z",
            revisionHash: "b".repeat(64),
          },
        ],
      };
      const markup = renderToStaticMarkup(
        createElement(ExtensionCatalog, {
          section,
          inventory,
          busy: false,
          runtimeActive: true,
          onRefresh: vi.fn(),
        }),
      );

      expect(markup).toContain("Live runtime catalog");
      expect(markup).toContain(`Refresh ${section.toLowerCase()} catalog`);
      expect(markup).not.toContain("installedPath");
      expect(markup).not.toContain("C:\\private");
      expect(markup).not.toContain("payload");
    },
  );
});
