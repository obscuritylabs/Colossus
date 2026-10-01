import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  appendFileSync,
  chmodSync,
  copyFileSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync, spawnSync } from "node:child_process";
import test from "node:test";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function read(relative) {
  return readFileSync(join(repository, relative), "utf8");
}

function filesUnder(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? filesUnder(path) : [path];
  });
}

function json(relative) {
  return JSON.parse(read(relative));
}

function directives(policy) {
  return new Map(
    policy.split(";").flatMap((raw) => {
      const tokens = raw.trim().split(/\s+/u).filter(Boolean);
      return tokens.length === 0 ? [] : [[tokens[0], tokens.slice(1)]];
    }),
  );
}

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

test("Desktop dropdowns use the app-owned accessible select control", () => {
  const sourceRoot = join(repository, "apps/desktop/src");
  const nativeSelects = filesUnder(sourceRoot)
    .filter((path) => path.endsWith(".tsx") && !path.endsWith(".test.tsx"))
    .filter((path) => /<select\b/u.test(readFileSync(path, "utf8")))
    .map((path) => path.slice(repository.length + 1));

  assert.deepEqual(nativeSelects, []);
  const dropdown = read("apps/desktop/src/components/DropdownSelect.tsx");
  assert.match(dropdown, /role="combobox"/u);
  assert.match(dropdown, /role="listbox"/u);
  assert.match(dropdown, /role="option"/u);
  assert.match(dropdown, /createPortal/u);
});

test("Desktop surfaces use the shared theme and readable typography contracts", () => {
  const styles = read("apps/desktop/src/styles.css");
  const theme = read("apps/desktop/src/theme/theme.css");

  for (const token of [
    "--surface-selected",
    "--surface-overlay",
    "--focus-ring",
    "--scroll-thumb",
    "--code-surface",
    "--code-text",
    "--code-add-surface",
    "--code-delete-surface",
    "--purple-soft",
  ]) {
    assert.equal(
      theme.match(new RegExp(`${token}:`, "gu"))?.length,
      2,
      `${token} must define dark and light values`,
    );
  }

  assert.doesNotMatch(
    styles,
    /background(?:-color)?:\s*#[\da-f]{3,8}\b/iu,
    "component backgrounds must use semantic theme tokens",
  );
  assert.doesNotMatch(
    styles,
    /font-size:\s*0\.[0-6]\d*rem/iu,
    "visible Desktop copy must not be smaller than the caption token",
  );
  assert.match(
    styles,
    /\.artifact-workspace\s*\{[^}]*background:\s*var\(--main\);/su,
  );
  assert.match(
    styles,
    /\.artifact-preview\s*\{[^}]*background:\s*var\(--code-surface\);/su,
  );
  assert.match(
    styles,
    /\.file-code-scroll\s*\{[^}]*background:\s*var\(--code-surface\);/su,
  );
});

test("development Tauri configuration uses prepared native executables", () => {
  const config = json("apps/desktop/src-tauri/tauri.conf.json");
  assert.equal(config.build.removeUnusedCommands, true);
  assert.equal(config.bundle.active, true);
  assert.deepEqual(config.bundle.targets, ["app"]);
  assert.deepEqual(config.bundle.externalBin, [
    "binaries/colossus-sidecar",
    "binaries/colossus",
  ]);
  assert.equal(config.bundle.macOS.hardenedRuntime, true);
  assert.deepEqual(config.app.security.capabilities, [
    "main-chat",
    "terminal-pty",
    "command-approval",
  ]);
  assert.deepEqual(config.plugins.updater, {
    endpoints: [],
    pubkey: "",
  });
});

test("Windows uninstall data deletion is opt-in, confirmed before mutation, and never used for updates", () => {
  const config = json("apps/desktop/src-tauri/tauri.windows.conf.json");
  assert.equal(config.bundle.windows.nsis.installerHooks, "installer-hooks.nsh");
  const hooks = read("apps/desktop/src-tauri/installer-hooks.nsh");
  assert.match(hooks, /\$DeleteAppDataCheckboxState = 1/u);
  assert.match(hooks, /\$UpdateMode <> 1/u);
  assert.match(hooks, /MB_DEFBUTTON2/u);
  assert.match(hooks, /\/SD IDNO/u);
  assert.match(hooks, /local conversations, provider and model configurations, saved Desktop credentials/u);
  assert.ok(hooks.indexOf("MessageBox MB_YESNO") < hooks.indexOf("ExecWait"));
  assert.ok(hooks.indexOf("CheckIfAppIsRunning") < hooks.indexOf("ExecWait"));
  assert.match(hooks, /IfErrors colossus_cleanup_failed/u);
  assert.match(hooks, /\$0 != 0/u);
  assert.match(hooks, /MB_RETRYCANCEL/u);
  assert.match(hooks, /\$0 = 2[\s\S]*Desktop data is still in use/u);
  assert.match(hooks, /\$0 = 3[\s\S]*could not verify that all Desktop data is safe/u);
  assert.match(hooks, /\$0 = 4[\s\S]*could not remove its saved credentials/u);
  assert.match(hooks, /\$0 = 5[\s\S]*Windows could not remove the Desktop data files/u);
  assert.match(hooks, /without selecting Delete application data/u);
  assert.doesNotMatch(hooks, /RMDir|DeleteRegKey|cmdkey|PowerShell/iu);
  const bridge = read("apps/desktop/src-tauri/src/lib.rs");
  assert.ok(bridge.indexOf("uninstall::run_if_requested()") < bridge.indexOf("SettingsStore::open_application()"));
  assert.ok(!bridge.slice(bridge.indexOf("tauri::generate_handler!")).includes("uninstall::"));
});

test("Windows Desktop seals signed releases in the required order", () => {
  const config = json("apps/desktop/src-tauri/tauri.windows.conf.json");
  assert.deepEqual(config.bundle.targets, ["nsis"]);
  assert.deepEqual(config.bundle.resources, {
    "binaries/colossus-bundle-manifest.json": "colossus-bundle-manifest.json",
  });
  assert.equal(config.bundle.windows.allowDowngrades, false);
  assert.deepEqual(config.bundle.windows.webviewInstallMode, {
    type: "offlineInstaller",
    silent: true,
  });
  assert.equal(config.bundle.windows.nsis.installMode, "currentUser");

  const packaging = read("scripts/package-desktop-windows.ps1");
  assert.match(packaging, /x86_64-pc-windows-msvc/u);
  assert.match(packaging, /"stable", "developer_preview"/u);
  assert.match(packaging, /COLOSSUS_DESKTOP_TEAM_ID -ne "OBSCURITY_LABS_LLC"/u);
  assert.match(packaging, /COLOSSUS_DESKTOP_TEAM_ID -ne "UNSIGNED"/u);
  assert.match(packaging, /"all", "build", "bind", "bundle", "finalize"/u);
  assert.match(
    packaging,
    /cargo metadata --locked --no-deps --format-version 1/u,
  );
  assert.match(packaging, /\$ReleaseVersion = \$BundledVersion/u);
  assert.match(
    packaging,
    /desktop release version \$ReleaseVersion must match bundled CLI version \$BundledVersion/u,
  );
  assert.match(packaging, /cargo xtask desktop prepare/u);
  assert.match(packaging, /--no-sign/u);
  assert.match(packaging, /\[IO\.Path\]::GetTempPath\(\)/u);
  assert.match(packaging, /ConvertTo-Json -Compress -Depth 4/u);
  assert.match(packaging, /\[IO\.File\]::WriteAllText\(/u);
  assert.equal(packaging.match(/"--config", \$TauriOverridePath/gu)?.length, 2);
  assert.doesNotMatch(packaging, /\$VersionOverride/u);
  assert.match(packaging, /write-desktop-bundle-manifest\.mjs/u);
  assert.match(packaging, /stage-ripgrep\.mjs/u);
  assert.match(packaging, /--ripgrep \$StagedRipgrep/u);
  assert.match(packaging, /patch-desktop-manifest-binding\.mjs/u);
  const detach = packaging.indexOf("[IO.File]::Move");
  const binding = packaging.indexOf("patch-desktop-manifest-binding.mjs");
  assert.ok(detach >= 0 && detach < binding);
  assert.match(packaging, /\[IO\.File\]::Move\(\$Detached, \$Path, \$true\)/u);
  assert.doesNotMatch(packaging, /\[IO\.File\]::Replace/u);
  assert.match(packaging, /Get-FileHash[\s\S]*detached executable/u);
  assert.match(packaging, /"--bundles", "nsis"/u);
  assert.match(packaging, /sign-tauri-windows\.ps1/u);
  assert.match(packaging, /signCommand = \[ordered\]@\{/u);
  assert.match(packaging, /if \(\$Phase -eq "all"\) \{\s*\$BundleArguments \+= "--no-sign"/u);
  assert.match(packaging, /Tauri must patch the unsigned desktop executable before signing it/u);
  assert.match(packaging, /LastWriteTimeUtc -ge \$BundleStartedAtUtc/u);
  assert.match(packaging, /Get-FileHash/u);
  assert.match(packaging, /verify-authenticode\.ps1/u);
  const nestedVerification = packaging.indexOf('if ($Phase -eq "bind")');
  const mainBinding = packaging.indexOf('patch-desktop-manifest-binding.mjs', nestedVerification);
  const mainVerification = packaging.indexOf('if ($Phase -in @("bundle", "finalize"))', mainBinding);
  const installerVerification = packaging.indexOf('if ($Phase -eq "finalize")', mainVerification);
  assert.ok(nestedVerification < mainBinding && mainBinding < mainVerification);
  assert.ok(mainVerification < installerVerification);
  assert.equal(
    (packaging.match(/@\(\$StagedSidecar, \$StagedCli, \$StagedRipgrep\)/gu) ?? []).length,
    2,
  );

  const signer = read("scripts/ci/sign-tauri-windows.ps1");
  assert.match(signer, /Import-Module ArtifactSigning/u);
  assert.match(signer, /Invoke-ArtifactSigning/u);
  assert.match(signer, /verify-authenticode\.ps1/u);
  assert.match(signer, /"\.exe", "\.dll", "\.tmp"/u);
  assert.match(signer, /NSIS !uninstfinalize passes its PE uninstaller/u);
  assert.match(signer, /ReadUInt32\(\) -ne 0x00004550/u);
  assert.match(signer, /Tauri signing input has an invalid existing signature/u);
});

test("repository import keeps its action footer inside compact windows", () => {
  const styles = read("apps/desktop/src/styles.css");
  assert.match(
    styles,
    /\.repository-import-dialog\s*\{[^}]*grid-template-rows:\s*auto auto minmax\(0, 1fr\) auto;/su,
  );
  assert.match(
    styles,
    /\.repository-import-content\s*\{[^}]*min-height:\s*0;[^}]*height:\s*min\(420px, 44vh\);[^}]*overflow:\s*auto;/su,
  );
});

test("managed settings render compact switches instead of stretched native checkboxes", () => {
  const settings = read("apps/desktop/src/components/ManagedSettingsPane.tsx");
  const styles = read("apps/desktop/src/styles.css");
  assert.match(
    settings,
    /function SwitchInput[\s\S]*className="switch-input"[\s\S]*role="switch"/u,
  );
  assert.match(
    styles,
    /\.switch-input\s*\{[^}]*width:\s*34px;[^}]*height:\s*18px;[^}]*min-height:\s*18px;[^}]*padding:\s*0;[^}]*appearance:\s*none;[^}]*border-radius:\s*999px;/su,
  );
  assert.match(
    styles,
    /\.switch-input::after\s*\{[^}]*width:\s*12px;[^}]*height:\s*12px;[^}]*border-radius:\s*50%;/su,
  );
  assert.match(
    styles,
    /\.switch-input:checked::after\s*\{[^}]*transform:\s*translateX\(16px\);/su,
  );
});

