---
title: Source setup and test tiers
description: Build Colossus from source and choose focused, fast, and full verification.
audience: developer
type: tutorial
---

# Source setup and test tiers

## Goal

Build the workspace with the supported Rust toolchain and establish a fast, trustworthy
test loop.

## Prerequisites

- Rust `1.96` with edition `2024` support.
- Git and the native build dependencies required by your platform.
- A source checkout at the repository root.
- Permission for test processes to bind local loopback sockets. Integration tests
  start temporary servers; sandbox restrictions otherwise fail with
  `Operation not permitted`.

The tracked development container is the supported ready-to-build Linux environment.
It uses digest-pinned official Debian Bookworm base images and a locked Rust feature,
selects Clang for native dependencies, pins the Rust, Node.js, Python, and Go versions
used by CI, includes the Tauri system libraries, installs the pinned `actionlint`,
`cargo-deny`, and `cargo-audit` tools used by the local PR gate, and provides an isolated
Docker daemon for documentation builds. In Codespaces or VS Code, rebuild the container
after changing `.devcontainer/`.

Rust API contract builds use the exact cross-platform `protoc-bin-vendored` workspace
dependency, so contributors and release runners do not need an ambient `protoc` binary.
Language SDK generation remains separate and uses its own pinned local generator
toolchain under `sdk/`.

## Optional mise bootstrap

The root `mise.toml` is an opt-in bootstrap for Node, Python, Go, actionlint,
cargo-deny and cargo-audit. It reads Rust from `rust-toolchain.toml`; it does not
install native system libraries, Docker, SDK-local generators or the fuzz nightly.
See the [toolchain inventory](toolchain-inventory.md) for declarations and check owners.
CI and devcontainer provisioning still use their existing pins.

With mise already installed, inspect the repository configuration before trusting it:

```bash
mise trust mise.toml
mise install
mise exec -- cargo xtask check workflows
```

Plain Cargo and script commands remain supported without mise. A shell-level
`RUSTUP_TOOLCHAIN` override can supersede the repository pin; check
`rustup show active-toolchain` when diagnosing a mismatch. Initial installation needs
network access. An offline setup must already contain the tools, package caches and
native dependencies; the offline runtime does not imply offline tool installation.
The repository does not yet pin the mise executable or supply backend/platform locks.

