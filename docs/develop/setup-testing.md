---
title: Source setup and test tiers
description: Set up the source toolchain, iterate with focused checks, and validate once at handoff.
audience: developer
type: tutorial
---

# Source setup and test tiers

## Development loop

Use the changed package and relevant test targets while editing:

```bash
cargo xtask quick check -p colossus-policy
cargo xtask quick test -p colossus-policy -- --lib
cargo xtask quick test -p colossus-cli -- --test config_security
cargo xtask quick clippy -p colossus-policy -- --lib -- -D warnings
```

`quick` adds `--locked` and requires package selection; repeat `-p` for several
packages. Arguments after `--` go to Cargo, including features, target selectors and
test filters. A second `--` passes arguments to the test harness. Select `--lib` or
`--test <target>` before filtering: a filter alone can still compile every integration
target. `check` skips final code generation. Use `cargo xtask` to list commands.

Use `cargo xtask dev` when broader workspace library coverage is useful. It checks
formatting, crate roots, the diff and all workspace library tests. Use
`cargo build --workspace` when all workspace members need building. Run the final
[handoff validation](#verification) after the work is ready.

## Prerequisites

- Rust `1.96`, edition `2024`, rustfmt and Clippy, as pinned in `rust-toolchain.toml`.
- Git and the platform's native build dependencies. Windows shell tasks require Git Bash.
- The Node, Python, Go and check tools needed for the changed surface; see the
  [toolchain inventory](toolchain-inventory.md).
- Local loopback sockets for integration tests and Docker for documentation builds.

The tracked devcontainer provides the supported Linux environment, native dependencies,
pinned tools and isolated Docker daemon. Rebuild it after changing `.devcontainer/`.
Rust builds use vendored `protoc`; SDK generators have their own pinned toolchain.
Check `rustup show active-toolchain` if an environment override supersedes the pin.
Offline setup requires existing tools, package caches, native libraries and images.

`AGENTS.md` is the canonical guide; `CLAUDE.md` is a relative symlink to it. On Windows,
enable symlink support before checkout and verify the link in each new worktree.

Helm chart changes additionally require Helm 3.19.0 (or an explicit `HELM` executable)
and `cargo xtask check helm`. This gate lints, renders and packages without a cluster.
The [chart README](https://github.com/obscuritylabs/Colossus/tree/main/deploy/control-plane/chart)
owns supported deployment options.

## Optional mise bootstrap

With mise installed, inspect the configuration before trusting it:

```bash
mise trust mise.toml
mise install
```

This opt-in setup supplies Node, Python, Go, actionlint, cargo-deny and cargo-audit;
it reads Rust from `rust-toolchain.toml`. Native libraries, Docker, SDK generators and
fuzz nightly remain separate. Plain Cargo is supported. Mise itself and its backend
locks are not repository-pinned; see the [inventory](toolchain-inventory.md).

## Measure the development loop

`xtask` prints each subprocess's duration and a summary of the slowest commands to
stderr, including failed checks. For compilation detail:

```bash
cargo xtask quick check -p colossus-runtime -- --timings
```

Cargo writes HTML under `target/cargo-timings`, or the selected target directory.
Compare the same targets, features and profile without competing builds: first build,
warm repeat, then rebuild after a representative edit. A warm repeat alone does not
measure editing speed. See [Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html).

## Optional local compiler cache

Keep ordinary Cargo's incremental defaults and each worktree's normal build directory.
To opt into an installed `sccache`:

```bash
./scripts/cargo-sccache xtask quick test -p colossus-policy -- --lib
sccache --show-stats
```

The wrapper only selects `RUSTC_WRAPPER` and preserves inherited settings. With
incremental environment overrides unset, Cargo's default incremental builds coexist
with `sccache`: incremental invocations fall back to rustc, while eligible registry
dependencies can be cached. The current sccache rejects explicit `CARGO_INCREMENTAL=1`
or `CARGO_BUILD_INCREMENTAL=1`; use ordinary Cargo when keeping either override.
Metadata-only checks and executable linking also have cache limitations. See
[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental)
and [sccache Rust support](https://github.com/mozilla/sccache/blob/main/docs/Rust.md).

For a measured non-incremental comparison, set `CARGO_INCREMENTAL=0` for that build
only. Compare edit/rebuild times and cache statistics before changing local defaults.
Worktrees do not require a shared target directory or disabling incremental builds.

## Run local applications

```bash
./scripts/colossus-dev --approval-mode full-access tui
./scripts/desktop-dev
```

The TUI launcher builds before loading keys and uses development state under `.colossus`.
Desktop installs locked renderer dependencies, prepares the CLI/sidecar and starts Tauri.
On Windows, use `powershell -File .\scripts\desktop-dev.ps1`. Managed Local needs no
separate daemon. An optional `connection.local.json` must have a valid instance identity
and certificate pin; remove it to test Managed Local alone.

Debug Desktop uses the separate `development-plaintext/` journal partition. It retains
the hash chain but provides no payload confidentiality, signed checkpoints or external
rollback anchor. Release builds use platform-protected journals and a different partition.

### Run the embedded Chromium preview on macOS

```bash
./scripts/desktop-dev --embedded-chromium-preview
```

Requires macOS, Xcode command-line tools, CMake, Python 3.11+, Node and pinned Rust.
The launcher verifies CEF, builds sandboxed helpers and stages an ad-hoc development
app under `.local/`. Keep its checkout and prepared binaries available. A flat
`cargo run` or `tauri dev` cannot supply the required bundle layout. See the
[native launcher guide](https://github.com/obscuritylabs/Colossus/blob/main/native/browser/README.md#launch-the-macos-desktop-development-preview)
for staging/logs and [native acceptance](testing.md#desktop-chromium-development-acceptance-on-macos)
for the interactive test command. Preview rendering does not establish production
browser containment or certificate custody; [ADR 0008](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work)
owns readiness.

### Use isolated development credential custody

For unsigned rebuilds that repeatedly prompt Keychain, an explicit debug-only authority
can keep encrypted credentials in a private `COLOSSUS_HOME` outside every workspace.
Its wrapping key is in that same home, providing less same-user isolation than the
platform store. Release builds reject it. Read the
[credential boundary](security-architecture.md#development-credential-authority) first.

Build the debug CLI and stop the selected app, worker and connector. Run
`colossus dev-credentials init --home /absolute/private/home --workspace /absolute/workspace`
to create an inactive authority; `status` with the same paths checks its marker.
Use `colossus dev-credentials plan --help` to select exact sources. Review the generated
private plan and SHA-256 before applying:

```bash
colossus dev-credentials rewrap --plan-file FILE --expected-plan-sha256 SHA256 --apply
COLOSSUS_HOME=/absolute/private/home ./scripts/desktop-dev --dev-credentials
COLOSSUS_HOME=/absolute/private/home COLOSSUS_DEV_CONFIG=/absolute/config.yaml \
  ./scripts/colossus-dev --dev-credentials tui
```

Windows uses the same private home with `./scripts/desktop-dev.ps1 -DevCredentials`.
The launchers require Node and an activated authority. Planning reads metadata; apply
may request platform consent, preserves original entries and grants, and activates only
after verification. Named enrollment copy does not export sibling records. Platform-backed
redb journals retain historical key IDs, checkpoint seed and anchor; environment/headless
and PostgreSQL journal migration are unsupported. Security architecture documents the
read-only source handling, 64 MiB recovery limit and busy/unsafe-source rejection.

Only an empty home may use `plan --fresh-empty` followed by reviewed apply. Never use it
to recover missing keys. There is no platform fallback or key regeneration; restore a
protected backup or review a new plan instead of deleting a key to make startup succeed.

### Select a private Codex account for a worker

```bash
target/debug/colossus --workspace /absolute/workspace --config /absolute/config.yaml \
  --approval-mode ask worker --public-api-dir /absolute/private/api \
  --codex-auth-path /absolute/private/account/auth.json --no-model-network-tools
```

Use an existing owner-private file and directory outside the tool workspace, with an
absolute canonical path and no linked aliases. The directory stays bound across loads
and atomic refresh; parent aliases and hard links are rejected. Invalid selection fails
before worker acquisition without fallback. This serve-only option does not change
`CODEX_HOME` or ordinary login/logout and is unavailable for `--once`, status, shutdown
or enrollment administration. `--no-model-network-tools` hides generic fetch tools while
retaining provider HTTP/refresh transport and existing policy/grants. Configure the route
and exact origins using the [provider guide](../use/providers/codex-chatgpt.md).

## Optional live MCP checks

These use external services, so they supplement deterministic tests. The Splunk check
requires a dedicated endpoint and token:

```bash
COLOSSUS_LIVE_SPLUNK_MCP_URL=https://splunk.example.test/services/mcp \
SPLUNK_MCP_TOKEN=... cargo test -p colossus-mcp --features live-splunk \
  live_splunk_streamable_http_discovery -- --ignored
cargo test -p colossus-cli --test mcp_remote_smoke -- --ignored --nocapture
```

The second command uses Cloudflare's public documentation server without credentials,
checks stateless opt-in, discovery, one read-only search and audit. On Windows, use
`cargo test -p colossus-sidecar --test windows_lifecycle cloudflare_docs -- --ignored --nocapture`
for the authenticated sidecar diagnostic; it requires Windows Credential Manager,
loopback and public HTTPS and owns temporary private state/credential records.
Deterministic `mcp_smoke` covers local stdio on all platforms, with Windows isolation
in the pre-merge lane.

## macOS release packaging

Run from `apps/desktop` with the variables for the selected channel:

| Channel (`COLOSSUS_DESKTOP_RELEASE_CHANNEL`) | `COLOSSUS_DESKTOP_TEAM_ID` | Command and signing identity |
| --- | --- | --- |
| `validation_only` | `ADHOC` | `npm run tauri:build`; Managed Local refuses to run. Packaging requires identity `-`. |
| `developer_preview` | `ADHOC` | `npm run tauri:bundle:macos` with `COLOSSUS_DESKTOP_SIGNING_IDENTITY=-`. |
| `stable` | Exact 10-character Apple Team ID | `npm run tauri:bundle:macos` with the matching Developer ID Application signing identity. |

Set `COLOSSUS_DESKTOP_RELEASE_VERSION` to the coordinated preview version when needed.
Stable distribution also requires notarization: set `COLOSSUS_DESKTOP_NOTARY_PROFILE`
to a notarytool keychain profile and, for a nondefault keychain,
`COLOSSUS_DESKTOP_NOTARY_KEYCHAIN`. Preview/validation packaging must not use a notary
profile. Preview signatures do not establish Apple publisher identity. See
[sealed bundle requirements](application-sdk.md#sealed-macos-bundle-manifest),
[release operations](releasing.md#developer-previews-and-desktop) and
[release channels](ci-cd.md#developer-preview-channel) for tag, signing and publication gates.

## Clear local build caches

After builds/tests stop, use `cargo clean` for the root workspace or
`cargo clean --manifest-path apps/desktop/src-tauri/Cargo.toml` for Desktop. Match any
`--target-dir`/`CARGO_TARGET_DIR` used to build. Cleaning discards incremental state;
reserve it for disk recovery or stale-output diagnosis and preserve needed installers
and evidence elsewhere first.

## Verification

At handoff, inspect `git diff --check` and `git status --short`, then run one final gate:

```bash
cargo xtask pr --base origin/main
```

This selects checks from the diff and includes the complete Rust gate when Rust is
selected. Do not also run `check rust` on unchanged source; use that standalone gate
for Rust-only completion without a PR. Workflow contracts always run. Selected native
computer checks cover portable behavior; [Windows acceptance](testing.md#windows-computer-control-acceptance)
remains separate. Fix failures, then rerun affected checks before the final gate.
Missing tools are setup failures, not permission to skip a selected check.

Include focused reproduction commands and final results in the handoff. Hosted
platform/release acceptance remains separate; see [CI/CD](ci-cd.md) and
[Contributing](contributing.md). Read [Architecture](architecture.md) before moving code
across crates.