test("session map renders compact switches instead of stretched native checkboxes", () => {
  const workspace = read("apps/desktop/src/components/SessionWorkspace.tsx");
  const styles = read("apps/desktop/src/styles.css");
  assert.equal(workspace.match(/role="switch"/gu)?.length, 2);
  assert.match(
    styles,
    /\.session-map-layers input,\s*\.session-map-lineage-toggle input\s*\{[^}]*width:\s*28px;[^}]*height:\s*15px;[^}]*min-height:\s*15px;[^}]*padding:\s*0;[^}]*appearance:\s*none;[^}]*border-radius:\s*999px;/su,
  );
  assert.match(
    styles,
    /\.session-map-layers input::after,\s*\.session-map-lineage-toggle input::after\s*\{[^}]*width:\s*11px;[^}]*height:\s*11px;[^}]*border-radius:\s*50%;/su,
  );
});

test("conversation activity markers stay centered on their timeline rail", () => {
  const styles = read("apps/desktop/src/styles.css");
  assert.match(
    styles,
    /\.run-activity-thread \.compact-tool-activity::before,\s*\.run-activity-thread \.activity-thought::before,\s*\.run-activity-thread \.feed-entry::before\s*\{[^}]*left:\s*-39px;[^}]*box-sizing:\s*border-box;[^}]*width:\s*8px;[^}]*height:\s*8px;/su,
  );
});

test("session topology uses a lazy-loaded read-only React Flow surface", () => {
  const workspace = read("apps/desktop/src/components/SessionWorkspace.tsx");
  const graph = read("apps/desktop/src/components/SessionTopologyGraph.tsx");
  const packageManifest = json("apps/desktop/package.json");
  assert.equal(packageManifest.dependencies["@xyflow/react"], "12.11.3");
  assert.match(
    workspace,
    /lazy\(\(\)\s*=>\s*import\("\.\/SessionTopologyGraph"\)/u,
  );
  assert.match(graph, /<ReactFlow<SessionFlowNode, Edge>/u);
  assert.match(graph, /type:\s*"step"/u);
  assert.match(graph, /nodesDraggable=\{false\}/u);
  assert.match(graph, /nodesConnectable=\{false\}/u);
  assert.match(graph, /elementsSelectable=\{false\}/u);
  assert.doesNotMatch(workspace, /session-map-network/u);
});

test("release and development CSPs preserve the local-only boundary", () => {
  const { security } = json("apps/desktop/src-tauri/tauri.conf.json").app;
  const release = directives(security.csp);
  const development = directives(security.devCsp);
  assert.deepEqual(release.get("connect-src"), [
    "ipc:",
    "http://ipc.localhost",
  ]);
  assert.deepEqual(development.get("connect-src"), [
    "'self'",
    "ipc:",
    "http://ipc.localhost",
    "ws://127.0.0.1:1420",
  ]);
  assert.deepEqual(release.get("script-src"), ["'self'"]);
  assert.deepEqual(release.get("style-src"), ["'self'"]);
  for (const directive of [
    "object-src",
    "base-uri",
    "frame-src",
    "child-src",
    "worker-src",
    "media-src",
    "form-action",
  ]) {
    assert.deepEqual(release.get(directive), ["'none'"]);
  }
  assert.equal(security.freezePrototype, true);
  const vite = read("apps/desktop/vite.config.ts");
  const xtermCompatibility = read(
    "apps/desktop/build/xterm-frozen-prototype.ts",
  );
  const d3ColorCompatibility = read(
    "apps/desktop/build/d3-color-frozen-prototype.ts",
  );
  assert.match(vite, /xtermFrozenPrototypeCompatibility\(\)/u);
  assert.match(vite, /d3ColorFrozenPrototypeCompatibility\(\)/u);
  assert.match(
    vite,
    /exclude:\s*\[[\s\S]*"@xterm\/xterm"[\s\S]*"@xyflow\/system"[\s\S]*"d3-color"[\s\S]*\]/u,
  );
  assert.match(xtermCompatibility, /Qn\|\|=Object\.create\(null\)/u);
  assert.match(
    d3ColorCompatibility,
    /Object\.defineProperty\(prototype, "constructor"/u,
  );

  const terminalProtocol = read(
    "apps/desktop/src-tauri/src/terminal_protocol.rs",
  );
  const terminalCspSource = terminalProtocol.match(
    /const TERMINAL_CSP: &str = "([^"]+)"/u,
  );
  assert.notEqual(terminalCspSource, null);
  const terminal = directives(terminalCspSource[1]);
  assert.deepEqual(terminal.get("script-src"), ["'self'"]);
  assert.deepEqual(terminal.get("connect-src"), [
    "ipc:",
    "http://ipc.localhost",
  ]);
  assert.deepEqual(terminal.get("style-src"), ["'self'", "'unsafe-inline'"]);
  for (const directive of [
    "object-src",
    "base-uri",
    "frame-src",
    "child-src",
    "worker-src",
    "media-src",
    "form-action",
  ]) {
    assert.deepEqual(terminal.get(directive), ["'none'"]);
  }
  assert.match(terminalProtocol, /webview_label\(\) != TERMINAL_WEBVIEW/u);
  assert.match(terminalProtocol, /request\.method\(\) != http::Method::GET/u);
  assert.match(
    read("apps/desktop/src-tauri/src/lib.rs"),
    /register_uri_scheme_protocol\(terminal_protocol::SCHEME/u,
  );
});

test("command review has isolated IPC and cannot authorize an effect without the native broker", () => {
  const capability = json(
    "apps/desktop/src-tauri/capabilities/command-approval.json",
  );
  const configuration = json("apps/desktop/src-tauri/tauri.conf.json");
  assert.ok(
    configuration.app.security.capabilities.includes(capability.identifier),
  );
  assert.deepEqual(capability.windows, ["command-approval"]);
  assert.equal(capability.local, true);
  assert.deepEqual(capability.permissions, [
    "allow-command-review-context",
    "allow-finish-command-review",
  ]);
  const protocol = read(
    "apps/desktop/src-tauri/src/command_review_protocol.rs",
  );
  const policy = directives(
    protocol.match(/const REVIEW_CSP: &str = "([^"]+)"/u)[1],
  );
  assert.deepEqual(policy.get("script-src"), ["'self'"]);
  assert.deepEqual(policy.get("connect-src"), ["ipc:", "http://ipc.localhost"]);
  for (const directive of [
    "object-src",
    "base-uri",
    "frame-src",
    "child-src",
    "worker-src",
    "media-src",
    "form-action",
  ])
    assert.deepEqual(policy.get(directive), ["'none'"]);
  const review = read("apps/desktop/src-tauri/src/command_review.rs");
  assert.match(review, /caller\.label\(\) != WINDOW/u);
  assert.match(
    review,
    /\.on_navigation\(crate::command_review_protocol::navigation_allowed\)/u,
  );
  assert.doesNotMatch(review, /\.respond_interaction\(/u);
  const commands = read("apps/desktop/src-tauri/src/commands.rs");
  const confirmation = commands.slice(commands.indexOf("async fn confirm_effect_approval("), commands.indexOf("fn native_dialog_field("));
  assert.doesNotMatch(confirmation, /blocking_show/u);
  assert.match(confirmation, /revalidate_after_lookup/u);
  assert.match(confirmation, /window\.is_current\(\)/u);
  assert.match(
    review,
    /let observed = lookup\.await\?;\s*if observed != \*expected \|\| !still_current\(\)/u,
  );
  const manifest = read("apps/desktop/src-tauri/Cargo.toml");
  assert.match(manifest, /required-features = \["approval-test-bridge"\]/u);
});

test("terminal PTY authority is isolated from the main WebView", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  const terminal = json(
    "apps/desktop/src-tauri/capabilities/terminal-pty.json",
  );
  assert.deepEqual(main.webviews, ["main"]);
  assert.equal(main.windows, undefined);
  assert.equal(main.remote, undefined);
  assert.deepEqual(terminal.webviews, ["terminal"]);
  assert.equal(terminal.windows, undefined);
  assert.equal(terminal.remote, undefined);
  assert.equal(terminal.local, true);
  assert.deepEqual(terminal.permissions, [
    "allow-terminal-context",
    "allow-open-terminal",
    "allow-write-terminal",
    "allow-resize-terminal",
    "allow-signal-terminal",
    "allow-close-terminal",
  ]);
  const permissions = [...main.permissions, ...terminal.permissions];
  assert.equal(
    permissions.some((permission) => permission.includes("shell:")),
    false,
  );
  assert.equal(
    permissions.some((permission) => permission.includes("http:")),
    false,
  );
  assert.equal(
    permissions.some((permission) => permission.includes("fs:")),
    false,
  );
  const bridge = read("apps/desktop/src-tauri/src/terminal_commands.rs");
  assert.match(bridge, /\.on_navigation\(terminal_navigation_allowed\)/u);
  assert.match(bridge, /url\.scheme\(\) == "tauri"/u);
  assert.match(bridge, /url\.query\(\) == Some\("surface=terminal"\)/u);
  const terminalManager = read("apps/desktop/src-tauri/src/terminal.rs");
  assert.match(
    terminalManager,
    /const MACOS_SYSTEM_SHELL: &str = "\/bin\/zsh"/u,
  );
  assert.match(terminalManager, /TerminalKind::Shell/u);
  assert.match(terminalManager, /command\.env_clear\(\)/u);
  assert.match(terminalManager, /ColossusHome::ensure_at/u);
  const terminalProcess = read(
    "apps/desktop/src-tauri/src/terminal_process.rs",
  );
  assert.match(terminalProcess, /COLOSSUS_HOME/u);
  assert.match(
    terminalProcess,
    /minimal_windows_environment\(colossus_home\)/u,
  );
  assert.match(terminalProcess, /minimal_environment\(colossus_home\)/u);
  const managedRuntime = read("apps/desktop/src-tauri/src/managed_runtime.rs");
  assert.match(
    managedRuntime,
    /without_automatic_agent_instructions_for_diagnostics\(\)/u,
  );
  const dto = read("apps/desktop/src-tauri/src/dto.rs");
  assert.match(dto, /deny_unknown_fields/u);
});