`AGENTS.md` is the canonical agent guide and `CLAUDE.md` is a relative symlink to it.
On Windows, enable symlink support before checkout (for example, Developer Mode and
Git's `core.symlinks=true`); a checkout that materializes a text file containing only
`AGENTS.md` is not an equivalent guide. Verify the link again in a new worktree.

## Steps

1. Confirm the toolchain and build the workspace:

    ```bash
    rustc --version
    cargo build --workspace
    ```

2. Run one focused crate test while iterating:

    ```bash
    cargo test -p colossus-policy --lib
    ```

    Add directly affected integration targets where appropriate:

    ```bash
    cargo test -p colossus-cli --test config_security
    ```

    Maintainers with a dedicated Splunk test endpoint can also run the ignored native
    Streamable HTTP smoke test:

    ```bash
    COLOSSUS_LIVE_SPLUNK_MCP_URL=https://splunk.example.test/services/mcp \
    SPLUNK_MCP_TOKEN=... \
      cargo test -p colossus-mcp --features live-splunk \
        live_splunk_streamable_http_discovery -- --ignored
    ```

    Credential-free public MCP acceptance uses Cloudflare's documentation server:

    ```bash
    cargo test -p colossus-cli --test mcp_remote_smoke -- --ignored --nocapture
    ```

    This verifies that a stateless server is rejected without its explicit opt-in,
    discovers the documentation tool, performs one read-only search, and verifies
    the resulting audit journal. It requires outbound HTTPS and runs on Windows,
    macOS, and Linux. Public service availability is not a deterministic CI gate.

    On Windows, also exercise Desktop's authenticated managed-sidecar diagnostic
    channel with platform-protected state:

    ```powershell
    cargo test -p colossus-sidecar --test windows_lifecycle cloudflare_docs -- --ignored --nocapture
    ```

    This needs Windows Credential Manager, local sockets, and public HTTPS. It uses
    temporary private workspace/home directories and cleans up its credential records.
    The native Desktop unit suite separately verifies that ordinary Windows paths
    select the same worker pipe as the sidecar's canonical paths, even before the
    state file exists. The deterministic `mcp_smoke` CLI suite covers local stdio
    discovery, calls, redaction, research, and audit on all three supported platforms;
    Windows runs it with the AppContainer/Job Object backend in the pre-merge lane.

3. Run the fast development tier. It checks the diff, formatting, crate roots, and all
   workspace library tests:

    ```bash
    cargo xtask dev
    ```

4. Run the complete Rust gate when the change is ready:

    ```bash
    cargo xtask check rust
    ```

5. Before opening or updating a pull request, run change-selected validation against
   the target branch:

    ```bash
    cargo xtask pr --base origin/main
    ```

`cargo xtask pr` always checks workflow contracts and then uses the repository's
fail-closed path classifier to select Rust, public SDK, Desktop, documentation, and
dependency-policy components. Component checks are also directly available as
`cargo xtask check rust`, `sdk`, `desktop`, `docs`, `dependencies`, `sidecar`, and
`workflows`. The task runner orchestrates repository-owned checks; hosted CI still owns
trusted-base decisions, runner provisioning, AppArmor installation, artifact upload,
and platform acceptance.

Every pull-request update receives selected Linux/documentation validation, while
reviewed final heads receive the representative macOS, Windows, and live-security tier
only when a writer applies `ci:full`. Complete x64/ARM64 coverage is reserved for release
tags. See [Tiered CI/CD](ci-cd.md).

For cold builds or work across multiple worktrees, opt into the local compilation cache:

```bash
./scripts/cargo-sccache check -p colossus-runtime
./scripts/cargo-sccache xtask dev
sccache --show-stats
```

Ordinary `cargo` remains supported when `sccache` is unavailable.

To run an isolated development TUI:

```bash
./scripts/colossus-dev --approval-mode full-access tui
```

The launcher creates development-only configuration, independent environment key
material, state, and secure anchor under `.colossus`. It compiles before loading keys
and then executes the binary directly.

To run Colossus Desktop with its debug Managed Local sidecar and bundled CLI:

```bash
./scripts/desktop-dev
```

The launcher installs the locked renderer dependencies, builds and stages both native
executables for the host target, and opens the Tauri development app. An External
daemon `connection.local.json` is optional. If the file exists, its instance identity
and certificate pin must be valid; remove it to test Managed Local only.

Debug Desktop uses a keyless plaintext journal in a separate
`development-plaintext/` Managed Local state partition, so local iteration does not
prompt for journal keys in the platform keychain. This mode retains the journal hash
chain but has no payload confidentiality, signed checkpoints, or external rollback
anchor. Release builds continue to use platform-protected journals and never reuse the
debug partition.

### Use isolated development credential custody

Unsigned development rebuilds can change their macOS code identity and require renewed
Keychain consent. The ordinary launchers keep the platform credential store. An explicit
debug-only alternative stores encrypted credentials under a private, isolated
`COLOSSUS_HOME`, with its wrapping key in that same private home. This provides less
protection from other applications running as your OS user than the platform store;
use it only for a deliberately selected development home outside every workspace.
Release binaries reject the development selector.

Build the debug CLI with the pinned toolchain first. Stop the selected app, worker and
connector before preparing custody; the offline commands refuse busy vaults and
journals. `dev-credentials init --home /absolute/private/home --workspace
/absolute/workspace` creates an **inactive** authority. `dev-credentials status` with
the same arguments inspects its public marker without reading a key. Neither command
migrates platform entries or activates an existing home.

Use `colossus dev-credentials plan --help` to select exact existing sources, then review
the generated owner-private plan and its SHA-256 before applying it with
`dev-credentials rewrap --plan-file FILE --expected-plan-sha256 SHA256 --apply`.
Planning reads only nonsecret metadata. Applying may require one final platform-store
consent to read the selected original entries, seals and verifies them, and activates
the authority only after all selected sources pass. Old platform entries remain intact;
credentials and grants are not reissued. A named connector enrollment can be copied
from a shared source home into a new isolated home without exporting that source
vault's master or sibling enrollments. Platform-backed redb journals preserve their
historical key IDs, checkpoint seed and anchor; environment/headless and PostgreSQL
journal source migration is not supported by this command and fails explicitly.

Vault inspection and apply retain the original database bytes. An unclean redb vault
can be inspected through a bounded encrypted snapshot recovered only in memory; the
original file is not repaired. Vault sources larger than 64 MiB or held by a writer
are refused. This does not recover a missing platform key or authorize a different
source selection.

For an empty home, an explicit `plan --fresh-empty` followed by the same reviewed
rewrap boundary activates fresh development custody. It refuses existing runtime or
credential state. Never use a fresh-home plan to recover missing keys.

After activation, run the canonical launcher with its explicit opt-in:

```sh
COLOSSUS_HOME=/absolute/private/home ./scripts/desktop-dev --dev-credentials
COLOSSUS_HOME=/absolute/private/home COLOSSUS_DEV_CONFIG=/absolute/config.yaml \
  ./scripts/colossus-dev --dev-credentials tui
```

On Windows, set the same explicit private `COLOSSUS_HOME` and run
`./scripts/desktop-dev.ps1 -DevCredentials`. These launchers require Node.js, inspect
only the authority marker, and pass one nonsecret path selector to trusted native
composition. They remove raw development/journal key variables from build processes;
tools cannot inherit the selector or raw wrapping material. The CLI opt-in uses the
reviewed existing configuration and does not regenerate development journal IDs.

An inactive, malformed, missing, linked, moved or overly permissive wrapping-key file
fails closed. There is no automatic platform fallback or key regeneration. Restore the
original private custody from a protected backup or stop and review a new offline plan;
do not delete a key to make startup succeed. See the
[credential boundary](security-architecture.md#development-credential-authority).

### Select a private Codex account for a worker

A serving CLI worker can select one existing owner-private Codex auth file without
changing the default account store or setting `CODEX_HOME`:

```sh
target/debug/colossus --workspace /absolute/workspace --config /absolute/config.yaml \
  --approval-mode ask worker --public-api-dir /absolute/private/api \
  --codex-auth-path /absolute/private/account/auth.json --no-model-network-tools
```

The file and its directory must already exist, be private and have no linked aliases;
keep the selected account file outside the tool workspace.
The path must be absolute and canonical; credential contents remain late-bound behind
the provider permit. The selected private directory stays bound for every load and
refresh; parent aliases and added hard links are refused, while legitimate atomic
credential replacement remains supported. Invalid selection fails before worker
acquisition without falling
back to another account. This option is unavailable with `worker --once`, status,
shutdown or enrollment administration; ordinary `codex login/status/logout` behavior
is unchanged. Configure the `open_ai_codex` primary route and exact service/refresh
origins using the [provider guide](../use/providers/codex-chatgpt.md).
The optional serve-only `--no-model-network-tools` host control hides generic model
fetch tools while retaining the configured provider's HTTP and refresh transport. It
does not change access policy, runtime grants, approval handling or provider credentials;
omitting it preserves the ordinary CLI tool surface.

The pruned release compilation path requires an explicit non-runnable validation
channel and sentinel:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_RELEASE_CHANNEL=validation_only \
COLOSSUS_DESKTOP_TEAM_ID=ADHOC \
  npm run tauri:build
```

A stable, sealed macOS application requires both the signing identity and the exact
10-character Apple Team ID embedded into the native runtime:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_SIGNING_IDENTITY='Developer ID Application: Example (TEAMID)' \
COLOSSUS_DESKTOP_TEAM_ID='TEAMID1234' \
COLOSSUS_DESKTOP_RELEASE_CHANNEL=stable \
  npm run tauri:bundle:macos
```

Set `COLOSSUS_DESKTOP_NOTARY_PROFILE` to a `notarytool` keychain profile to submit,
staple, assess, and archive the signed app without placing Apple credentials in argv or
environment variables.
When the profile lives outside the default search list, also set
`COLOSSUS_DESKTOP_NOTARY_KEYCHAIN` to its absolute keychain path.

For an explicitly labeled, runnable Developer Preview, use only the ad-hoc identity and
preview channel. This historical 0.10.1 preview command illustrates the contract; never
attach a notary profile to this build:

```bash
cd apps/desktop
COLOSSUS_DESKTOP_RELEASE_VERSION=0.10.1-preview.2 \
COLOSSUS_DESKTOP_RELEASE_CHANNEL=developer_preview \
COLOSSUS_DESKTOP_TEAM_ID=ADHOC \
COLOSSUS_DESKTOP_SIGNING_IDENTITY=- \
  npm run tauri:bundle:macos
```

The Developer Preview retains strict signature, fixed identifier, sealed-manifest, and
nested-binary hash verification, but ad-hoc signing does not establish Apple publisher
identity and the app is not notarized. The native release channel supplies the Developer
Preview banner, shown when **Show security warnings** is enabled in Desktop appearance
settings (off by default). The separate `validation_only` channel also requires
the `ADHOC` sentinel and identity `-`, but its runtime intentionally rejects Managed Local
startup. Stable packaging rejects both ad-hoc channels, and stable release publication
for the production Desktop track still requires Developer ID plus notarization.

Stable core `vX.Y.Z` tags publish signed Windows x64 Desktop and Windows CLI candidates
alongside the SDK. They need no Apple signing, notarization, or Tauri updater credential;
automatic Windows updates remain disabled until a separate updater key and feed exist.

A canonical `vX.Y.Z-preview.N` Developer Preview tag uses the ad-hoc macOS preview
channel and creates an unnotarized macOS archive plus signed Windows Desktop assets.
It reads no Apple signing secret. Manual dispatch of
a preview version remains validation-only, embeds the rejected `ADHOC` sentinel, labels
its artifacts `VALIDATION-ONLY-ADHOC`, and cannot create a runnable app or draft release.
Manual dispatch of a stable version instead validates the immutable SDK candidate and
requires all Desktop jobs to be skipped. See [Core release operations](releasing.md) for
registry bootstrap, stable tag, approval, and recovery steps.

## Expected result

The workspace builds, focused tests provide a short feedback loop, and the local
completion gates finish without formatting drift, warnings, or test failures. Hosted
pre-merge acceptance remains a separate final-PR requirement.

## Verification

Confirm that `git status --short` contains only intentional source, test, and
documentation changes. Run the smallest command a reviewer can use to reproduce the
behavior and include it in the handoff.

## Failure path

Use the first compiler, Clippy, or test failure as the diagnostic source. Do not bypass
the required toolchain, deny-warnings policy, dependency rules, or platform acceptance
tests. A fast tier is useful feedback but never substitutes for the completion gates.

## Next step

Read [Architecture overview](architecture.md) before moving code across crates.
