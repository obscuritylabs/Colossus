import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type {
  DesktopStatus,
  ManagedCredentialMetadata,
  ManagedExtensionInventory,
  RepositoryConfigurationProposal,
  SpaceSummary,
} from "../types";
import {
  advancedSectionContainsField,
  buildManagedSettingsFixture,
  ExtensionCatalog,
  managedFieldDestination,
  ManagedSettingsPane,
  RepositoryImportDialog,
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
          capabilities: { toolCalls: true, streaming: true },
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
    expect(snapshot.lockedInvariants.map((entry) => entry.id)).toContain(
      "runtime.bootstrapAuthentication",
    );

    const serialized = JSON.stringify(snapshot);
    expect(serialized).not.toContain("apiKey");
    expect(serialized).not.toContain("secretValue");
    expect(serialized).not.toContain("accessToken");
  });

  it("renders scope, provenance, lifecycle, and dirty-state controls", () => {
    const markup = renderPane();

    expect(markup).toContain('aria-label="Configuration scope"');
    expect(markup).toContain("Global");
    expect(markup).toContain("Space");
    expect(markup).toContain("Runtime defaults");
    expect(markup).toContain("built in");
    expect(markup).toContain("Authority summary");
    expect(markup).toContain("No local changes");
    expect(markup).toContain('disabled=""');
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
