import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { DesktopStatus } from "../types";
import { ModelConfigurationEditor } from "./ModelConfigurationEditor";

const desktop: DesktopStatus = {
  releaseChannel: "development",
  connection: {
    state: "connected",
    message: "Connected.",
    targetId: "managed-local",
  },
  targets: [],
  selectedTargetId: "managed-local",
  managedState: "ready",
  workspace: {
    workspaceId: "workspace-1",
    displayName: "Colossus",
    displayPath: "~/Colossus",
  },
  provider: {
    configured: true,
    kind: "openai_compatible",
    model: "example-model",
  },
  managedModelConfiguration: {
    providers: [
      {
        profile: "local-provider",
        providerKind: "openai_compatible",
        baseUrl: "http://127.0.0.1:11434/v1",
        hasCredential: false,
        timeoutMs: null,
        effectiveTimeoutMs: 900_000,
      },
    ],
    models: [
      {
        profile: "primary",
        providerProfile: "local-provider",
        model: "example-model",
        contextWindowTokens: 32_768,
        maxOutputTokens: 4_096,
        capabilities: { toolCalls: false, streaming: false },
      },
    ],
    roles: { primary: "primary" },
  },
  accessProfile: "minimal",
  approvalMode: "ask",
  terminalEnabled: false,
  additionalCaBundle: {
    configured: false,
    certificateCount: 0,
    fingerprintsSha256: [],
  },
  capabilities: {
    delegation: false,
    skills: false,
    tui: false,
    shellTerminal: false,
    files: false,
    artifacts: true,
    planContinuation: false,
    updateAvailable: false,
    agentWorkflows: false,
    attachments: false,
  },
};

describe("ModelConfigurationEditor", () => {
  it("renders safe provider/model metadata and every supported role", () => {
    const markup = renderToStaticMarkup(
      createElement(ModelConfigurationEditor, {
        desktop,
        busy: false,
        onApply: vi.fn(),
        onBack: vi.fn(),
      }),
    );

    expect(markup).toContain("http://127.0.0.1:11434/v1");
    expect(markup).toContain("example-model");
    expect(markup).toContain("No credential");
    expect(markup).toContain("Automatic · 15 minutes");
    expect(markup).not.toContain("Custom timeout (ms)");
    for (const role of [
      "primary",
      "risk evaluator",
      "context summarizer",
      "subagent default",
      "research planner",
      "research worker",
      "research synthesizer",
    ]) {
      expect(markup).toContain(role);
    }
    expect(markup).not.toContain("credentialId");
    expect(markup).not.toContain("apiKey");
  });
});
