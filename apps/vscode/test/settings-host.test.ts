import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { build } from "esbuild";
import { mkdtemp, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { workspaceIdentity } from "../src/connection.js";

test("settings use one editor panel, persist user preferences, and never read credentials", async () => {
  const bundle = await build({
    entryPoints: ["src/extension.ts"],
    bundle: true,
    write: false,
    format: "cjs",
    platform: "node",
    external: ["vscode", "@napi-rs/keyring"],
    logLevel: "silent",
  });
  const providers = new Map<
    string,
    { resolveWebviewView(view: unknown): void }
  >();
  const commands = new Map<string, () => unknown>();
  const executed: unknown[][] = [];
  const errors: string[] = [];
  let themeFailure = false;
  let userSettingsFailure = false;
  let preferenceFailure: Error | undefined;
  let missingPalette = false;
  const properties = JSON.parse(await readFile("package.json", "utf8"))
    .contributes.configuration.properties as Record<
    string,
    { default: unknown }
  >;
  const messages: unknown[] = [];
  const config = new Map<string, unknown>();
  const updates: unknown[][] = [];
  const subscriptions: { dispose(): void }[] = [];
  let receive: ((value: unknown) => void) | undefined;
  let close: (() => void) | undefined;
  let panels = 0;
  let reveals = 0;
  let secretReads = 0;
  let savedProfile: string | undefined;
  const storedProfiles: string[] = [];
  const inputs: (string | undefined)[] = [];
  const prompts: { value: string }[] = [];
  const home = await mkdtemp(join(tmpdir(), "colossus-credential-selector-"));
  const workspace = {
    name: "credential-fixture",
    uri: { scheme: "file", fsPath: home, toString: () => home },
  };
  const disposable = () => ({ dispose() {} });
  const webview = {
    options: {},
    html: "",
    cspSource: "vscode-webview://fixture",
    asWebviewUri: (uri: string) => uri,
    postMessage: (value: unknown) => {
      messages.push(JSON.parse(JSON.stringify(value)));
      return Promise.resolve(true);
    },
    onDidReceiveMessage: (listener: (value: unknown) => void) => {
      receive = listener;
      return disposable();
    },
  };
  const vscode = {
    StatusBarAlignment: { Left: 1 },
    ViewColumn: { Active: -1 },
    ConfigurationTarget: { Global: 1 },
    Uri: {
      joinPath: (base: string, ...paths: string[]) =>
        [base, ...paths].join("/"),
    },
    window: {
      createOutputChannel: () => ({
        ...disposable(),
        appendLine() {},
        show() {},
      }),
      createStatusBarItem: () => ({ ...disposable(), show() {} }),
      registerWebviewViewProvider: (
        id: string,
        provider: { resolveWebviewView(view: unknown): void },
      ) => {
        providers.set(id, provider);
        return disposable();
      },
      createWebviewPanel: () => {
        panels++;
        return {
          webview,
          reveal: () => {
            reveals++;
          },
          onDidDispose: (listener: () => void) => {
            close = listener;
            return disposable();
          },
          dispose() {},
        };
      },
      showErrorMessage: (message: string) => {
        errors.push(message);
        return Promise.resolve(undefined);
      },
      showInputBox: async (options: { value: string }) => {
        prompts.push(options);
        return inputs.shift();
      },
    },
    workspace: {
      isTrusted: true,
      workspaceFolders: [workspace],
      registerTextDocumentContentProvider: disposable,
      onDidCloseTextDocument: disposable,
      onDidChangeWorkspaceFolders: disposable,
      onDidChangeConfiguration: disposable,
      getConfiguration: (namespace: string) => {
        assert.equal(namespace, "colossus");
        return {
          get: (key: string) => config.get(key),
          inspect: (key: string) =>
            missingPalette && key === "appearance.palette"
              ? undefined
              : { defaultValue: properties[`colossus.${key}`]?.default },
          update: async (key: string, value: unknown, target: number) => {
            updates.push([key, value, target]);
            if (preferenceFailure) throw preferenceFailure;
            config.set(key, value);
          },
        };
      },
    },
    commands: {
      registerCommand: (name: string, callback: () => unknown) => {
        commands.set(name, callback);
        return disposable();
      },
      executeCommand: async (...args: unknown[]) => {
        executed.push(args);
        if (themeFailure && args[0] === "workbench.action.selectTheme")
          throw new Error("private-path sensitive-token-fixture");
        if (
          userSettingsFailure &&
          args[0] === "workbench.action.openSettingsJson"
        )
          throw new Error("private-path sensitive-token-fixture");
      },
    },
  };
  const module = { exports: {} as { activate: (context: unknown) => void } };
  const require = createRequire(import.meta.url);
  runInNewContext(bundle.outputFiles[0]!.text, {
    module,
    exports: module.exports,
    require: (name: string) => (name === "vscode" ? vscode : require(name)),
    process,
    Buffer,
    setTimeout,
    clearTimeout,
    console,
    TextEncoder,
    TextDecoder,
    structuredClone,
    URL,
  });
  module.exports.activate({
    extensionUri: "/fixture/extension",
    subscriptions,
    secrets: {
      get: () => {
        secretReads++;
        if (savedProfile) return savedProfile;
        throw new Error("must not read credentials");
      },
      store: async (_key: string, value: string) => {
        storedProfiles.push(value);
        savedProfile = value;
      },
    },
  });
  try {
    await commands.get("colossus.openSettings")!();
    await commands.get("colossus.openSettings")!();
    assert.equal(panels, 1);
    assert.equal(reveals, 1);
    assert.match(webview.html, /settings\.js/u);
    assert.match(webview.html, /connect-src 'none'/u);
    assert.match(webview.html, /script-src 'nonce-/u);
    const send = async (value: unknown) => {
      receive!(value);
      await new Promise<void>((resolve) => setImmediate(resolve));
    };
    await send({ type: "ready" });
    await send({ type: "setPreference", name: "sendShortcut", value: "enter" });
    assert.deepEqual(updates, [["composer.sendShortcut", "enter", 1]]);
    assert.equal(
      (messages.at(-1) as { view: { preferences: { sendShortcut: string } } })
        .view.preferences.sendShortcut,
      "enter",
    );
    await send({
      type: "setPreference",
      name: "access.danger_full_access",
      value: true,
    });
    await send({
      type: "setPreference",
      name: "sendShortcut",
      value: "enter",
      target: "/other",
    });
    assert.equal(updates.length, 1);
    await send({ type: "setPreference", name: "palette", value: "hacker" });
    assert.deepEqual(updates.at(-1), ["appearance.palette", "hacker", 1]);
    assert.equal(
      (messages.at(-1) as { view: { preferences: { palette: string } } }).view
        .preferences.palette,
      "hacker",
    );
    await send({ type: "openThemeSettings" });
    await send({ type: "openWork" });
    assert.deepEqual(executed, [
      ["workbench.action.selectTheme"],
      ["colossus.work.focus"],
    ]);
    assert.equal(secretReads, 0);
    const beforeThemeFailure = messages.length;
    themeFailure = true;
    await send({ type: "openThemeSettings" });
    assert.equal(messages.length, beforeThemeFailure);
    assert.equal(errors.length, 1);
    assert.match(errors[0]!, /VS Code’s theme picker/u);
    assert.doesNotMatch(
      errors[0]!,
      /worker|reconnect|private-path|sensitive-token/u,
    );
    assert.equal(secretReads, 0);
    themeFailure = false;
    missingPalette = true;
    const beforeMissingPalette = updates.length;
    await send({ type: "setPreference", name: "palette", value: "colossus" });
    const unregistered = (
      messages.at(-1) as {
        view: {
          preferenceError: string;
          error: string;
          preferences: { palette: string };
        };
      }
    ).view;
    assert.equal(updates.length, beforeMissingPalette);
    assert.match(unregistered.preferenceError, /Reload Window/u);
    assert.equal(unregistered.error, "");
    assert.equal(unregistered.preferences.palette, "hacker");
    missingPalette = false;
    preferenceFailure = new Error(
      "Unable to write into user settings. Please open the user settings to correct errors/warnings in it and try again. private-path sensitive-token-fixture",
    );
    await send({ type: "setPreference", name: "palette", value: "colossus" });
    const rejected = (
      messages.at(-1) as {
        view: {
          preferences: { palette: string };
          preferenceError: string;
          error: string;
          connected: boolean;
        };
      }
    ).view;
    assert.equal(rejected.preferences.palette, "hacker");
    assert.match(rejected.preferenceError, /Could not save Surface palette/u);
    assert.match(
      rejected.preferenceError,
      /user settings file contains errors/u,
    );
    assert.doesNotMatch(
      rejected.preferenceError,
      /worker|reconnect|private-path|sensitive-token/u,
    );
    assert.equal(rejected.error, "");
    assert.equal(rejected.connected, false);
    assert.equal(errors.length, 1);
    assert.equal(secretReads, 0);
    await send({ type: "openUserSettings" });
    assert.deepEqual(executed.at(-1), ["workbench.action.openSettingsJson"]);
    const beforeUserSettingsFailure = messages.length;
    userSettingsFailure = true;
    await send({ type: "openUserSettings" });
    assert.equal(messages.length, beforeUserSettingsFailure);
    assert.match(
      errors.at(-1)!,
      /VS Code’s user settings could not be opened/u,
    );
    assert.doesNotMatch(
      errors.at(-1)!,
      /worker|reconnect|private-path|sensitive-token/u,
    );
    userSettingsFailure = false;
    preferenceFailure = undefined;
    for (const palette of ["colossus", "editor", "hacker"]) {
      await send({ type: "setPreference", name: "palette", value: palette });
      const saved = (
        messages.at(-1) as {
          view: {
            preferences: { palette: string };
            preferenceError: string;
            error: string;
          };
        }
      ).view;
      assert.equal(saved.preferences.palette, palette);
      assert.equal(saved.preferenceError, "");
      assert.equal(saved.error, "");
    }
    assert.equal(secretReads, 0);
    await commands.get("colossus.openWorkspace")!();
    assert.deepEqual(executed.at(-1), ["colossus.workspace.focus"]);
    assert.equal(secretReads, 0);
    assert.doesNotMatch(
      JSON.stringify(messages),
      /keyring|certificate|credential|discoveryDirectory/u,
    );
    const identity = await workspaceIdentity(home);
    const enrollment = {
      schemaVersion: 1,
      workspacePath: identity.path,
      workspaceIdentity: identity.identity,
      discoveryDirectory: "/private/api",
      instanceId: "00000000-0000-4000-8000-000000000001",
      certificateSha256: "a".repeat(64),
      keyringService: "old.service",
      keyringAccount: "old-account",
      role: "primary",
    };
    savedProfile = JSON.stringify(enrollment);
    // Cancelling the native selector dialog must preserve the saved profile.
    await commands.get("colossus.configureCredential")!();
    assert.equal(storedProfiles.length, 0);
    assert.equal(prompts[0]?.value, "old.service");
    inputs.push("enrolled.service", "enrolled-account");
    await commands.get("colossus.configureCredential")!();
    assert.equal(storedProfiles.length, 1);
    assert.deepEqual(JSON.parse(storedProfiles[0]!), {
      ...enrollment,
      keyringService: "enrolled.service",
      keyringAccount: "enrolled-account",
    });
    assert.equal(prompts[2]?.value, "old-account");
    const reads = secretReads;
    await send({
      type: "configureCredential",
      service: "forged",
      account: "forged",
    });
    assert.equal(secretReads, reads);
    assert.equal(storedProfiles.length, 1);
    await send({ type: "ready" });
    assert.doesNotMatch(
      JSON.stringify(messages),
      /enrolled\.service|enrolled-account|\/private\/api/u,
    );
    close!();
    await commands.get("colossus.openSettings")!();
    assert.equal(panels, 2);
    assert.deepEqual(
      [...providers.keys()],
      ["colossus.work", "colossus.workspace"],
    );
    providers
      .get("colossus.workspace")!
      .resolveWebviewView({ webview, onDidDispose: disposable });
    assert.match(webview.html, /explorer\.js/);
    assert.match(webview.html, /theme\.css/);
    const executions = executed.length;
    const readsBefore = secretReads;
    await send({
      type: "send",
      text: "Forged execution from the data view",
      mode: "execute",
    });
    await send({ type: "respond", id: "forged-obligation" });
    assert.equal(executed.length, executions);
    assert.equal(secretReads, readsBefore);
    await send({ type: "openWork" });
    assert.deepEqual(executed.at(-1), ["colossus.work.focus"]);
  } finally {
    for (const item of subscriptions.reverse()) item.dispose();
    await rm(home, { recursive: true, force: true });
  }
});
