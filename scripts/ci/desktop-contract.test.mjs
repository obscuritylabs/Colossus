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
  realpathSync,
  rmSync,
  symlinkSync,
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

test("Tauri bundles only the two native-owned executables", () => {
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
  ]);
  assert.deepEqual(config.plugins.updater, {
    endpoints: [],
    pubkey: "",
  });
});

test("Windows Desktop is a per-user unsigned Developer Preview package", () => {
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
  assert.match(packaging, /developer_preview", "validation_only/u);
  assert.match(packaging, /COLOSSUS_DESKTOP_TEAM_ID -ne "UNSIGNED"/u);
  assert.match(packaging, /cargo xtask desktop prepare/u);
  assert.match(packaging, /--no-sign/u);
  assert.match(packaging, /\[IO\.Path\]::GetTempPath\(\)/u);
  assert.match(packaging, /ConvertTo-Json -Compress -Depth 4/u);
  assert.match(packaging, /\[IO\.File\]::WriteAllText\(/u);
  assert.equal(
    packaging.match(/"--config", \$TauriOverridePath/gu)?.length,
    2,
  );
  assert.doesNotMatch(packaging, /\$VersionOverride/u);
  assert.match(packaging, /write-desktop-bundle-manifest\.mjs/u);
  assert.match(packaging, /patch-desktop-manifest-binding\.mjs/u);
  const detach = packaging.indexOf("[IO.File]::Move");
  const binding = packaging.indexOf("patch-desktop-manifest-binding.mjs");
  assert.ok(detach >= 0 && detach < binding);
  assert.match(
    packaging,
    /\[IO\.File\]::Move\(\$Detached, \$Path, \$true\)/u,
  );
  assert.doesNotMatch(packaging, /\[IO\.File\]::Replace/u);
  assert.match(packaging, /Get-FileHash[\s\S]*detached executable/u);
  assert.match(packaging, /"--bundles", "nsis"/u);
  assert.match(packaging, /Get-FileHash/u);
  assert.doesNotMatch(packaging, /stable/u);
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
  assert.match(vite, /xtermFrozenPrototypeCompatibility\(\)/u);
  assert.match(vite, /exclude: \["@xterm\/xterm"\]/u);
  assert.match(xtermCompatibility, /Qn\|\|=Object\.create\(null\)/u);

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

test("terminal PTY authority is isolated from the main WebView", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  const terminal = json(
    "apps/desktop/src-tauri/capabilities/terminal-pty.json",
  );
  assert.deepEqual(main.windows, ["main"]);
  assert.deepEqual(terminal.windows, ["terminal"]);
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
  const dto = read("apps/desktop/src-tauri/src/dto.rs");
  assert.match(dto, /deny_unknown_fields/u);
});

test("main WebView exposes the advanced configuration and updater commands it calls", () => {
  const main = json("apps/desktop/src-tauri/capabilities/main-chat.json");
  for (const permission of [
    "allow-apply-managed-model-configuration",
    "allow-check-desktop-update",
    "allow-install-desktop-update",
  ]) {
    assert.ok(main.permissions.includes(permission), permission);
  }

  const bridge = read("apps/desktop/src-tauri/src/lib.rs");
  const api = read("apps/desktop/src/api.ts");
  for (const command of [
    "apply_managed_model_configuration",
    "check_desktop_update",
    "install_desktop_update",
  ]) {
    assert.match(bridge, new RegExp(`\\b${command}\\b`, "u"));
    assert.match(api, new RegExp(`"${command}"`, "u"));
  }
  assert.doesNotMatch(bridge, /\bopen_workspace_terminal\b/u);
  assert.doesNotMatch(api, /"open_workspace_terminal"/u);
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
  ];
  for (const capability of capabilities) {
    for (const permission of capability.permissions) {
      assert.match(permission, /^allow-[a-z0-9-]+$/u);
      const command = permission.slice("allow-".length).replaceAll("-", "_");
      assert.ok(
        commands.has(command),
        `${capability.identifier} permission ${permission} is missing ${command} from build.rs COMMANDS`,
      );
    }
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
    /settings\.selected_target_id\.as_deref\(\) != Some\(MANAGED_TARGET_ID\)/u,
  );
  assert.match(
    implementation,
    /settings\.access_profile != AccessProfileSetting::Development/u,
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

test("Windows private storage is created with a protected native DACL", () => {
  const settingsSource = read("apps/desktop/src-tauri/src/desktop_settings.rs");
  const settings = settingsSource.slice(
    0,
    settingsSource.indexOf("#[cfg(test)]"),
  );
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
  assert.doesNotMatch(configureRequest, /apiKey|baseUrl/u);
  const managedConfigurationRequest = types.slice(
    types.indexOf("export type CredentialAction"),
    types.indexOf("export type TerminalKind"),
  );
  assert.match(managedConfigurationRequest, /baseUrl/u);
  assert.match(managedConfigurationRequest, /credentialAction/u);
  assert.doesNotMatch(
    managedConfigurationRequest,
    /apiKey|credentialId|credentialValue|secret/u,
  );

  const onboarding = read("apps/desktop/src/components/OnboardingSurface.tsx");
  assert.doesNotMatch(onboarding, /type=["']password["']|API base URL/u);
  assert.match(onboarding, /native secure prompt/u);
  assert.match(onboarding, /WebView or renderer IPC/u);
  const modelEditor = read(
    "apps/desktop/src/components/ModelConfigurationEditor.tsx",
  );
  assert.match(modelEditor, /contextWindowTokens/u);
  assert.match(modelEditor, /maxOutputTokens/u);
  assert.match(modelEditor, /credentialAction/u);
  assert.doesNotMatch(
    modelEditor,
    /type=["']password["']|apiKey|credentialId/u,
  );

  const enrollment = read("apps/desktop/src-tauri/src/provider_enrollment.rs");
  const enrollmentImplementation = enrollment.slice(
    0,
    enrollment.indexOf("#[cfg(test)]"),
  );
  assert.match(
    enrollmentImplementation,
    /Command::new\("\/usr\/bin\/osascript"\)/u,
  );
  assert.match(enrollmentImplementation, /\.env_clear\(\)/u);
  assert.match(enrollmentImplementation, /with hidden answer/u);
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
  assert.match(commands, /fn development_access_elevation/u);
  assert.match(commands, /confirm_development_access\(&app\)/u);
  assert.match(commands, /fn confirm_provider_origins/u);
  assert.match(commands, /fn rollback_staged_provider_credentials/u);
  assert.match(commands, /fn reject_active_managed_runs/u);
  assert.match(commands, /request_provider_secret\(\)/u);
  assert.match(commands, /Enable Development/u);
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
  const manifest = source.indexOf("write-desktop-bundle-manifest.mjs");
  const binding = source.indexOf("patch-desktop-manifest-binding.mjs");
  const main = source.indexOf('sign_one "$main"');
  const app = source.indexOf('sign_one "$app"');
  assert.ok(sidecar >= 0 && sidecar < cli && cli < manifest);
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
  const desktopStart = workflow.indexOf("  macos-desktop:");
  const windowsStart = workflow.indexOf("  windows-runtime:", desktopStart);
  assert.ok(desktopStart >= 0 && windowsStart > desktopStart);
  const desktop = workflow.slice(desktopStart, windowsStart);
  assert.match(desktop, /COLOSSUS_DESKTOP_TEAM_ID: "ADHOC"/u);
  assert.match(desktop, /COLOSSUS_DESKTOP_RELEASE_CHANNEL: "validation_only"/u);
  assert.match(desktop, /Build validation-only ADHOC macOS bundle structure/u);

  const windowsEnd = workflow.indexOf("  fuzz:", windowsStart);
  assert.ok(windowsEnd > windowsStart);
  const windows = workflow.slice(windowsStart, windowsEnd);
  assert.match(windows, /runs-on: blacksmith-8vcpu-windows-2025/u);
  assert.match(windows, /COLOSSUS_DESKTOP_TEAM_ID: "UNSIGNED"/u);
  assert.match(windows, /cargo xtask desktop prepare --profile debug/u);
  assert.match(
    windows,
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
  assert.match(windows, /steps\.worker_acceptance\.outcome/u);
  assert.match(windows, /steps\.desktop_prepare\.outcome/u);
  assert.match(windows, /steps\.native_clippy\.outcome/u);
  assert.match(windows, /steps\.native_tests\.outcome/u);
  assert.match(windows, /if: steps\.desktop_prepare\.outcome == 'success'/u);
  assert.ok(
    windows.indexOf("npm run typecheck") < windows.indexOf("Install Rust 1.96"),
  );
});

test("release compilation and signing authority use separate runners", () => {
  const workflow = read(".github/workflows/release.yml");
  const buildStart = workflow.indexOf("  desktop_macos_build:");
  const signStart = workflow.indexOf("  desktop_macos:", buildStart);
  const windowsStart = workflow.indexOf(
    "  desktop_windows_preview:",
    signStart,
  );
  const gateStart = workflow.indexOf("  gate:", windowsStart);
  assert.ok(
    buildStart >= 0 &&
      buildStart < signStart &&
      signStart < windowsStart &&
      windowsStart < gateStart,
  );

  const buildJob = workflow.slice(buildStart, signStart);
  const signJob = workflow.slice(signStart, windowsStart);
  const windowsJob = workflow.slice(windowsStart, gateStart);
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
  assert.match(buildJob, /MACOS_TEAM_ID: \$\{\{ vars\.MACOS_TEAM_ID \}\}/u);
  assert.match(
    buildJob,
    /if: needs\.validate\.outputs\.release_channel == 'stable'/u,
  );
  assert.match(
    buildJob,
    /if: needs\.validate\.outputs\.release_channel != 'stable'/u,
  );
  assert.match(
    buildJob,
    /if ! \[\[ "\$MACOS_TEAM_ID" =~ \^\[A-Z0-9\]\{10\}\$ \]\]; then/u,
  );
  assert.match(
    buildJob,
    /MACOS_TEAM_ID repository variable must be a 10-character Apple Team ID' >&2\r?\n\s+exit 1\r?\n\s+fi/u,
  );

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
    "verify-tauri-updater-signature.mjs",
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
    /if: needs\.validate\.outputs\.release_channel == 'stable'/u,
  );
  assert.match(
    signJob,
    /if: needs\.validate\.outputs\.release_channel != 'stable'/u,
  );
  assert.match(signJob, /security import/u);
  assert.match(signJob, /grep -F "\(\$MACOS_TEAM_ID\)"/u);
  assert.match(
    signJob,
    /if ! \[\[ "\$MACOS_TEAM_ID" =~ \^\[A-Z0-9\]\{10\}\$ \]\]; then/u,
  );
  assert.match(
    signJob,
    /MACOS_TEAM_ID secret must be a 10-character Apple Team ID' >&2\r?\n\s+exit 1\r?\n\s+fi/u,
  );
  assert.match(signJob, /actions\/setup-node@[0-9a-f]{40}/u);
  assert.match(signJob, /npm ci --ignore-scripts/u);
  assert.doesNotMatch(signJob, /rust-toolchain@/u);
  assert.doesNotMatch(signJob, /\bcargo\s/u);
  assert.doesNotMatch(signJob, /\btauri\s/u);

  assert.match(
    windowsJob,
    /if: needs\.validate\.outputs\.release_channel != 'stable'/u,
  );
  assert.match(windowsJob, /runs-on: blacksmith-8vcpu-windows-2025/u);
  assert.match(windowsJob, /COLOSSUS_DESKTOP_TEAM_ID: UNSIGNED/u);
  assert.match(windowsJob, /package-desktop-windows\.ps1/u);
  assert.match(windowsJob, /Get-FileHash/u);
  assert.match(windowsJob, /codeSigning = "unsigned_developer_preview"/u);
  assert.match(windowsJob, /smartScreenWarningExpected = \$true/u);
  assert.match(windowsJob, /Start-Process -FilePath \$installer/u);
  assert.match(windowsJob, /Start-Process -FilePath \$uninstallers/u);
  assert.match(windowsJob, /Colossus processes remained after uninstall/u);

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
  const credentialImport = signJob.indexOf(
    "Import Developer ID and notarization credentials",
  );
  assert.ok(
    hashCapture >= 0 &&
      hashCapture < archiveCheck &&
      archiveCheck < extraction &&
      extraction < hashComparison &&
      hashComparison < extractedCheck &&
      extraction < extractedCheck &&
      extractedCheck < credentialImport,
  );

  assert.match(
    workflow,
    /desktop_macos_build=\$\{\{ needs\.desktop_macos_build\.result \}\}/u,
  );
  assert.match(workflow, /desktop_windows_preview="\$WINDOWS_DESKTOP_RESULT"/u);
  assert.match(
    workflow,
    /if \[ "\$RELEASE_CHANNEL" = stable \]; then\s+test "\$WINDOWS_DESKTOP_RESULT" = skipped/u,
  );
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

test("stable desktop updates are signature-bound and unsigned previews have no update authority", () => {
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
  assert.match(build, /let updates_enabled = release_channel == "stable";/u);
  assert.match(
    build,
    /unsigned Developer Preview and validation-only Desktop builds must not advertise/u,
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
    /unsigned Windows packaging unexpectedly created an updater signature/u,
  );
  assert.match(release, /DESKTOP_UPDATE_PUBLIC_KEY/u);
  assert.match(release, /DESKTOP_UPDATE_PRIVATE_KEY/u);
  assert.match(
    release,
    /release_channel == 'stable' && secrets\.DESKTOP_UPDATE_PRIVATE_KEY/u,
  );
  assert.match(release, /write-desktop-update-manifest\.mjs/u);
  assert.match(release, /verify-tauri-updater-signature\.mjs/u);
  assert.match(channels, /types: \[published\]/u);
  assert.match(channels, /github\.event\.release\.prerelease == false/u);
  assert.doesNotMatch(channels, /developer_preview/u);
  assert.match(channels, /desktop-update-channels/u);
  assert.match(channels, /gh release upload "\$channel_tag"/u);
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
  assert.match(acceptance, /Allow once/u);
  assert.match(premerge, /npm run test:browser:install/u);
  assert.match(premerge, /npm run test:browser/u);
});

test("draft release binds GitHub CLI without checking out tagged sources", () => {
  const workflow = read(".github/workflows/release.yml");
  const draftStart = workflow.indexOf("  draft-release:");
  assert.ok(draftStart >= 0);

  const draftJob = workflow.slice(draftStart);
  assert.match(draftJob, /GH_REPO: \$\{\{ github\.repository \}\}/u);
  assert.match(draftJob, /gh release upload "\$RELEASE_TAG" dist\/\*/u);
  assert.match(draftJob, /gh release create "\$RELEASE_TAG" dist\/\*/u);
  assert.doesNotMatch(draftJob, /actions\/checkout@/u);
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
    const sidecar = join(macos, "colossus-sidecar");
    const cli = join(macos, "colossus");
    copyFileSync(process.execPath, sidecar);
    copyFileSync(process.execPath, cli);
    chmodSync(sidecar, 0o755);
    chmodSync(cli, 0o755);
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
      "--output",
      output,
    ]);
    const manifest = JSON.parse(readFileSync(output, "utf8"));
    assert.deepEqual(manifest, {
      schemaVersion: 2,
      targetTriple: "aarch64-apple-darwin",
      profile: "release",
      releaseChannel: "developer_preview",
      sidecar: { fileName: "colossus-sidecar", sha256: digest(sidecar) },
      cli: { fileName: "colossus", sha256: digest(cli) },
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
      "--output",
      output,
    ]);
    assert.notEqual(rejected.status, 0);
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
    copyFileSync(process.execPath, sidecar);
    copyFileSync(process.execPath, cli);
    chmodSync(sidecar, 0o755);
    chmodSync(cli, 0o755);
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
      "--output",
      output,
    ]);
    assert.deepEqual(JSON.parse(readFileSync(output, "utf8")), {
      schemaVersion: 2,
      targetTriple: "x86_64-pc-windows-msvc",
      profile: "release",
      releaseChannel: "developer_preview",
      sidecar: {
        fileName: "colossus-sidecar.exe",
        sha256: digest(sidecar),
      },
      cli: { fileName: "colossus.exe", sha256: digest(cli) },
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
