import assert from "node:assert/strict";
import { test } from "node:test";
import { build } from "esbuild";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { workspaceIdentity } from "../src/connection.js";
import type { ManagedSetup } from "../src/managed.js";

test("native Anthropic setup keeps the Messages protocol and stores its key only in native custody", async () => {
  const bundle = await build({
    entryPoints: ["src/managed-setup.ts"],
    bundle: true,
    platform: "node",
    format: "cjs",
    external: ["vscode"],
    write: false,
  });
  const temporary = await mkdtemp(join(tmpdir(), "colossus-managed-setup-"));
  const canonical = await workspaceIdentity(temporary);
  let state: ManagedSetup | undefined;
  const stored = new Map<string, string>();
  const vscode = {
    workspace: {
      isTrusted: true,
      workspaceFolders: [{ uri: { scheme: "file", fsPath: canonical.path } }],
    },
    window: {
      async showQuickPick(items: { protocol?: string }[]) {
        return items.find((item) => item.protocol === "anthropic_messages");
      },
      async showInputBox(options: {
        title: string;
        value?: string;
        password?: boolean;
      }) {
        if (options.title === "Provider API base URL") {
          assert.equal(options.value, "https://api.anthropic.com/v1");
        }
        if (options.title === "Provider API key") {
          assert.equal(options.password, true);
          return "private-anthropic-fixture-key";
        }
        if (options.title === "Model identifier") return "claude-fixture";
        return options.value;
      },
    },
  };
  const module = {
    exports: {} as {
      configureManaged(
        context: unknown,
        workspace: string,
      ): Promise<ManagedSetup>;
    },
  };
  const require = createRequire(import.meta.url);
  runInNewContext(bundle.outputFiles[0]!.text, {
    module,
    exports: module.exports,
    require: (name: string) => (name === "vscode" ? vscode : require(name)),
    process,
    Buffer,
    URL,
    structuredClone,
    setTimeout,
    clearTimeout,
  });
  try {
    const configured = await module.exports.configureManaged(
      {
        workspaceState: {
          get: () => state,
          async update(_key: string, value: ManagedSetup) {
            state = value;
          },
        },
        secrets: {
          async store(key: string, value: string) {
            stored.set(key, value);
          },
        },
      },
      canonical.path,
    );
    const provider = configured.providers[0]!;
    assert.equal(provider.kind, "anthropic_messages");
    assert.equal(provider.base_url, "https://api.anthropic.com/v1");
    assert.equal(configured.models[0]!.provider_profile, provider.profile);
    assert.equal(configured.models[0]!.model, "claude-fixture");
    assert.equal(
      stored.get(`colossus.provider.v1:${provider.credential_id}`),
      "private-anthropic-fixture-key",
    );
    assert.ok(!JSON.stringify(state).includes("private-anthropic-fixture-key"));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("clearing provider authentication deletes custody only after saving and retains a retryable reference on failure", async () => {
  const bundle = await build({
    entryPoints: ["src/managed-setup.ts"],
    bundle: true,
    platform: "node",
    format: "cjs",
    external: ["vscode"],
    write: false,
  });
  const temporary = await mkdtemp(join(tmpdir(), "colossus-managed-setup-"));
  const canonical = await workspaceIdentity(temporary);
  const initial: ManagedSetup = {
    version: 1,
    workspaceIdentity: canonical.identity,
    access: "minimal",
    primary: "model",
    providers: [
      {
        profile: "provider",
        kind: "open_ai_compatible",
        base_url: "https://example.invalid/v1",
        credential_id: "old-key",
        timeout_ms: 5000,
      },
    ],
    models: [
      {
        profile: "model",
        provider_profile: "provider",
        model: "model",
        context_window_tokens: 32768,
        max_output_tokens: 4096,
        capabilities: {
          tool_calls: "auto",
          streaming: "auto",
          image_inputs: "auto",
          server_compaction: "auto",
          declared: {},
        },
      },
    ],
  };
  let state = structuredClone(initial);
  let saveFails = false;
  let deleteFails = false;
  let deleted = false;
  const vscode = {
    workspace: {
      isTrusted: true,
      workspaceFolders: [{ uri: { scheme: "file", fsPath: canonical.path } }],
    },
    window: {
      async showQuickPick(items: { id?: string; provider?: unknown }[]) {
        return items.find((item) => item.id === "credential") ?? items[0];
      },
      async showInputBox() {
        return "";
      },
    },
  };
  const module = {
    exports: {} as {
      configureManaged(
        context: unknown,
        workspace: string,
      ): Promise<ManagedSetup>;
    },
  };
  const require = createRequire(import.meta.url);
  runInNewContext(bundle.outputFiles[0]!.text, {
    module,
    exports: module.exports,
    require: (name: string) => (name === "vscode" ? vscode : require(name)),
    process,
    Buffer,
    URL,
    structuredClone,
    setTimeout,
    clearTimeout,
  });
  const context = {
    workspaceState: {
      get: () => state,
      async update(_key: string, value: ManagedSetup) {
        if (saveFails) throw new Error("private storage failure");
        state = structuredClone(value);
      },
    },
    secrets: {
      async delete(key: string) {
        assert.equal(key, "colossus.provider.v1:old-key");
        assert.equal(
          state.providers[0]!.credential_id,
          null,
          "save the new configuration before deleting custody",
        );
        if (deleteFails) throw new Error("private custody failure");
        deleted = true;
      },
    },
  };
  try {
    await module.exports.configureManaged(context, canonical.path);
    assert.equal(deleted, true);
    assert.equal(state.providers[0]!.credential_id, null);
    state = structuredClone(initial);
    deleted = false;
    saveFails = true;
    await assert.rejects(
      module.exports.configureManaged(context, canonical.path),
    );
    assert.equal(deleted, false);
    assert.equal(state.providers[0]!.credential_id, "old-key");
    saveFails = false;
    deleteFails = true;
    await assert.rejects(
      module.exports.configureManaged(context, canonical.path),
      /provider key could not be deleted/u,
    );
    assert.equal(state.providers[0]!.credential_id, "old-key");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});
