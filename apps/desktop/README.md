# Colossus Operations Studio

This Tauri 2 desktop client is the first Operations Studio for an already-running,
enrolled Colossus worker. Work is the primary surface, with coordinated-agent flow,
released artifacts, fleet workload, operational activity, and native connection
settings around it. The app exercises the public Rust SDK through a native-only
bridge: create and resume runs, stream updates, cancel work, browse recent runs, and
answer interactions.

This first build intentionally does not start or enroll the worker. Keeping those
privileged operations outside the WebView preserves the worker's existing ownership
and enrollment boundaries while the embedded-agent distribution mode is designed.

## UI showcase

The complete interface can be reviewed without a daemon by using its deterministic,
development-only Operations Studio fixture:

```bash
cd apps/desktop
npm ci --ignore-scripts
npm run dev -- --host 127.0.0.1
```

Open <http://127.0.0.1:1420/?fixture=operations-studio>. The showcase supports the
primary navigation, new-work flow, artifact tabs, and approval responses. Vite removes
the fixture from production builds; the normal route always uses the native bridge.

## Quick start

Run these commands from the repository root.

1. Build the Colossus CLI if it is not already installed:

   ```bash
   cargo build -p colossus-cli
   ```

2. Stop any running worker before the one-time enrollment operation:

   ```bash
   ./target/debug/colossus --config .colossus/config.yaml worker --shutdown
   ```

   It is harmless if there was no worker to stop.

3. Enroll this desktop application. The following least-privilege grant supports chat,
   history, cancellation, and user prompts, but grants no tools and no approval power:

   ```bash
   ./target/debug/colossus --config .colossus/config.yaml worker \
     --public-api-dir "$HOME/.colossus-public-api" \
     --enroll-application app:colossus-desktop \
     --scope runs:execute \
     --scope runs:read \
     --scope runs:control \
     --scope prompts:respond \
     --role primary \
     --credential-keyring-service com.obscuritylabs.colossus.desktop \
     --credential-keyring-account colossus-public-api
   ```

   To test effect approvals, add `--scope approvals:respond` deliberately. That scope
   lets this app approve effects and should not be part of the default grant. Add one
   exact `--tool TOOL_NAME` for each tool the app is allowed to invoke; omitting all
   tool flags denies every tool.

4. Copy the non-secret native trust configuration and paste the values printed by
   enrollment. The CLI prints snake-case JSON: map `instance_id` to `instanceId` and
   `certificate_sha256` to `certificateSha256` in the desktop file:

   ```bash
   cp apps/desktop/src-tauri/connection.json \
     apps/desktop/src-tauri/connection.local.json
   ```

   `connection.local.json` is ignored by Git. It is validated and compiled into the
   native binary; it is never loaded by or returned to the WebView. Keep the configured
   keyring service/account identical to the enrollment command.

   Do not copy these values from the mutable `endpoint.json` discovery file. If the
   enrollment output was lost, stop the worker and deliberately rotate the desktop
   credential with `--replace-credential` to obtain a fresh trusted enrollment record.

5. Start the public worker and leave it running:

   ```bash
   ./target/debug/colossus --config .colossus/config.yaml worker \
     --public-api-dir "$HOME/.colossus-public-api"
   ```

6. In a second terminal, launch the app:

   ```bash
   ./scripts/desktop-dev
   ```

The first native read of the enrolled credential may trigger a macOS Keychain access
prompt. Changing the local trust configuration requires rebuilding/restarting the app.

## Development checks

```bash
cd apps/desktop
npm ci --ignore-scripts
npm run check
npm run build
npm run tauri:build
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo deny --manifest-path src-tauri/Cargo.toml --config ../../deny.toml --locked check -A duplicate -A license-not-encountered licenses sources bans
cargo audit --file src-tauri/Cargo.lock
```

## Security boundary

- The bearer stays in the operating-system keyring and native Rust memory. It is never
  accepted through files, environment variables, command-line flags, URLs, or IPC.
- The endpoint descriptor is mutable discovery metadata. The independently enrolled
  instance ID and TLS certificate fingerprint are compiled into the native app and
  checked before authenticated application calls.
- Only the `main` window has the eight narrow Colossus commands. No shell, filesystem,
  process, or generic HTTP capability is granted to the renderer.
- A restrictive production CSP permits bundled content and Tauri IPC only. Completed
  assistant responses use sanitized Markdown with raw HTML, remote images, and
  model-authored navigation disabled; streamed, user, tool, system, and interaction
  text remains plain React text.
- The loopback development server explicitly denies every `src-tauri` path, including
  the local trust configuration, even if renderer code attempts a same-origin fetch.
- The SDK/server still enforce exact scopes, role and tool ceilings, message limits,
  idempotency, and replay-safe sequence numbers. Treat the enrollment grant as the
  maximum damage a compromised renderer could request.

The generic keyring provider protects against file, log, argument, environment, and
other-user leakage, but it is not a portable same-user process sandbox. Packaged apps
that must defend against hostile processes running as the same OS user need a
platform-specific application-bound credential provider and signing/keychain policy.
The native endpoint implementation is currently Unix-only. This MVP is built and
release-gated on macOS; Linux packaging remains a follow-up. Tauri's target-specific
Linux GTK3 dependency graph currently emits RustSec maintenance and unsoundness
advisories even when the macOS target is selected, so it must be reviewed again before
claiming a supported Linux release. The audit gate still fails on vulnerability-class
advisories and reports informational advisories for review.