test("main WebView exposes every advanced configuration, Space, Aside, thread lifecycle, and updater command it calls", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  for (const permission of [
    "allow-delete-global-mcp-server",
    "allow-apply-managed-model-configuration",
    "allow-set-approval-mode",
    "allow-get-thread-delegate",
    "allow-get-session-map",
    "allow-create-space",
    "allow-list-spaces",
    "allow-select-space",
    "allow-rename-space",
    "allow-archive-space",
    "allow-restore-space",
    "allow-search-space-threads",
    "allow-list-asides",
    "allow-archive-thread",
    "allow-restore-thread",
    "allow-check-desktop-update",
    "allow-install-desktop-update",
  ]) {
    assert.ok(main.permissions.includes(permission), permission);
  }

  const bridge = read("apps/desktop/src-tauri/src/lib.rs");
  const api = read("apps/desktop/src/api.ts");
  const build = read("apps/desktop/src-tauri/build.rs");
  for (const command of [
    "delete_global_mcp_server",
    "apply_managed_model_configuration",
    "set_approval_mode",
    "get_thread_delegate",
    "get_session_map",
    "create_space",
    "list_spaces",
    "select_space",
    "rename_space",
    "archive_space",
    "restore_space",
    "search_space_threads",
    "list_asides",
    "archive_thread",
    "restore_thread",
    "check_desktop_update",
    "install_desktop_update",
  ]) {
    assert.match(bridge, new RegExp(`\\b${command}\\b`, "u"));
    assert.match(api, new RegExp(`"${command}"`, "u"));
    assert.match(build, new RegExp(`"${command}"`, "u"));
  }
  assert.doesNotMatch(bridge, /\bopen_workspace_terminal\b/u);
  assert.doesNotMatch(api, /"open_workspace_terminal"/u);
});

