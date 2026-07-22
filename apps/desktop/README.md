# Colossus Operations Studio

This Tauri 2 Operations Studio defaults to a supervised **Managed Local** Colossus
sidecar bundled with the signed app. Work is the primary surface, with coordinated-agent
flow, released artifacts, fleet workload, operational activity, a verified local TUI,
and native settings around it. The renderer reaches the public Rust SDK only through a
narrow native bridge: it never receives provider keys, API bearers, discovery paths, or
private runtime paths.

Advanced users can also save multiple authenticated **External** daemon targets. Both
modes use the same pinned loopback gRPC and SDK contracts; only the native lifecycle
owner differs.

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

## Managed Local quick start

Run these commands from the repository root.

1. Install renderer dependencies and launch the development app:

   ```bash
   cd apps/desktop
   npm ci --ignore-scripts
   cd ../..
   ./scripts/desktop-dev
   ```

2. Choose a workspace, configure a provider and model, and let Desktop start Managed
   Local. Use the offline self-test when you want to verify the sidecar without a key or
   network call.

   Settings reuses the stored key for same-provider model/profile changes. Select
   **Replace the stored API key** only when rotating it; first setup and provider
   changes always use native secure input. Development access also requires a fixed
   native authority confirmation.

No daemon enrollment or separate terminal is required for this path. The launcher
builds and stages the two native binaries before starting Tauri.

## External daemon development

Stop the daemon before one-time application enrollment:

```bash
./target/debug/colossus --config .colossus/config.yaml worker --shutdown
```

Enroll the Desktop application. `auto` derives a keyring account bound to the daemon's
instance identity and full TLS fingerprint:

```bash
./target/debug/colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:colossus-desktop \
  --scope runs:execute \
  --scope runs:read \
  --scope runs:control \
  --scope prompts:respond \
  --scope approvals:respond \
  --role primary \
  --credential-keyring-service com.obscuritylabs.colossus.desktop.external \
  --credential-keyring-account auto
```

Add one exact `--tool TOOL_NAME` for each tool the External target may invoke; omitting
all tool flags denies every tool. Approval authority remains a separate deliberate
grant: omit `--scope approvals:respond` when the target has no approval-gated tools or
another application handles approvals. The scope never expands the exact tool ceiling.

Copy the non-secret connection template and paste every printed identity and credential
destination value using camel-case field names:

```bash
cp apps/desktop/src-tauri/connection.json \
  apps/desktop/src-tauri/connection.local.json
```

`connection.local.json` is ignored by Git. It is validated and compiled into the native
binary; it is never loaded by or returned to the WebView. The service must be the fixed
Desktop external service, and the account must be the bound value printed by enrollment.

Do not copy these values from the mutable `endpoint.json` discovery file. If the
enrollment output was lost, stop the worker and deliberately rotate the desktop
credential with `--replace-credential` to obtain a fresh trusted enrollment record.

Start the public worker and leave it running:

```bash
./target/debug/colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api"
```

In a second terminal, launch the app and select the External target:

```bash
./scripts/desktop-dev
```

The first native read of the enrolled credential may trigger a macOS Keychain access
prompt. Changing the local trust configuration requires rebuilding/restarting the app.

### Migrate an older Desktop credential

Older builds stored the External bearer at
`com.obscuritylabs.colossus.desktop / colossus-public-api`. With the worker stopped,
enroll the identity-bound destination and retire that legacy entry in one explicit
operation:

```bash
./target/debug/colossus --config .colossus/config.yaml worker \
  --public-api-dir "$HOME/.colossus-public-api" \
  --enroll-application app:colossus-desktop \
  --scope runs:execute \
  --scope runs:read \
  --scope runs:control \
  --scope prompts:respond \
  --scope approvals:respond \
  --role primary \
  --credential-keyring-service com.obscuritylabs.colossus.desktop.external \
  --credential-keyring-account auto \
  --retire-credential-keyring-service com.obscuritylabs.colossus.desktop \
  --retire-credential-keyring-account colossus-public-api
```

Repeat any required `--tool` ceilings. Omit `--scope approvals:respond` only when
Desktop must not approve effects for that target. The CLI accepts the legacy source
only after it authenticates under this daemon's API root as the same application. It activates the
new credential before durably revoking the old one and deletes the legacy keyring entry
only after revocation is confirmed. Both retirement flags are required together and
cannot be combined with `--replace-credential`. A sanitized reconciliation error leaves
the new identity-bound credential active and reports only non-secret credential IDs.
During reconciliation, never delete the source selector's current value unless it is
confirmed to be the printed prior credential.

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

- External API bearers stay in the operating-system keyring and native Rust memory.
  Managed worker authentication is generated in native memory and delivered only to
  the verified bundled TUI through bounded inherited anonymous pipes before the
  renderer receives the PTY. Neither
  credential is accepted through files, environment variables, command-line values,
  URLs, or renderer IPC.
- The endpoint descriptor is mutable discovery metadata. The independently enrolled
  instance ID and TLS certificate fingerprint are compiled into the native app and
  checked before authenticated application calls.
- The `main` window has only narrow Colossus and lifecycle commands. The dedicated
  terminal WebView has a separate narrow PTY capability limited to the verified
  bundled TUI for a native-selected workspace. The child must independently open and
  attest that exact workspace before native worker authentication is released; the
  native DTO rejects general Shell requests. Neither renderer receives arbitrary
  process, filesystem, or HTTP authority.
- Managed Local persists a macOS device/inode/birthtime workspace identity. Legacy
  path-only or inode-only previews require explicit folder reselection and cannot reuse
  the prior managed state partition. Repository skill reads stay relative to the
  retained workspace descriptor; app-private skill roots receive their own no-follow
  capabilities.
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
