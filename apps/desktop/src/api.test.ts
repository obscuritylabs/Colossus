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
  applyManagedModelConfiguration,
  cancelRun,
  closeTerminal,
  configureManagedRuntime,
  connectColossus,
  createRun,
  desktopReleaseChannel,
  getRun,
  listRuns,
  openTerminal,
  resizeTerminal,
  removeExternalTarget,
  respondInteraction,
  runManagedSelfTest,
  selectTarget,
  setTerminalEnabled,
  showTerminalWindow,
  signalTerminal,
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
  WatchRunRequest,
} from "./types";

describe("desktop API target routing", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.invoke.mockResolvedValue(undefined);
    tauri.channels.length = 0;
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
    await respondInteraction(targetId, respond);

    expect(tauri.invoke.mock.calls).toEqual([
      ["create_run", { targetId, request: create }],
      ["get_run", { targetId, request: get }],
      ["list_runs", { targetId, request: list }],
      ["cancel_run", { targetId, request: cancel }],
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
      replaceCredential: false,
    };

    await configureManagedRuntime(request);
    await runManagedSelfTest();
    await selectTarget("managed-local");
    await setTerminalEnabled(true);
    await showTerminalWindow("colossus_tui");

    expect(tauri.invoke.mock.calls).toEqual([
      ["configure_managed_runtime", { request }],
      ["run_managed_self_test", undefined],
      ["select_target", { targetId: "managed-local" }],
      ["set_terminal_enabled", { enabled: true }],
      ["show_terminal_window", { request: { kind: "colossus_tui" } }],
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
          capabilities: { toolCalls: false, streaming: false },
        },
      ],
      roles: { primary: "primary", context_summarizer: "primary" },
      accessProfile: "minimal",
    };

    await applyManagedModelConfiguration(request);

    expect(tauri.invoke).toHaveBeenCalledWith(
      "apply_managed_model_configuration",
      { request },
    );
    expect(JSON.stringify(request)).not.toContain("apiKey");
    expect(JSON.stringify(request)).not.toContain("credentialId");
  });

  it("reads the native compile-time release channel without renderer input", async () => {
    await desktopReleaseChannel();

    expect(tauri.invoke).toHaveBeenCalledWith(
      "desktop_release_channel",
      undefined,
    );
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
});
