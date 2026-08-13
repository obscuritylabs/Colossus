# Colossus Operations Studio

This Tauri 2 Operations Studio defaults to a supervised **Managed Local** Colossus
sidecar bundled with the signed app. Work is the primary surface, with coordinated-agent
flow, released artifacts, fleet workload, operational activity, a verified local TUI,
and native settings around it. The renderer reaches the public Rust SDK only through a
narrow native bridge: it never receives provider keys, API bearers, discovery paths, or
private runtime paths.

Native settings live in `$COLOSSUS_HOME/desktop/`, and each selected repository gets a
separate `workspaces/<partition-id>/desktop/` Managed Local partition. Desktop never
reuses the matching CLI/TUI partition or writes managed state into the repository.
Earlier application-support data is preserved but ignored; there is no silent import
or deletion. See the
[Colossus home contract](../../docs/reference/colossus-home.md).

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
Use <http://127.0.0.1:1420/?fixture=interaction-question> to review the compact
`user.ask` question and response state.

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
   Local. OpenAI Responses and compatible providers enroll a native key; the Codex
   provider uses **Sign in with ChatGPT** through the installed official Codex CLI. Use
   the offline self-test when you want to verify the sidecar without a credential or
   network call.

   Fresh settings select **Allow all** access and the intentionally unsafe **Full
   access** execution boundary. Access and execution boundary are independent controls:
   choose **Development** or **Minimal** to narrow tool decisions, and choose
   **Workspace isolated** or **Offline isolated** to restore platform containment.
   Legacy settings migrations retain their prior isolated posture. Native confirmation
   is required for the first non-Minimal access selection, later access elevation, and
   execution-boundary elevation; the Full access warning remains visible while that
   Managed Local runtime is active.

   Settings reuses the stored key for same-provider model/profile changes. Select
   **Replace the stored API key** only when rotating it; first setup and provider
   changes always use native secure input.

   Codex auth tokens never enter the renderer or saved Desktop settings. Native code
   validates the Codex-owned credential file and supplies only its private path through
   inherited sidecar bootstrap IPC. Advanced configuration can set reasoning effort per
   model.

   Every top-level run snapshots the owner-private home `AGENTS.md` and the selected
   repository's `AGENTS.md` before explicit run and immutable mode instructions. Goal
   iterations and delegated subagents keep that exact snapshot for the run.

No daemon enrollment or separate terminal is required for this path. The launcher
builds and stages the two native binaries before starting Tauri.

### Plan Mode in Desktop

The Work composer’s **Plan** mode sends the typed public `RunMode::Plan` request. Each
completed turn must write exactly one new durable Draft and cannot implement the plan or
perform external mutation. Desktop preserves the canonical Plan ID, revision, and
status returned by the runtime and renders them with the owning session.

An actionable Draft renders **Revise in chat**, **Run once**, and **Run as Goal**.
Revision starts another public Plan Mode run bound to the source run and exact visible
revision. Execution starts a public Execute run that approves and consumes that exact
revision directly or into a bounded Goal. Every action remains durable, watchable,
policy- and audit-bound. The runtime advertises `plans.continue`; the SDK refuses these
typed actions when an older or lower-privilege target does not advertise it.

For Managed Local, **Advanced workflow** still launches the authenticated embedded TUI,
resumes the exact session, and selects the Plan using native-validated identifiers. It
is the complete lifecycle surface for inspection, approval, discard, and Goal resume.
The main WebView can request this bounded handoff but cannot write arbitrary PTY input.

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
  terminal WebView has a separate narrow PTY capability with two fixed kinds. The
  verified bundled TUI independently attests the native-selected workspace before
  worker authentication is released. The macOS shell kind launches only the validated
  system `/bin/zsh -l` with a native-selected workspace and cleared environment; it
  receives no Colossus credential and sits outside policy and audit. A fixed native
  warning is required before first enablement. The renderer cannot select a process,
  arguments, environment, working directory, filesystem API, or HTTP authority.
- A completed public Plan Mode run exposes canonical Plan identity, revision, and
  status. Main-window Plan actions reference a caller-owned source run and exact
  revision, not a renderer-supplied Plan ID; native and server validation bind them to
  the source session before a normal public run begins. A Managed Local TUI handoff may
  additionally prefill fixed `/session resume` and `/plan use` selections after both
  identifiers pass native bounds and control-character checks. It cannot inject a
  general command.
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