test("delegated-agent inspection is selected-Space scoped and releases only bounded child activity", () => {
  const command = read("apps/desktop/src-tauri/src/desktop_commands.rs");
  assert.match(command, /pub\(crate\) async fn get_thread_delegate/u);
  assert.match(command, /settings\.selected_space_id/u);
  assert.match(
    command,
    /state\.selected_target_id\(\)\.await\.as_deref\(\) != Some\(space_id\)/u,
  );
  assert.match(command, /managed_lifecycle_ready_for\(space_id\)/u);
  assert.match(command, /inspect_thread_delegate\(&parent_run_id, &job_id\)/u);

  const projection = read("crates/colossus-worker/src/delegate_inspection.rs");
  const productionProjection = projection.slice(
    0,
    projection.indexOf("#[cfg(test)]"),
  );
  assert.match(projection, /job\.parent_run_id != parent_run_id/u);
  assert.match(
    projection,
    /event\.context\.subagent_id\.as_deref\(\) != Some\(job_id\)/u,
  );
  assert.match(projection, /MAX_DELEGATE_ACTIVITIES: usize = 24/u);
  assert.match(projection, /MAX_ACTIVITY_TEXT_BYTES: usize = 64 \* 1024/u);
  assert.match(projection, /"tool\.call\.started\.v1"/u);
  assert.match(projection, /"tool\.call\.completed\.v1"/u);
  assert.doesNotMatch(
    productionProjection,
    /reasoning\.summary|model\.delta|system_message|provider_response/iu,
  );

  const renderer = read("apps/desktop/src/App.tsx");
  assert.match(
    renderer,
    /const parentRunId = participant\.parentRunId \?\? activeRun\.runId/u,
  );
  assert.match(
    renderer,
    /parentView\.run\.sessionId !== activeRun\.sessionId/u,
  );
  assert.match(renderer, /getThreadDelegate\(parentRunId, participant\.id\)/u);
  assert.match(renderer, /inspection\.parentRunId !== parentRunId/u);
  assert.doesNotMatch(renderer, /getRun\([^\n]*childRunId/u);
});

test("session map inspection is selected-Space scoped and releases bounded canonical records", () => {
  const command = read("apps/desktop/src-tauri/src/desktop_commands.rs");
  assert.match(command, /pub\(crate\) async fn get_session_map/u);
  assert.match(command, /settings\.selected_space_id/u);
  assert.match(command, /managed_lifecycle_ready_for\(space_id\)/u);
  assert.match(command, /state\.selected_target\(space_id\)\.await/u);
  assert.match(
    command,
    /state\.run_is_bound\(&target, &source_run_id\)\.await/u,
  );
  assert.match(command, /inspect_session_map\(&response\.run\.session_id\)/u);

  const projection = read(
    "crates/colossus-worker/src/session_map_inspection.rs",
  );
  assert.match(projection, /MAX_RECORDS_PER_FAMILY: usize = 32/u);
  assert.match(projection, /MAX_DOCUMENT_BYTES: usize = 16 \* 1024/u);
  assert.match(
    projection,
    /MemoryScope::Repository\(id\) => id == repository_id/u,
  );
  assert.match(projection, /MemoryScope::Session\(id\) => id == session_id/u);
  assert.match(projection, /list_subagents\(Some\(session_id\)/u);
  assert.match(projection, /list_tasks\(Some\(session_id\)/u);
  assert.match(projection, /list_research_runs\(Some\(session_id\)/u);
  assert.match(projection, /context_snapshots\(session_id\)/u);
  assert.match(projection, /bounded_snapshot_items/u);

  const renderer = read("apps/desktop/src/App.tsx");
  assert.match(renderer, /getSessionMap\(runId\)/u);
  assert.match(renderer, /next\.sessionId === sessionId/u);
  assert.match(
    renderer,
    /\["topology", "snapshots", "resources"\]\.includes\(\s*activeSessionWorkspaceView/u,
  );
  assert.match(
    renderer,
    /requestSessionMap\(run\.runId, run\.sessionId, false\)/u,
  );

  const workSurface = read("apps/desktop/src/components/WorkSurface.tsx");
  const detailsNavigation = workSurface.match(
    /function openSessionViewFromDetails[\s\S]*?\n  \}/u,
  )?.[0];
  assert.ok(
    detailsNavigation,
    "thread-details navigation helper must remain explicit",
  );
  assert.match(detailsNavigation, /changeSessionWorkspaceView\(view\)/u);
  assert.doesNotMatch(detailsNavigation, /setSessionWorkspaceView\(view\)/u);
});

test("Aside context stays canonical, selected-Space scoped, and out of metadata search", () => {
  const api = read("crates/colossus-api/src/runs.rs");
  assert.match(api, /pub struct RunBranch \{/u);
  assert.match(api, /pub source_run_id: String/u);
  assert.match(api, /pub source_message_count: u64/u);
  assert.match(api, /pub context_mode: RunBranchContextMode/u);
  assert.doesNotMatch(api, /struct RunBranch[\s\S]{0,300}prompt/u);

  const dto = read("apps/desktop/src-tauri/src/dto.rs");
  assert.match(
    dto,
    /context_mode: RunBranchContextMode::SourceRunConversation/u,
  );
  assert.match(dto, /source_message_count: 0/u);
  const rendererTypes = read("apps/desktop/src/types.ts");
  assert.doesNotMatch(rendererTypes, /sourceMessageCount/u);
  const runtime = read("crates/colossus-runtime/src/sessions_context.rs");
  assert.match(runtime, /RunBranchContextMode::Conversation/u);
  assert.match(runtime, /RunBranchContextMode::SourceRunConversation/u);
  assert.match(runtime, /ModelMessageRole::Tool => None/u);

  const service = read("crates/colossus-api-runtime/src/service.rs");
  assert.match(service, /let \(source_message_count, context_mode\)/u);
  assert.match(service, /message\.run_id == source\.id/u);

  const commands = read("apps/desktop/src-tauri/src/commands.rs");
  assert.match(commands, /require_selected_space\(&settings, &target_id\)/u);
  assert.match(commands, /filter\(\|run: &RunDto\| !aside_sessions\.contains/u);

  const settings = read("apps/desktop/src-tauri/src/desktop_settings.rs");
  const asideRecord = settings.match(
    /pub\(crate\) struct AsideSetting \{(?<record>[\s\S]*?)\n\}/u,
  );
  assert.notEqual(asideRecord, null);
  assert.doesNotMatch(
    asideRecord.groups.record,
    /prompt|quote|message|tool_output/iu,
  );

  const search = read("apps/desktop/src-tauri/src/space_search.rs");
  assert.match(search, /settings\s*\.asides\s*\.iter\(\)\s*\.any/u);
  assert.doesNotMatch(search, /prompt|tool_output|selected_text/iu);
});

test("every native capability permission is generated by the clean-build command manifest", () => {
  const build = read("apps/desktop/src-tauri/build.rs");
  const commandBlock = build.match(
    /const COMMANDS: &\[&str\] = &\[(?<commands>[\s\S]*?)\n\];/u,
  );
  assert.notEqual(commandBlock, null);
  const commands = new Set(
    [...commandBlock.groups.commands.matchAll(/"(?<command>[^"]+)"/gu)].map(
      (match) => match.groups.command,
    ),
  );
  const capabilities = [
    json("apps/desktop/src-tauri/capabilities/main-chat.json"),
    json("apps/desktop/src-tauri/capabilities/terminal-pty.json"),
    json("apps/desktop/src-tauri/capabilities/command-approval.json"),
  ];
  const allowedCommands = new Set();
  for (const capability of capabilities) {
    for (const permission of capability.permissions) {
      assert.match(permission, /^allow-[a-z0-9-]+$/u);
      const command = permission.slice("allow-".length).replaceAll("-", "_");
      allowedCommands.add(command);
      assert.ok(
        commands.has(command),
        `${capability.identifier} permission ${permission} is missing ${command} from build.rs COMMANDS`,
      );
    }
  }

  const rendererApi = read("apps/desktop/src/api.ts");
  const rendererCommands = new Set(
    [
      ...rendererApi.matchAll(/\bcall(?:<[^>]+>)?\(\s*"(?<command>[^"]+)"/gu),
    ].map((match) => match.groups.command),
  );
  for (const command of rendererCommands) {
    assert.ok(
      allowedCommands.has(command),
      `renderer command ${command} must be allowed by a native capability`,
    );
  }
});

test("workspace file preview is read-only, bounded, and workspace-bound", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  assert.ok(main.permissions.includes("allow-list-workspace-directory"));
  assert.ok(main.permissions.includes("allow-read-workspace-file"));

  const source = read("apps/desktop/src-tauri/src/workspace_files.rs");
  const implementation = source.slice(0, source.indexOf("#[cfg(test)]"));
  assert.match(implementation, /MAX_FILE_BYTES: u64 = 256 \* 1_024/u);
  assert.match(
    implementation,
    /settings\.selected_target_id != settings\.selected_space_id/u,
  );
  assert.match(
    implementation,
    /settings\.access_profile == AccessProfileSetting::Minimal/u,
  );
  assert.match(implementation, /revalidate_workspace\(workspace\)/u);
  assert.match(implementation, /OFlags::NOFOLLOW/u);
  assert.match(
    implementation,
    /colossus_windows_native::BoundPath::open_file/u,
  );
  assert.match(implementation, /binding\.revalidate\(\)/u);
  assert.doesNotMatch(
    implementation,
    /std::os::windows::fs::MetadataExt|file_index\(\)|volume_serial_number\(\)/u,
  );
  assert.match(implementation, /\.file_type\(\)\.is_symlink\(\)/u);
  assert.match(implementation, /"\.colossus"/u);
  assert.match(implementation, /"\.env"/u);
  assert.match(implementation, /"pem" \| "key"/u);
  assert.doesNotMatch(implementation, /write_all|create_dir|remove_file/u);
});

test("Space search indexes only bounded released thread metadata", () => {
  const source = read("apps/desktop/src-tauri/src/space_search.rs");
  const record = source.match(
    /struct ThreadSummaryRecord \{(?<fields>[\s\S]*?)\n\}/u,
  );
  assert.notEqual(record, null);
  assert.match(record.groups.fields, /space_id: String/u);
  assert.match(record.groups.fields, /run_id: String/u);
  assert.match(record.groups.fields, /session_id: String/u);
  assert.match(record.groups.fields, /title: String/u);
  assert.match(record.groups.fields, /mode: String/u);
  assert.match(record.groups.fields, /status: String/u);
  assert.match(record.groups.fields, /updated_at: String/u);
  assert.match(record.groups.fields, /archived: bool/u);
  assert.doesNotMatch(
    record.groups.fields,
    /prompt|message|tool|output|credential|secret|path/iu,
  );
  assert.match(source, /const MAX_TITLE_BYTES: usize = 512/u);
  assert.match(source, /const MAX_SEARCH_RESULTS: usize = 100/u);
  assert.match(
    source,
    /\(space\.archived \|\| record\.archived\) && !include_archived/u,
  );
  assert.match(source, /thread_archived: record\.archived/u);

  const dto = read("apps/desktop/src-tauri/src/desktop_dto.rs");
  const event = dto.match(
    /struct SpaceStatusEventDto \{(?<fields>[\s\S]*?)\n\}/u,
  );
  assert.notEqual(event, null);
  assert.doesNotMatch(
    event.groups.fields,
    /path|prompt|message|tool|output|credential|secret/iu,
  );
  const commands = read("apps/desktop/src-tauri/src/desktop_commands.rs");
  assert.match(commands, /app\.emit_to\(\s*tauri::EventTarget::webview\("main"\),\s*"space-status-changed"/u);
  assert.match(commands, /"space-attention"/u);
});

test("Managed Local trusted tool ceiling exactly matches declared built-ins", () => {
  const builtinSource = read("crates/colossus-tools/src/builtin.rs");
  const builtinNames = [
    ...builtinSource.matchAll(/name: "([a-z][a-z0-9_.-]+)"\.into\(\)/gu),
  ].map((match) => match[1]);
  assert.ok(builtinNames.length > 0);
  assert.match(
    builtinSource,
    /specs\.extend\(super::process_sessions::session_specs\(\)\)/u,
  );
  const sessionDeclarations = read(
    "crates/colossus-tools/src/process_sessions.rs",
  ).match(/fn session_specs\(\)[^{]*\{\s*\[(?<names>[^\]]+)\]\s*\.into_iter\(\)/u);
  assert.notEqual(sessionDeclarations, null);
  builtinNames.push(
    ...[...sessionDeclarations.groups.names.matchAll(/"([a-z][a-z0-9_.-]+)"/gu)]
      .map((match) => match[1]),
  );

  const runtimeSource = read("apps/desktop/src-tauri/src/managed_runtime.rs");
  const grantStart = runtimeSource.indexOf("const TRUSTED_BUILTIN_TOOL_GRANT");
  const grantEnd = runtimeSource.indexOf("];", grantStart);
  assert.ok(grantStart >= 0 && grantEnd > grantStart);
  const grantNames = [
    ...runtimeSource
      .slice(grantStart, grantEnd)
      .matchAll(/"([a-z][a-z0-9_.-]+)"/gu),
  ].map((match) => match[1]);

  assert.deepEqual(new Set(grantNames), new Set(builtinNames));
  assert.equal(grantNames.length, new Set(grantNames).size);
});

test("Windows private storage is created with a protected native DACL", () => {
  const settingsSource = read("apps/desktop/src-tauri/src/desktop_settings.rs");
  const testModule = settingsSource.search(/^#\[cfg\(test\)\]\r?\nmod tests/mu);
  assert.notEqual(testModule, -1);
  const settings = settingsSource.slice(0, testModule);
  assert.match(
    settings,
    /colossus_windows_native::create_private_directory\(path\)/u,
  );
  assert.match(settings, /validate_private_owner_dacl\(\)/u);

  const native = read("crates/colossus-windows-native/src/windows.rs");
  assert.match(native, /CreateDirectoryW/u);
  assert.match(native, /SE_DACL_PROTECTED/u);
  assert.match(native, /SetSecurityDescriptorOwner/u);
  assert.match(native, /WinLocalSystemSid/u);
  assert.match(native, /WinBuiltinAdministratorsSid/u);
  assert.match(native, /parent\.revalidate\(\)/u);
});

test("CA bundle management stays native and exposes only sanitized trust metadata", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  assert.ok(main.permissions.includes("allow-import-ca-bundle"));
  assert.ok(main.permissions.includes("allow-remove-ca-bundle"));

  const commands = read("apps/desktop/src-tauri/build.rs");
  const bridge = read("apps/desktop/src-tauri/src/lib.rs");
  for (const command of ["import_ca_bundle", "remove_ca_bundle"]) {
    assert.match(commands, new RegExp(`"${command}"`, "u"));
    assert.match(bridge, new RegExp(`\\b${command}\\b`, "u"));
  }

  const dto = read("apps/desktop/src-tauri/src/desktop_dto.rs");
  const caStatus = dto.slice(
    dto.indexOf("pub(crate) struct CaBundleStatusDto"),
    dto.indexOf("impl CaBundleStatusDto"),
  );
  assert.match(caStatus, /configured/u);
  assert.match(caStatus, /certificate_count/u);
  assert.match(caStatus, /fingerprints_sha256/u);
  assert.doesNotMatch(caStatus, /path|pem|source/u);

  const types = read("apps/desktop/src/types.ts");
  const rendererStatus = types.slice(
    types.indexOf("export interface CaBundleStatus"),
    types.indexOf("export interface DesktopCapabilities"),
  );
  assert.match(rendererStatus, /configured/u);
  assert.match(rendererStatus, /certificateCount/u);
  assert.match(rendererStatus, /fingerprintsSha256/u);
  assert.doesNotMatch(rendererStatus, /path|pem|source/u);
});

test("provider enrollment and external trust stay behind native UI", () => {
  const types = read("apps/desktop/src/types.ts");
  const configureRequest = types.slice(
    types.indexOf("export interface ConfigureManagedRuntimeRequest"),
    types.indexOf("export type CredentialAction"),
  );
  assert.match(configureRequest, /baseUrl/u);
  assert.match(configureRequest, /credentialId/u);
  assert.doesNotMatch(configureRequest, /apiKey|credentialValue|secretValue/u);
  const managedConfigurationRequest = types.slice(
    types.indexOf("export type CredentialAction"),
    types.indexOf("export interface ManagedFieldOverride"),
  );
  assert.match(managedConfigurationRequest, /baseUrl/u);
  assert.match(managedConfigurationRequest, /credentialAction/u);
  assert.doesNotMatch(
    managedConfigurationRequest,
    /apiKey|credentialValue|secret/u,
  );
  const managedSettingsContracts = types.slice(
    types.indexOf("export interface ManagedFieldOverride"),
    types.indexOf("export type TerminalKind"),
  );
  assert.doesNotMatch(
    managedSettingsContracts,
    /apiKey|credentialValue|secretValue/u,
  );

  const onboarding = read("apps/desktop/src/components/OnboardingSurface.tsx");
  assert.doesNotMatch(onboarding, /type=["']password["']/u);
  assert.match(onboarding, /API base URL/u);
  assert.match(onboarding, /enter your API key in a separate secure window/u);
  assert.match(onboarding, /saves the key encrypted on this computer/u);
  const modelEditor = read(
    "apps/desktop/src/components/ModelConfigurationEditor.tsx",
  );
  assert.match(modelEditor, /contextWindowTokens/u);
  assert.match(modelEditor, /maxOutputTokens/u);
  assert.match(modelEditor, /credentialAction/u);
  assert.doesNotMatch(
    modelEditor,
    /type=["']password["']|apiKey|credentialValue/u,
  );

  const enrollment = read("apps/desktop/src-tauri/src/provider_enrollment.rs");
  const enrollmentImplementation = enrollment.slice(
    0,
    enrollment.indexOf("#[cfg(test)]"),
  );
  assert.match(
    enrollmentImplementation,
    /colossus_native_credential_ui::prompt/u,
  );
  assert.match(enrollmentImplementation, /HostSecret/u);
  assert.doesNotMatch(
    enrollmentImplementation,
    /osascript|CredUI|Command::new/u,
  );
  assert.doesNotMatch(
    enrollmentImplementation,
    /api\.openai\.com|openrouter\.ai/u,
  );
  assert.doesNotMatch(enrollmentImplementation, /starts_with|sk-or-v1/u);

  const commands = read("apps/desktop/src-tauri/src/desktop_commands.rs");
  assert.match(commands, /fn reusable_provider_credential/u);
  assert.match(commands, /!request\.replace_credential/u);
  assert.match(commands, /provider\.kind == request\.provider_kind/u);
  assert.match(commands, /verify_reused_provider_credential/u);
  assert.match(commands, /fn access_profile_elevation/u);
  assert.match(commands, /confirm_access_profile\(&app/u);
  assert.match(commands, /fn execution_boundary_elevation/u);
  assert.match(commands, /confirm_execution_boundary\(&app/u);
  assert.match(commands, /fn confirm_provider_origins/u);
  assert.match(commands, /fn rollback_staged_provider_credentials/u);
  assert.match(commands, /fn reject_active_managed_runs/u);
  assert.match(commands, /request_provider_secret\(credential_parent/u);
  assert.match(commands, /DesktopCredentials::for_settings/u);
  assert.match(commands, /Unsafe: Full access/u);
  assert.match(commands, /approval mode is a separate setting/u);
  for (const action of ["Import", "Connect", "Select", "Remove"]) {
    assert.match(commands, new RegExp(`ExternalConsentAction::${action}`, "u"));
  }
  assert.match(commands, /Certificate SHA-256:/u);
  assert.match(commands, /\.blocking_show\(\)/u);
  assert.doesNotMatch(read("apps/desktop/src/App.tsx"), /window\.confirm/u);
});

test("release packaging records hashes only after nested signing", () => {
  const source = read("scripts/package-desktop-macos");
  const sidecar = source.indexOf('sign_one "$sidecar"');
  const cli = source.indexOf('sign_one "$cli"');
  const ripgrep = source.indexOf('sign_one "$rg"');
  const manifest = source.indexOf("write-desktop-bundle-manifest.mjs");
  const binding = source.indexOf("patch-desktop-manifest-binding.mjs");
  const main = source.indexOf('sign_one "$main"');
  const app = source.indexOf('sign_one "$app"');
  assert.ok(sidecar >= 0 && sidecar < cli && cli < ripgrep && ripgrep < manifest);
  assert.match(source, /stage-ripgrep\.mjs/u);
  assert.match(source, /--ripgrep "\$rg"/u);
  assert.ok(manifest < binding && binding < main && main < app);
  assert.equal(/codesign\s+--force[^\n]*--deep/u.test(source), false);
  assert.match(source, /COLOSSUS_DESKTOP_NOTARY_KEYCHAIN/u);
  assert.match(source, /--keychain "\$notary_keychain" --wait/u);
  assert.match(source, /COLOSSUS_DESKTOP_RELEASE_VERSION/u);
  assert.match(source, /build --config "\$tauri_override" --no-sign/u);
  assert.match(source, /\n    build\)/u);
  assert.match(source, /\n    sign\)/u);
  assert.match(source, /Colossus Desktop\.unsigned\.zip/u);
  assert.match(source, /\/usr\/bin\/ditto -c -k --keepParent/u);
  assert.match(source, /Built credential-free unsigned/u);
  assert.match(source, /build mode rejects signing identity state/u);
  assert.match(source, /sign ABSOLUTE_APP_PATH/u);
  assert.match(source, /prepare_resources_directory/u);
  assert.match(
    source,
    /resources must remain inside the canonical application bundle/u,
  );
  assert.match(source, /COLOSSUS_DESKTOP_TEAM_ID/u);
  assert.match(source, /COLOSSUS_DESKTOP_RELEASE_CHANNEL/u);
  assert.match(source, /developer_preview \| validation_only/u);
  assert.match(source, /release_channel" = developer_preview/u);
  assert.match(source, /not notarized/u);
  assert.match(source, /--release-channel "\$release_channel"/u);
  assert.match(source, /--identifier "\$code_identifier"/u);
  assert.match(source, /TeamIdentifier=/u);
  assert.match(source, /com\.obscuritylabs\.colossus\.desktop\.sidecar/u);
  assert.match(source, /com\.obscuritylabs\.colossus\.desktop\.cli/u);
  assert.match(source, /Managed Local runtime intentionally rejects it/u);
  assert.match(source, /--executable "\$main"/u);
  assert.match(source, /--manifest "\$manifest"/u);
  const staple = source.indexOf('xcrun stapler staple "$app"');
  const postStapleSignature = source.indexOf(
    'codesign --verify --deep --strict --verbose=2 "$app"',
    staple,
  );
  const postStapleManifest = source.indexOf(
    'verify-desktop-bundle.mjs"',
    staple,
  );
  const archive = source.lastIndexOf("/usr/bin/ditto -c -k --keepParent");
  assert.ok(
    staple >= 0 &&
      staple < postStapleSignature &&
      postStapleSignature < postStapleManifest &&
      postStapleManifest < archive,
  );

  const build = read("apps/desktop/src-tauri/build.rs");
  assert.match(build, /"unsealed_release"/u);
  assert.match(build, /"0"\.repeat\(64\)/u);
  assert.match(build, /COLOSSUS_DESKTOP_TARGET_TRIPLE/u);
  assert.match(build, /COLOSSUS_DESKTOP_RELEASE_CHANNEL/u);
  assert.match(build, /cargo:rustc-env=\{TEAM_VARIABLE\}=\{team_id\}/u);
  assert.match(build, /"developer_preview" \| "validation_only"/u);
  assert.match(build, /team_id == "ADHOC"/u);
  assert.match(build, /schema_version: 2/u);
  assert.match(
    build,
    /env::var\("PROFILE"\)\.as_deref\(\) == Ok\("debug"\) && local\.is_file\(\)/u,
  );

  const runtime = read("apps/desktop/src-tauri/src/bundle.rs");
  assert.match(runtime, /env!\("COLOSSUS_DESKTOP_TEAM_ID"\)/u);
  assert.match(runtime, /env!\("COLOSSUS_DESKTOP_RELEASE_CHANNEL"\)/u);
  assert.match(runtime, /ReleaseChannel::DeveloperPreview/u);
  assert.match(runtime, /configured_team == "ADHOC"/u);
  assert.match(runtime, /Some\("not set"\)/u);
  assert.match(runtime, /ReleaseChannel::ValidationOnly/u);
  assert.match(runtime, /identifier != expected_identifier/u);
  assert.match(runtime, /team != expected_team/u);
  assert.match(runtime, /std::hint::black_box\(&RELEASE_MANIFEST_BINDING\)/u);
  assert.match(runtime, /rustix::fs::OFlags::NOFOLLOW/u);
  assert.match(runtime, /verify_release_manifest_binding/u);
  assert.match(runtime, /com\.obscuritylabs\.colossus\.desktop\.sidecar/u);
  assert.match(runtime, /com\.obscuritylabs\.colossus\.desktop\.cli/u);
});

test("portable Desktop validation owns formatting and canonical line endings", () => {
  const attributes = read(".gitattributes");
  assert.match(attributes, /^\* text=auto eol=lf$/mu);

  const checks = read("xtask/src/checks/surfaces.rs");
  const desktopStart = checks.indexOf("pub(super) fn desktop");
  const docsStart = checks.indexOf("pub(super) fn docs", desktopStart);
  assert.ok(desktopStart >= 0 && docsStart > desktopStart);
  const desktop = checks.slice(desktopStart, docsStart);
  assert.match(
    desktop,
    /\.args\(\[\s*"fmt",\s*"--manifest-path",\s*"apps\/desktop\/src-tauri\/Cargo\.toml",\s*"--",\s*"--check",\s*\]\)/u,
  );
});

test("pre-merge desktop packaging declares its non-runnable trust channel", () => {
  const workflow = read(".github/workflows/premerge.yml");
  const acceptanceStart = workflow.indexOf("  macos-desktop-acceptance:");
  const bundleStart = workflow.indexOf("  macos-desktop-bundle:", acceptanceStart);
  const runtimeStart = workflow.indexOf("  windows-runtime:", bundleStart);
  const windowsDesktopStart = workflow.indexOf("  windows-desktop:", runtimeStart);
  const windowsEnd = workflow.indexOf("  fuzz:", windowsDesktopStart);
  assert.ok(
    acceptanceStart >= 0 &&
      bundleStart > acceptanceStart &&
      runtimeStart > bundleStart &&
      windowsDesktopStart > runtimeStart &&
      windowsEnd > windowsDesktopStart,
  );
  const acceptance = workflow.slice(acceptanceStart, bundleStart);
  const bundle = workflow.slice(bundleStart, runtimeStart);
  assert.match(acceptance, /npm run test:browser-native/u);
  assert.doesNotMatch(acceptance, /npm run tauri:build/u);
  assert.match(bundle, /COLOSSUS_DESKTOP_TEAM_ID: "ADHOC"/u);
  assert.match(bundle, /COLOSSUS_DESKTOP_RELEASE_CHANNEL: "validation_only"/u);
  assert.match(bundle, /Build validation-only ADHOC macOS bundle structure/u);
  assert.doesNotMatch(bundle, /npm run test:browser-native/u);

  const windows = workflow.slice(runtimeStart, windowsDesktopStart);
  const windowsDesktop = workflow.slice(windowsDesktopStart, windowsEnd);
  assert.match(windows, /runs-on: windows-2025/u);
  assert.match(windowsDesktop, /runs-on: windows-latest-l/u);
  assert.match(windowsDesktop, /COLOSSUS_DESKTOP_TEAM_ID: "UNSIGNED"/u);
  assert.match(windowsDesktop, /cargo xtask desktop prepare --profile debug/u);
  assert.match(
    windowsDesktop,
    /cargo test --locked --manifest-path apps\/desktop\/src-tauri\/Cargo\.toml --lib/u,
  );
  assert.match(windows, /npm run typecheck/u);
  assert.match(windows, /npm run test/u);
  assert.match(windows, /npm run check:security/u);
  assert.doesNotMatch(windows, /npm run format:check/u);
  assert.doesNotMatch(windows, /run: npm run check\s*$/mu);
  assert.match(windows, /continue-on-error: true/u);
  assert.match(windows, /steps\.renderer_typecheck\.outcome/u);
  assert.match(windows, /steps\.renderer_tests\.outcome/u);
  assert.match(windows, /steps\.renderer_contracts\.outcome/u);
  assert.match(windows, /steps\.windows_native\.outcome/u);
  assert.match(
    windows,
    /cargo test --locked -p colossus-sdk --lib --features sidecar\s+native_sidecar::tests::ordinary_windows_instance_path_matches_its_bound_canonical_identity/u,
  );
  assert.match(
    windows,
    /if \(\$LASTEXITCODE -ne 0\) \{ exit \$LASTEXITCODE \}\s+cargo test --locked -p colossus-sdk --lib --features sidecar sidecar_agent_runs::tests/u,
  );
  assert.match(windows, /steps\.windows_sdk_path\.outcome/u);
  assert.match(windows, /steps\.worker_acceptance\.outcome/u);
  assert.match(windowsDesktop, /steps\.desktop_prepare\.outcome/u);
  assert.match(windowsDesktop, /steps\.native_clippy\.outcome/u);
  assert.match(windowsDesktop, /steps\.native_tests\.outcome/u);
  assert.match(windowsDesktop, /if: steps\.desktop_prepare\.outcome == 'success'/u);
  assert.match(windows, /Require every Windows runtime acceptance check/u);
  assert.match(windowsDesktop, /Require every Windows Desktop acceptance check/u);
  assert.ok(
    windows.indexOf("npm run typecheck") < windows.indexOf("Install Rust 1.96"),
  );
});

test("Developer Preview compilation and ad-hoc signing use separate runners", () => {
  const workflow = read(".github/workflows/release.yml");
  const buildStart = workflow.indexOf("  desktop_macos_build:");
  const signStart = workflow.indexOf("  desktop_macos:", buildStart);
  const windowsStart = workflow.indexOf(
    "  desktop_windows_preview:",
    signStart,
  );
  const signedWindowsStart = workflow.indexOf("  desktop_windows_signed:", windowsStart);
  const gateStart = workflow.indexOf("  gate:", signedWindowsStart);
  assert.ok(
    buildStart >= 0 &&
      buildStart < signStart &&
      signStart < windowsStart &&
      windowsStart < signedWindowsStart &&
      windowsStart < gateStart,
  );

  const buildJob = workflow.slice(buildStart, signStart);
  const signJob = workflow.slice(signStart, windowsStart);
  const windowsJob = workflow.slice(windowsStart, signedWindowsStart);
  const signedWindowsJob = workflow.slice(signedWindowsStart, gateStart);
  assert.match(buildJob, /npm ci --ignore-scripts/u);
  assert.match(buildJob, /package-desktop-macos build/u);
  assert.match(buildJob, /Colossus Desktop\.unsigned\.zip/u);
  assert.match(buildJob, /CARGO_TARGET_DIR/u);
  assert.match(buildJob, /CARGO_INCREMENTAL: "0"/u);
  assert.match(
    buildJob,
    /COLOSSUS_DESKTOP_RELEASE_CHANNEL: \$\{\{ needs\.validate\.outputs\.release_channel \}\}/u,
  );
  assert.doesNotMatch(buildJob, /MACOS_DEVELOPER_ID_P12/u);
  assert.doesNotMatch(buildJob, /MACOS_NOTARY/u);
  assert.doesNotMatch(buildJob, /security import/u);
  assert.doesNotMatch(buildJob, /\$\{\{ secrets\./u);
  assert.doesNotMatch(buildJob, /\$\{\{ vars\./u);
  assert.match(
    buildJob,
    /if: needs\.validate\.outputs\.target_channel != 'stable'/u,
  );
  assert.match(buildJob, /COLOSSUS_DESKTOP_UPDATE_ENDPOINT: ""/u);
  assert.match(buildJob, /COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY: ""/u);
  assert.match(buildJob, /COLOSSUS_DESKTOP_TEAM_ID=ADHOC/u);

  assert.match(signJob, /actions\/download-artifact@[0-9a-f]{40}/u);
  assert.match(signJob, /\/usr\/bin\/ditto -x -k/u);
  assert.match(signJob, /verify-desktop-unsigned-archive\.mjs/u);
  assert.match(signJob, /--extracted-root "\$destination"/u);
  assert.match(signJob, /test ! -e "\$destination"/u);
  assert.match(signJob, /protected_paths=\(/u);
  assert.match(signJob, /protected_hashes=\(\)/u);
  assert.match(signJob, /realpathSync\(process\.execPath\)/u);
  for (const protectedScript of [
    "package-desktop-macos",
    "write-desktop-bundle-manifest.mjs",
    "patch-desktop-manifest-binding.mjs",
    "verify-desktop-bundle.mjs",
    "verify-desktop-unsigned-archive.mjs",
  ]) {
    assert.match(
      signJob,
      new RegExp(protectedScript.replaceAll(".", "\\."), "u"),
    );
  }
  assert.match(signJob, /package-desktop-macos sign/u);
  assert.match(
    signJob,
    /COLOSSUS_DESKTOP_RELEASE_CHANNEL: \$\{\{ needs\.validate\.outputs\.release_channel \}\}/u,
  );
  assert.match(
    signJob,
    /if: needs\.validate\.outputs\.target_channel != 'stable'/u,
  );
  assert.doesNotMatch(signJob, /security import/u);
  assert.doesNotMatch(signJob, /\$\{\{ secrets\./u);
  assert.doesNotMatch(signJob, /\$\{\{ vars\./u);
  assert.doesNotMatch(signJob, /DESKTOP_UPDATE_PRIVATE_KEY/u);
  assert.match(signJob, /COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY: ""/u);
  assert.match(signJob, /actions\/setup-node@[0-9a-f]{40}/u);
  assert.doesNotMatch(signJob, /rust-toolchain@/u);
  assert.doesNotMatch(signJob, /\bcargo\s/u);
  assert.doesNotMatch(signJob, /\btauri\s/u);

  assert.match(
    windowsJob,
    /if: needs\.validate\.outputs\.publish_draft != 'true' && needs\.validate\.outputs\.target_channel != 'stable'/u,
  );
  assert.match(windowsJob, /runs-on: windows-latest-l/u);
  assert.match(windowsJob, /COLOSSUS_DESKTOP_TEAM_ID: UNSIGNED/u);
  assert.match(windowsJob, /package-desktop-windows\.ps1/u);
  assert.match(windowsJob, /Get-FileHash/u);
  assert.match(windowsJob, /codeSigning = "unsigned_validation_only"/u);
  assert.match(windowsJob, /smartScreenWarningExpected = \$true/u);
  assert.match(windowsJob, /Start-Process -FilePath \$installer/u);
  assert.match(windowsJob, /Start-Process -FilePath \$uninstallers/u);
  assert.match(windowsJob, /Colossus processes remained after uninstall/u);

  assert.match(signedWindowsJob, /runs-on: windows-latest-l/u);
  assert.match(signedWindowsJob, /environment: release-signing/u);
  assert.match(signedWindowsJob, /id-token: write/u);
  assert.match(signedWindowsJob, /COLOSSUS_DESKTOP_TEAM_ID: OBSCURITY_LABS_LLC/u);
  assert.match(signedWindowsJob, /azure\/login@[0-9a-f]{40}/u);
  assert.equal((signedWindowsJob.match(/azure\/artifact-signing-action@[0-9a-f]{40}/gu) ?? []).length, 1);
  const signedBuild = signedWindowsJob.indexOf("-Phase build");
  const nestedSign = signedWindowsJob.indexOf("Sign bundled sidecar, CLI, and ripgrep");
  const bind = signedWindowsJob.indexOf("-Phase bind");
  const bundle = signedWindowsJob.indexOf("-Phase bundle");
  const finalize = signedWindowsJob.indexOf("-Phase finalize");
  assert.ok(signedBuild < nestedSign && nestedSign < bind && bind < bundle);
  assert.ok(bundle < finalize);
  assert.match(
    signedWindowsJob.slice(nestedSign, bind),
    /binaries\\rg-x86_64-pc-windows-msvc\.exe/u,
  );
  assert.match(signedWindowsJob, /Bundle NSIS and sign patched app, uninstaller, and installer/u);
  assert.match(signedWindowsJob, /codeSigning = 'azure_artifact_signing'/u);
  assert.match(signedWindowsJob, /Start-Process -FilePath \$installed\[0\]\.FullName/u);

  const archiveCheck = signJob.indexOf(
    "node ./scripts/verify-desktop-unsigned-archive.mjs",
  );
  const hashCapture = signJob.indexOf("protected_hashes=()");
  const extraction = signJob.indexOf("/usr/bin/ditto -x -k");
  const hashComparison = signJob.indexOf(
    'for index in "${!protected_paths[@]}"',
    extraction,
  );
  const extractedCheck = signJob.indexOf("--extracted-root", extraction);
  const adHocSign = signJob.indexOf("package-desktop-macos sign");
  assert.ok(
    hashCapture >= 0 &&
      hashCapture < archiveCheck &&
      archiveCheck < extraction &&
      extraction < hashComparison &&
      hashComparison < extractedCheck &&
      extraction < extractedCheck &&
      extractedCheck < adHocSign,
  );

  assert.match(
    workflow,
    /MACOS_DESKTOP_BUILD_RESULT: \$\{\{ needs\.desktop_macos_build\.result \}\}/u,
  );
  assert.match(
    workflow,
    /SDK_RELEASE_RESULT: \$\{\{ needs\.sdk_release\.result \}\}/u,
  );
  assert.match(
    workflow,
    /if \[ "\$TARGET_CHANNEL" = stable \]; then[\s\S]*test "\$MACOS_DESKTOP_RESULT" = skipped[\s\S]*test "\$WINDOWS_DESKTOP_RESULT" = skipped/u,
  );
  assert.match(workflow, /desktop_windows_signed="\$WINDOWS_SIGNED_DESKTOP_RESULT"/u);
});

test("standalone Desktop release builds stay bounded before sealed packaging", () => {
  const manifest = read("apps/desktop/src-tauri/Cargo.toml");
  const profileStart = manifest.indexOf("[profile.release]");
  const profileEnd = manifest.indexOf("\n[", profileStart + 1);
  assert.ok(profileStart >= 0 && profileEnd > profileStart);
  const profile = manifest.slice(profileStart, profileEnd);
  assert.match(profile, /lto = "thin"/u);
  assert.match(profile, /codegen-units = 1/u);
  assert.match(profile, /strip = "symbols"/u);

  const patcher = read("scripts/patch-desktop-manifest-binding.mjs");
  assert.match(patcher, /MAX_EXECUTABLE_BYTES = 1024 \* 1024 \* 1024/u);
});

test("Windows signed releases use manual updates until a separate updater key is configured", () => {
  const manifest = read("apps/desktop/src-tauri/Cargo.toml");
  const build = read("apps/desktop/src-tauri/build.rs");
  const updater = read("apps/desktop/src-tauri/src/updates.rs");
  const macos = read("scripts/package-desktop-macos");
  const windows = read("scripts/package-desktop-windows.ps1");
  const release = read(".github/workflows/release.yml");
  const channels = read(".github/workflows/desktop-update-channels.yml");

  assert.match(manifest, /tauri-plugin-updater = \{ version = "=2\.9\.0"/u);
  assert.match(build, /COLOSSUS_DESKTOP_UPDATE_ENDPOINT/u);
  assert.match(build, /COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY/u);
  assert.match(build, /let updates_enabled = release_channel == "stable"/u);
  assert.match(build, /target_os == "windows" && update_endpoint\.is_empty\(\) && update_public_key\.is_empty\(\)/u);
  assert.match(
    build,
    /Desktop builds without a configured update channel must not advertise/u,
  );
  assert.match(updater, /AdditionalRootCertificates/u);
  assert.match(updater, /MAX_UPDATE_BYTES/u);
  assert.match(updater, /verify_update_signature/u);
  assert.match(updater, /download_url\.scheme\(\)/u);
  assert.match(updater, /schemaVersion/u);
  assert.match(updater, /attempt\.url\(\)\.scheme\(\) == "https"/u);
  assert.match(macos, /Colossus Desktop\.app\.tar\.gz/u);
  assert.match(macos, /if \[ "\$release_channel" = stable \]; then/u);
  assert.match(macos, /signer sign "\$updater_archive"/u);
  assert.match(windows, /createUpdaterArtifacts = \$false/u);
  assert.match(
    windows,
    /Windows packaging unexpectedly created an updater signature/u,
  );
  assert.match(release, /COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY: ""/u);
  assert.match(release, /COLOSSUS_DESKTOP_UPDATE_ENDPOINT: ""/u);
  assert.doesNotMatch(release, /DESKTOP_UPDATE_PRIVATE_KEY/u);
  assert.doesNotMatch(release, /write-desktop-update-manifest\.mjs/u);
  assert.doesNotMatch(release, /verify-tauri-updater-signature\.mjs/u);
  assert.match(channels, /types: \[published\]/u);
  assert.match(channels, /github\.event\.release\.prerelease == false/u);
  assert.match(
    channels,
    /contains\(github\.event\.release\.assets\.\*\.name, 'stable\.json'\)/u,
  );
  assert.doesNotMatch(channels, /developer_preview/u);
  assert.match(channels, /desktop-update-channels/u);
  assert.match(channels, /gh release upload "\$channel_tag"/u);
});

test("Windows CLI archives are signed before final release hashing", () => {
  const workflow = read(".github/workflows/release.yml");
  const buildStart = workflow.indexOf("  artifacts:");
  const signStart = workflow.indexOf("  windows_cli_sign:", buildStart);
  const bootstrapStart = workflow.indexOf("  bootstrap_installers:", signStart);
  const gateStart = workflow.indexOf("  gate:", bootstrapStart);
  assert.ok(buildStart >= 0 && buildStart < signStart && signStart < bootstrapStart);

  const build = workflow.slice(buildStart, signStart);
  const sign = workflow.slice(signStart, bootstrapStart);
  const gate = workflow.slice(gateStart);
  assert.match(build, /unsigned-colossus-\{0\}/u);
  assert.match(sign, /if: needs\.validate\.outputs\.publish_draft == 'true'/u);
  assert.match(sign, /environment: release-signing/u);
  assert.match(sign, /runs-on: windows-2025/u);
  assert.match(sign, /id-token: write/u);
  assert.match(sign, /target: \[x86_64-pc-windows-msvc, aarch64-pc-windows-msvc\]/u);
  assert.match(sign, /azure\/login@[0-9a-f]{40}/u);
  assert.match(sign, /azure\/artifact-signing-action@[0-9a-f]{40}/u);
  const unsignedCheck = sign.indexOf("unsigned CLI checksum mismatch");
  const azureSign = sign.indexOf("Sign CLI executable");
  const signedCheck = sign.indexOf("Verify signed CLI and seal release archive");
  const upload = sign.indexOf("Upload signed CLI archive and checksum");
  assert.ok(unsignedCheck < azureSign && azureSign < signedCheck && signedCheck < upload);
  assert.match(gate, /windows_cli_sign="\$WINDOWS_CLI_SIGN_RESULT"/u);
  assert.match(gate, /test "\$WINDOWS_CLI_SIGN_RESULT" = skipped/u);
  assert.match(
    gate,
    /if \[ "\$TARGET_CHANNEL" = stable \]; then[\s\S]*if \[ "\$\{\{ needs\.validate\.outputs\.publish_draft \}\}" = true \]; then[\s\S]*desktop_windows_signed="\$WINDOWS_SIGNED_DESKTOP_RESULT"[\s\S]*test "\$WINDOWS_SIGNED_DESKTOP_RESULT" = skipped/u,
  );
});

test("desktop browser acceptance covers the supported minimum layout", () => {
  const packageManifest = JSON.parse(read("apps/desktop/package.json"));
  const config = read("apps/desktop/playwright.config.ts");
  const acceptance = read(
    "apps/desktop/tests/browser/operations-studio.spec.ts",
  );
  const premerge = read(".github/workflows/premerge.yml");

  assert.equal(packageManifest.devDependencies["@playwright/test"], "1.62.0");
  assert.equal(
    packageManifest.devDependencies["@axe-core/playwright"],
    "4.12.1",
  );
  assert.equal(packageManifest.scripts["test:browser"], "playwright test");
  assert.equal(
    packageManifest.scripts["test:browser:install"],
    "playwright install chromium",
  );
  assert.match(config, /viewport: \{ width: 880, height: 640 \}/u);
  assert.match(config, /fixture=operations-studio/u);
  assert.match(acceptance, /new AxeBuilder/u);
  assert.match(acceptance, /forcedColors: "active"/u);
  assert.match(acceptance, /Shift\+Tab/u);
  assert.match(acceptance, /page\.keyboard\.press\("Escape"\)/u);
  assert.match(acceptance, /Review approval…/u);
  assert.match(premerge, /npm run test:browser:install/u);
  assert.match(premerge, /npm run test:browser/u);
});

test("draft release checks out the exact verifier and binds GitHub CLI", () => {
  const workflow = read(".github/workflows/release.yml");
  const draftStart = workflow.indexOf("  draft-release:");
  assert.ok(draftStart >= 0);

  const draftJob = workflow.slice(draftStart);
  // `!cancelled()` keeps a status function so intentionally skipped Desktop preview
  // jobs cannot skip this job before its gate check runs, while still refusing to
  // publish or mutate a draft release once the workflow is cancelled.
  assert.match(
    draftJob,
    /if: \$\{\{ !cancelled\(\) && needs\.validate\.outputs\.publish_draft == 'true' && needs\.gate\.result == 'success' \}\}/u,
  );
  assert.match(draftJob, /GH_REPO: \$\{\{ github\.repository \}\}/u);
  assert.match(draftJob, /gh release upload "\$RELEASE_TAG" dist\/\*/u);
  assert.match(draftJob, /gh release create "\$RELEASE_TAG" dist\/\*/u);
  assert.match(draftJob, /actions\/checkout@/u);
  assert.match(draftJob, /ref: \$\{\{ needs\.validate\.outputs\.tag \}\}/u);
  assert.match(draftJob, /persist-credentials: false/u);
});

test("release manifest writer emits exact final binary digests", () => {
  const root = realpathSync(
    mkdtempSync(join(tmpdir(), "colossus-desktop-contract-")),
  );
  try {
    const macos = join(root, "Colossus Desktop.app", "Contents", "MacOS");
    const resources = join(
      root,
      "Colossus Desktop.app",
      "Contents",
      "Resources",
    );
    mkdirSync(macos, { recursive: true, mode: 0o755 });
    mkdirSync(resources, { recursive: true, mode: 0o755 });
    const notices = join(resources, "ripgrep");
    mkdirSync(notices, { mode: 0o755 });
    for (const name of ["COPYING", "LICENSE-MIT", "UNLICENSE"]) {
      writeFileSync(join(notices, name), "test license\n", { mode: 0o644 });
    }
    const sidecar = join(macos, "colossus-sidecar");
    const cli = join(macos, "colossus");
    const ripgrep = join(macos, "rg");
    copyFileSync(process.execPath, sidecar);
    copyFileSync(process.execPath, cli);
    copyFileSync(process.execPath, ripgrep);
    chmodSync(sidecar, 0o755);
    chmodSync(cli, 0o755);
    chmodSync(ripgrep, 0o755);
    const output = join(resources, "colossus-bundle-manifest.json");
    execFileSync(process.execPath, [
      join(repository, "scripts/write-desktop-bundle-manifest.mjs"),
      "--target",
      "aarch64-apple-darwin",
      "--release-channel",
      "developer_preview",
      "--sidecar",
      sidecar,
      "--cli",
      cli,
      "--ripgrep",
      ripgrep,
      "--output",
      output,
    ]);
    const manifest = JSON.parse(readFileSync(output, "utf8"));
    assert.deepEqual(manifest, {
      schemaVersion: 3,
      targetTriple: "aarch64-apple-darwin",
      profile: "release",
      releaseChannel: "developer_preview",
      sidecar: { fileName: "colossus-sidecar", sha256: digest(sidecar) },
      cli: { fileName: "colossus", sha256: digest(cli) },
      ripgrep: { fileName: "rg", sha256: digest(ripgrep) },
    });
    if (process.platform !== "win32") {
      assert.equal(lstatSync(output).mode & 0o777, 0o644);
    }
    if (process.platform !== "win32") {
      execFileSync(process.execPath, [
        join(repository, "scripts/verify-desktop-bundle.mjs"),
        "--app",
        join(root, "Colossus Desktop.app"),
        "--target",
        "aarch64-apple-darwin",
        "--release-channel",
        "developer_preview",
      ]);
      const wrongChannel = spawnSync(process.execPath, [
        join(repository, "scripts/verify-desktop-bundle.mjs"),
        "--app",
        join(root, "Colossus Desktop.app"),
        "--target",
        "aarch64-apple-darwin",
        "--release-channel",
        "stable",
      ]);
      assert.notEqual(wrongChannel.status, 0);
      appendFileSync(ripgrep, "tampered");
      const tamperedRipgrep = spawnSync(process.execPath, [
        join(repository, "scripts/verify-desktop-bundle.mjs"),
        "--app",
        join(root, "Colossus Desktop.app"),
        "--target",
        "aarch64-apple-darwin",
        "--release-channel",
        "developer_preview",
      ]);
      assert.notEqual(tamperedRipgrep.status, 0);
      copyFileSync(process.execPath, ripgrep);
      appendFileSync(cli, "tampered");
      const tampered = spawnSync(process.execPath, [
        join(repository, "scripts/verify-desktop-bundle.mjs"),
        "--app",
        join(root, "Colossus Desktop.app"),
        "--target",
        "aarch64-apple-darwin",
        "--release-channel",
        "developer_preview",
      ]);
      assert.notEqual(tampered.status, 0);
    }

    if (process.platform !== "win32") {
      rmSync(sidecar);
      symlinkSync(cli, sidecar);
      const rejected = spawnSync(process.execPath, [
        join(repository, "scripts/write-desktop-bundle-manifest.mjs"),
        "--target",
        "aarch64-apple-darwin",
        "--release-channel",
        "developer_preview",
        "--sidecar",
        sidecar,
        "--cli",
        cli,
        "--ripgrep",
        ripgrep,
        "--output",
        output,
      ]);
      assert.notEqual(rejected.status, 0);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("release manifest writer uses final Windows executable names", () => {
  const root = realpathSync(
    mkdtempSync(join(tmpdir(), "colossus-windows-desktop-contract-")),
  );
  try {
    const sidecar = join(root, "colossus-sidecar-x86_64-pc-windows-msvc.exe");
    const cli = join(root, "colossus-x86_64-pc-windows-msvc.exe");
    const ripgrep = join(root, "rg-x86_64-pc-windows-msvc.exe");
    copyFileSync(process.execPath, sidecar);
    copyFileSync(process.execPath, cli);
    copyFileSync(process.execPath, ripgrep);
    chmodSync(sidecar, 0o755);
    chmodSync(cli, 0o755);
    chmodSync(ripgrep, 0o755);
    const output = join(root, "colossus-bundle-manifest.json");
    execFileSync(process.execPath, [
      join(repository, "scripts/write-desktop-bundle-manifest.mjs"),
      "--target",
      "x86_64-pc-windows-msvc",
      "--release-channel",
      "developer_preview",
      "--sidecar",
      sidecar,
      "--cli",
      cli,
      "--ripgrep",
      ripgrep,
      "--output",
      output,
    ]);
    assert.deepEqual(JSON.parse(readFileSync(output, "utf8")), {
      schemaVersion: 3,
      targetTriple: "x86_64-pc-windows-msvc",
      profile: "release",
      releaseChannel: "developer_preview",
      sidecar: {
        fileName: "colossus-sidecar.exe",
        sha256: digest(sidecar),
      },
      cli: { fileName: "colossus.exe", sha256: digest(cli) },
      ripgrep: { fileName: "rg.exe", sha256: digest(ripgrep) },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("instruction Markdown cannot authorize a native browser launch without OS consent", () => {
  const commands = read("apps/desktop/src-tauri/src/setup_package/commands.rs");
  const start = commands.indexOf("pub(crate) async fn open_setup_link(");
  const end = commands.indexOf(
    "pub(super) fn validated_instruction_url(",
    start,
  );
  assert.ok(start >= 0 && end > start);
  const command = commands.slice(start, end);
  assert.match(
    command,
    /let url = validated_instruction_url\(&request\.url, belongs_to_instructions\)\?;/u,
  );
  assert.match(command, /let destination = url\.as_str\(\)\.to_owned\(\);/u);
  assert.match(
    command,
    /let approved = tauri::async_runtime::spawn_blocking\(move \|\| \{\s*app\.dialog\(\)/u,
  );
  assert.match(command, /\{destination\}/u);
  assert.match(command, /\.buttons\(MessageDialogButtons::OkCancelCustom\(/u);
  assert.match(
    command,
    /\.blocking_show\(\)[\s\S]*?\.await[\s\S]*?\.map_err\([^;]*\)\?;\s*if !approved \{\s*return Ok\(\(\)\);\s*\}\s*colossus_native_browser::open_external\(&view, url\.as_str\(\)\)/u,
  );
  assert.equal(
    command.match(/colossus_native_browser::open_external/g)?.length,
    1,
  );
});
