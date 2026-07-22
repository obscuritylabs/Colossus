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
});

test("provider enrollment and external trust stay behind native UI", () => {
  const types = read("apps/desktop/src/types.ts");
  const configureRequest = types.slice(
    types.indexOf("export interface ConfigureManagedRuntimeRequest"),
    types.indexOf("export type TerminalKind"),
  );
  assert.doesNotMatch(configureRequest, /apiKey|baseUrl/u);

  const onboarding = read("apps/desktop/src/components/OnboardingSurface.tsx");
  assert.doesNotMatch(onboarding, /type=["']password["']|API base URL/u);
  assert.match(onboarding, /native secure prompt/u);
  assert.match(onboarding, /WebView or renderer IPC/u);

  const enrollment = read("apps/desktop/src-tauri/src/provider_enrollment.rs");
  assert.match(enrollment, /Command::new\("\/usr\/bin\/osascript"\)/u);
  assert.match(enrollment, /\.env_clear\(\)/u);
  assert.match(enrollment, /with hidden answer/u);
  assert.match(enrollment, /api\.openai\.com/u);
  assert.match(enrollment, /openrouter\.ai/u);

  const commands = read("apps/desktop/src-tauri/src/desktop_commands.rs");
  assert.match(commands, /fn reusable_provider_credential/u);
  assert.match(commands, /!request\.replace_credential/u);
  assert.match(commands, /provider\.kind == request\.provider_kind/u);
  assert.match(commands, /verify_reused_provider_credential/u);
  assert.match(commands, /fn development_access_elevation/u);
  assert.match(commands, /confirm_development_access\(&app\)/u);
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
  assert.match(source, /resources must remain inside the canonical application bundle/u);
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

test("pre-merge desktop packaging declares its non-runnable trust channel", () => {
  const workflow = read(".github/workflows/premerge.yml");
  const desktopStart = workflow.indexOf("  macos-desktop:");
  const windowsStart = workflow.indexOf("  windows-runtime:", desktopStart);
  assert.ok(desktopStart >= 0 && windowsStart > desktopStart);
  const desktop = workflow.slice(desktopStart, windowsStart);
  assert.match(desktop, /COLOSSUS_DESKTOP_TEAM_ID: "ADHOC"/u);
  assert.match(
    desktop,
    /COLOSSUS_DESKTOP_RELEASE_CHANNEL: "validation_only"/u,
  );
  assert.match(desktop, /Build validation-only ADHOC macOS bundle structure/u);
});

test("release compilation and signing authority use separate runners", () => {
  const workflow = read(".github/workflows/release.yml");
  const buildStart = workflow.indexOf("  desktop_macos_build:");
  const signStart = workflow.indexOf("  desktop_macos:", buildStart);
  const gateStart = workflow.indexOf("  gate:", signStart);
  assert.ok(buildStart >= 0 && buildStart < signStart && signStart < gateStart);

  const buildJob = workflow.slice(buildStart, signStart);
  const signJob = workflow.slice(signStart, gateStart);
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
    /MACOS_TEAM_ID repository variable must be a 10-character Apple Team ID' >&2\n\s+exit 1\n\s+fi/u,
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
    /MACOS_TEAM_ID secret must be a 10-character Apple Team ID' >&2\n\s+exit 1\n\s+fi/u,
  );
  assert.doesNotMatch(signJob, /actions\/setup-node@/u);
  assert.doesNotMatch(signJob, /rust-toolchain@/u);
  assert.doesNotMatch(signJob, /\bnpm\s/u);
  assert.doesNotMatch(signJob, /\bcargo\s/u);
  assert.doesNotMatch(signJob, /\btauri\s/u);

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
    assert.equal(lstatSync(output).mode & 0o777, 0o644);
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
