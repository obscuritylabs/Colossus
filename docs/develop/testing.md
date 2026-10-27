---
title: Test strategy and crate audit
description: Choose the right Rust test boundary and understand the retained test ownership for every Colossus crate.
audience: developer
type: reference
---

# Test strategy and crate audit

Put each supported-behavior assertion at the lowest boundary that can prove it.

## Test levels

| Level | Use it for | Location |
| --- | --- | --- |
| Unit | Pure rules, parsing, state transitions, bounded rendering, and adapter-local failure | Next to the module or in `src/tests.rs` / `src/tests/` |
| Integration | Public crate boundaries, persistence conformance, subprocess protocols, platform isolation, and live adapters | `<crate>/tests/` |
| End to end | Installed CLI, worker, Desktop, SDK, release, and operator workflows | CLI/Desktop suites and CI workflows |

Use process tests when the process boundary matters. Preserve security, protocol,
persistence and packaging boundary tests; mocks do not replace them. An ignored live
test needs explicit prerequisites and a named CI job or documented operator command.

## Crate-by-crate audit

These suites protect current boundaries; apply the [removal criteria](#removal-criteria)
when those boundaries change.

| Crate | Test ownership and disposition |
| --- | --- |
| `colossus-access` | Keep profile resolution, action precedence, and tool-ceiling unit tests. |
| `colossus-agent` | Keep turn-loop, tool, cancellation, and observability tests; the separate observability target proves exported signals. |
| `colossus-api-proto` | Keep generated-contract and compatibility tests for the public protocol. |
| `colossus-api-runtime` | Keep service authorization, streaming, enrollment, and repository integration tests. |
| `colossus-api` | Keep server composition and public API lifecycle tests. |
| `colossus-audit` | Keep journal export, retry, recovery, and live WORM acceptance tests. |
| `colossus-cli` | Keep command-level smoke suites because they exercise public parsing and embedded/worker boundaries; remove a suite only when its command or contract is removed. |
| `colossus-cloud`, `colossus-cloud-protocol`, `colossus-cloud-server`, `colossus-connector` | Keep project/personal ownership, placement, receipt, cursor, renewal and resource-admission invariants and the signed OIDC-to-runtime transport acceptance; run the operator-owned PostgreSQL variants described in [cloud development](cloud-control-plane.md). |
| `colossus-cloud-chat` | Keep credential purpose/project/user/generation/destination binding, personal/shared separation, controlled runtime composition and private-state identity tests; server acceptance owns browser authentication and supervised connector integration. |
| `colossus-control-plane-client` | Keep origin and route confinement, cookie custody, permission/expiry distinctions, logout/server switching, bounded reads, redirect rejection and single-attempt mutation tests. Server transport acceptance verifies the shared native client against the authenticated HTTP edge. |
| `colossus-codex-auth` | Keep OAuth/device-flow parsing, storage, and redaction tests. |
| `colossus-context` | Keep compaction budgets, snapshots, and deterministic fallback unit tests. |
| `colossus-contracts` | Keep serialization, validation, and stable contract-shape tests. |
| `colossus-credentials` | Keep encrypted persistence, process restart, initialization interruption, lease, scope/key/tamper, path, size, and sanitized-error conformance tests; run real OS-store acceptance on Windows, macOS, and Linux. |
| `colossus-darwin-process` | Keep platform process-isolation and limit tests on macOS CI. |
| `colossus-domain` | Keep dependency-free domain invariant tests. |
| `colossus-fuzzing` | Keep corpus regressions and fuzz harness compilation; they cover hostile parsers. |
| `colossus-grpc` | Keep transport translation, authentication, and stream-boundary tests. |
| `colossus-home` | Keep confinement, workspace identity, permissions, and symlink-escape tests. |
| `colossus-integrations` | Keep manifest, credential, dispatch, and live Splunk MCP acceptance tests. |
| `colossus-journal-postgres` | Keep shared journal conformance, transaction ownership, outage, and recovery tests. |
| `colossus-journal-redb` | Keep shared journal conformance, encryption, tamper, migration, and crash-recovery tests; retained on-disk readers protect current state. |
| `colossus-linux-native` | Keep bounded file-handle capture and strict NFS volume-scope parser tests; run native capture checks on Linux CI. |
| `colossus-mcp` | Keep strict configuration, protocol, OAuth, tool ceiling, and subprocess/remote tests. |
| `colossus-memory` | Keep canonical lifecycle, scope, Tantivy projection, and fallback tests. |
| `colossus-network` | Keep DNS pinning, redirects, trust roots, response bounds, and authority tests. |
| `colossus-observability` | Keep disabled-by-default, redaction, payload-mode, and exporter tests. |
| `colossus-plugins` | Keep upstream schema/frontmatter, component isolation, OCI archive, registry auth/origin, Sigstore trust, lifecycle lease, MCP overlay, and confinement tests. |
| `colossus-bundles` | Keep retained release-bundle signature, inventory, no-clobber, and installation tests. |
| `colossus-policy` | Keep built-in and OPA decision tests, including opt-in live and mTLS targets. |
| `colossus-ports` | Keep reusable port-conformance helpers; avoid adapter behavior here. |
| `colossus-presentation` | Keep pure document/theme/rendering tests. |
| `colossus-projection` | Keep deterministic projection, checkpoint, and rebuild tests. |
| `colossus-provider` | Keep provider translation, streaming, limits, malformed output, and redaction tests. |
| `colossus-research` | Keep evidence bounds, citations, lane failures, and deterministic fallback tests. |
| `colossus-runtime` | Keep composition and cross-service security tests. |
| `colossus-sandbox` | Keep native/OCI/Windows contract, broker, cleanup, and hostile-input tests. |
| `colossus-sdk` | Keep embedded/native-sidecar/gRPC parity and subprocess lifecycle tests. |
| `colossus-app-host` | Keep bounded private framing checks; VS Code's `test:managed` exercises the bundled native lifecycle, authenticated turns, provider credentials, restart, writer conflicts, and recovery. |
| `colossus-search` | Keep SearXNG/SerpAPI parsing, credentials, bounds, and role-routing tests. |
| `colossus-session` | Keep message, branch, restore, and context-view repository tests. |
| `colossus-sidecar-protocol` | Keep authenticated framing and workspace-identity compatibility tests; they protect deployed sidecars. |
| `colossus-sidecar` | Keep native and Windows bootstrap/lifecycle acceptance targets. |
| `colossus-telemetry` | Keep durable event, query, retention, and bounded-export tests. |
| `colossus-testkit` | Keep shared conformance tests and fixtures used by adapter crates. |
| `colossus-tools` | Keep schema-first validation, gateway adapters, confinement, output, and mutation tests. |
| `colossus-tui` | Keep reducer, layout, input, theme, restoration, and PTY history tests; they cover behavior not proved by snapshots alone. |
| `colossus-terminal` | Keep real PTY ownership, input deduplication, controller fencing, output-gap, slow-reader headroom, and native process cleanup tests. Control Plane acceptance proves the authenticated cross-replica browser-to-Fleet path. |
| `colossus-update` | Keep release metadata, signature, channel, and atomic-update tests. |
| `colossus-windows-native` | Keep AppContainer/native binding tests on Windows CI. |
| `colossus-windows-process` | Keep Job Object, memory pressure, cleanup, and process-tree tests on Windows CI. |
| `colossus-native-computer` | Keep the isolated workspace's portable framing, target identity and geometry tests; explicit Windows acceptance owns capture, UI Automation, foreground input and helper cleanup. |
| `colossus-work` | Keep durable task, decision, plan, and goal lifecycle tests. |
| `colossus-worker-protocol` | Keep versioned authenticated request/prompt/cancellation/replay tests. |
| `colossus-worker` | Keep worker composition, authentication, shutdown, and restart tests. |
| `colossus-workflow` | Keep parsing, control flow, recovery, triggers, idempotency, and compensation tests. |

Separate CLI integration targets let CI select expensive prerequisites while retaining
each public-boundary assertion.

Linux workspace-identity changes need provider-seam tests in `colossus-home` and
`colossus-runtime`. Preserve a known version-4 birthtime digest and prove missing NFS
birthtime selects version 5. With identical filesystem scope and kernel handle,
transient device, inode, mount-ID and mount-point fields must not independently affect
the hash; the handle itself need not survive inode remapping. Prove separation across
scope, handle type, length and bytes. Fail closed on invalid handle sizing, unsupported
syscalls/filesystems, malformed or changing results, missing/ambiguous scope, and
descriptor/stat disagreement. Runtime tests must independently derive the identity kind
and reject replacement before repository, tool or effect access. Reject inode-only
bootstrap authorization for version 5 even when device/inode match. An unsupported
unrelated NFS volume must not block a supported workspace; malformed records and
duplicate device matches remain rejected. Version-5 revalidation accepts unchanged
scoped digests across changed client device/inode values and rejects changed digests;
retain version-4 metadata checks. Live NFS acceptance supplements these deterministic
negative cases.

## Removal criteria

Remove or rewrite tests for deleted behavior, duplicate weaker evidence, implementation
details without a contract, or fixtures in explicitly rejected formats. Retain historical
storage/protocol coverage while current deployments can present those formats; record
the removal decision before deleting it.

## Verification tiers

During edits, check the changed package and directly affected targets. Run the full
change-selected PR gate after implementation and focused validation are finished;
it already includes the Rust gate when selected. [Source setup and test tiers](setup-testing.md)
owns the commands, prerequisites, timings and CI mapping.

Native browser results below establish only their stated source/fixture boundaries.
[ADR 0008](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work)
owns production readiness, containment, certificate custody and signed-installation
requirements; each platform must supply its own native evidence.

### Portable CI contract prerequisites

`scripts/ci/test-contracts.sh` runs release, classification and offline native-browser
fixtures. Use the repository Node toolchain and Python 3.11+ with `cryptography` 50.x;
set `NODE` and `PYTHON` if these differ from ambient tools. This prerequisite is separate
from the SDK Python pin in the [toolchain inventory](toolchain-inventory.md). Missing
cryptography is a setup failure; do not skip the suite.

The Linux PR **Classify PR changes** job owns POSIX fixture contracts. Unix executable
modes, owner/private modes, no-follow descriptors, descriptor-relative retirement and
file-symlink tests declare their prerequisites and also run on macOS. Windows retains
portable ELF, image/export metadata, compiler-output, origin and evidence validation.
Node release fixtures use Windows junctions or POSIX file symlinks for link rejection.

Native staging, handle/Job cleanup and input belong to the
[Windows browser tiers](#desktop-chromium-development-acceptance-on-windows);
macOS supervisor and PKI acceptance belong to the
[process-broker guide](https://github.com/obscuritylabs/Colossus/blob/main/native/browser/process-broker/README.md).

For Git Bash GNU tar on Windows, set `$env:TAR_OPTIONS = '--force-local'` in the gate's
PowerShell process so drive-letter paths are local filenames. Archive assertions and
machine settings remain unchanged. Use the same Python environment for native-browser
and TLS-broker fixture discovery.

### Desktop provider setup acceptance

Prepare current bundled binaries with `cargo xtask desktop prepare --profile debug`,
then run `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib native_catalog_ -- --ignored`.
This operator-owned tier needs loopback access and the platform credential store.
Disposable private homes, the verified bundled sidecar, authenticated IPC and the
production gateway prove first-run Chat Completions/Responses discovery without a
selected model, card limits, no authorization for unauthenticated endpoints,
Windows/macOS encrypted-vault forwarding, and retries after malformed catalogs or
HTTP 401. Success requires removal of generated runtime keys and homes. Cleanup
failures fail the test; cleanup during another failure preserves that failure and may
retain resources for inspection. Native consent and credential-entry dialogs require
separate on-screen acceptance.

The Windows pre-merge lane also runs `native_catalog_` and `native_uninstall_` with
`--ignored`. The reset regression retains an old secure anchor while recreating the
same home path and verifies both restart stability and successful fresh discovery.
Uninstall acceptance removes generated runtime/vault keys and read-only hard-linked
plugin caches while preserving unrelated entries and external hard links. Setup
acceptance removes its sidecar-created homes through the Windows uninstall helper.
Neither targets the default Desktop home.

### Desktop setup package acceptance

After preparing the Desktop sidecar, run
`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib native_setup_package_ -- --ignored`.
The real sidecar's YAML inspector must accept provider-only packages and the documented
provider/model example without provider requests. Ordinary `setup_package::tests` cover
offline ZIP bounds, credential
placeholders, metadata migration, export, CA handling, and activation conflicts.
From `apps/desktop`, `npx playwright test tests/browser/setup-package.spec.ts`
checks the import review, explicit trust/replacement choices, deferred credentials,
Markdown isolation, and imported provider selection. Native file pickers and vault
entry dialogs still require on-screen acceptance.

### Windows single-instance acceptance

The operator-owned Windows Desktop acceptance tier runs
`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib desktop_instance::native_tests::native_single_instance_restores_original_window -- --ignored --exact`.
It requires WebView2 and an interactive desktop. A temporary browser profile and unique
application identifier isolate a real second process: it must exit successfully and
restore the original hidden/minimized window without opening installed settings or
credentials. Unit tests cover startup serialization and tray click routing.

### Windows computer control acceptance

The separate `native/computer` workspace keeps Windows FFI outside the runtime's
unsafe-code prohibition. The Rust and Desktop gates run its locked metadata,
formatting, tests and Clippy; the PR Desktop job owns the Linux unavailable-platform
checks. Ordinary tests neither capture a desktop nor send input. To check this boundary:

```bash
cargo fmt --manifest-path native/computer/Cargo.toml --all -- --check
cargo test --locked --manifest-path native/computer/Cargo.toml --all-targets
cargo clippy --locked --manifest-path native/computer/Cargo.toml --all-targets --all-features -- -D warnings
```

To check authenticated Desktop tool discovery without capturing a window or
sending input, prepare a matching sidecar with
`cargo xtask desktop prepare --profile debug`, then run from PowerShell 7:

```powershell
$env:COLOSSUS_ACCEPTANCE_SIDECAR = (Resolve-Path apps/desktop/src-tauri/binaries/colossus-sidecar-x86_64-pc-windows-msvc.exe).Path
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features --lib native_desktop_grant_discovers_computer_tools_without_computer_effects -- --ignored
```

This background-capable check needs the ordinary user's unlocked Default desktop,
native credential storage and loopback. Disposable homes and a credential-free local
provider verify the Desktop grant offers all 13 computer tools and `tool.search`
returns them. It removes generated keys/homes, preserves the operator's profile and
performs no computer operation or activation. Capture/input acceptance remains below.

To verify that a persisted Minimal profile can return to Development and Allow All,
use the prepared debug Desktop binaries and run:

```powershell
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --no-default-features --lib native_reused_provider_profiles_restart_and_expose_authenticated_inventory -- --ignored
```

The check reopens disposable settings, restarts Managed Local for each profile and
reads authenticated tool, MCP, search and plugin inventories without external provider
requests. It closes the sidecar before removing generated keys and home.

Native acceptance needs an unlocked interactive desktop under the ordinary user's
token. Its disposable target must be the verified host's sibling: host, controller and
descendants are denied. Titles/process names do not establish target authority.
Missing capture, accessibility or foreground prerequisites must fail acceptance.

From an ordinary PowerShell 7 session on Windows, run:

```powershell
./scripts/ci/computer-windows-native.ps1
```

The harness builds feature-gated `computer_window_fixture` and `computer_acceptance`, then starts
the green target and magenta occluding window as the verified host's siblings. It
requires capture of the selected target while covered, accessibility, explicit focus
and input, stale observations/run revocation, foreign-window denial, owned modal input,
competing writers, cancellation and held-input cleanup after helper termination. If
needed, activate the exact nonce-named cover within the setup deadline; capture and
focus assertions still apply. Children have bounded lifetimes; forced cleanup fails.
Screenshots, JSON receipts and bounded logs remain in the printed private
`ColossusComputerTest-*` directory under the user profile. Remove it after inspection.

The **Representative Windows runtime and sandbox** job runs default tests, lints all
targets/features and builds acceptance examples. Hosted Windows runners use administrator
tokens, so this establishes protocol/compilation evidence, not ordinary-user acceptance.

Before merging computer-control changes, run native acceptance on the final clean
commit. Retain `harness.json`, `acceptance.json` and the selected-window screenshot;
both receipts must report `passed: true`, with the exact `source_revision`,
`source_dirty: false`, `acceptance_sha256` and `fixture_sha256` in `harness.json`.
Repeat dirty-worktree runs on that commit. Physical human takeover and Pause require
a separate manual check.

### Desktop embedded browser preview

From `apps/desktop`, run `npm run test:browser-native` in a graphical Windows/macOS
session with loopback sockets. The feature-gated harness uses a private UUID home and
isolated main WebView profile, removes only that directory after exit, and never uses
saved Desktop credentials/workspaces.

It checks engine history, temporary cookie sharing/isolation, foreign-workspace
rejection, guest IPC/app-origin and native permission denial, suppressed script dialogs,
popups, downloads, clear/close behavior, script exceptions and successful later probes.
Evaluation requires `browser-test-bridge` and `native-test-driver`; no generic evaluation
IPC exists. macOS probes call WebKit directly because the hardened guest replaces Wry's
navigation delegate without draining its initial script queue. Both platform pre-merge
lanes own this tier.

Set `COLOSSUS_BROWSER_INTERACTIVE_ACCEPTANCE=1` to require foreground keyboard/focus
acceptance. CI without a foreground window must deny the viewport lease and report
interactive acceptance outstanding; it cannot prove device keyboard, overlay or focus.

Run native contracts from the repository root:

```sh
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --workspace --lib --features browser-preview
```

From `apps/desktop`, run `npm run test:browser -- tests/browser/browser-pane.spec.ts`
for controls, composer preservation, responsive layout and accessibility.
[ADR 0003](adr/0003-desktop-browser-boundary.md) owns the native isolation,
site-compatibility and release matrix.

For developer review, the canonical launcher can prepare debug sidecars and enable
Browser for the development build:

```sh
./scripts/desktop-dev --browser-preview
```

Combine with `--dev-credentials` for an activated private development home. Direct
Tauri launches accept `--features browser-preview`; normal builds keep it off pending
native release acceptance.

### Desktop Chromium development acceptance on macOS

Prepare the pinned native component with the
[Chromium development launcher](setup-testing.md#run-the-embedded-chromium-preview-on-macos).
Use the source and native build paths it prints:

```sh
COLOSSUS_CEF_ROOT=/absolute/path/to/verified/cef/source \
COLOSSUS_CEF_NATIVE_LIB_DIR=/absolute/path/to/native/build \
  npm --prefix apps/desktop run test:browser-chromium-native
```

Use an unlocked interactive macOS session with loopback and native process inspection;
keep the test window in front. The runner builds renderer assets and a debug CEF-linked
Tauri app, verifies staged ad-hoc signatures and launches through LaunchServices with
an isolated UUID home, without saved workspaces or credentials.

The main WebView document must pass browser IPC's native controller authorization.
Assertions cover PNG pixels, child geometry, CEF/Tauri AppKit coexistence,
navigation/back/forward/reload, shared tabs/hidden siblings, resizing, overlay
occlusion/restoration, expired leases, workspace generations, privileged-origin denial
and disabled agent control. The runner records live renderer seatbelt startup and
rejects sandbox-disabling/remote-debugging flags. It reopens a visible tab before AppKit
quit and requires the exit guard to acknowledge closure. Success requires native close
acknowledgements, `CefShutdown`, cache removal, fixture HTTP requests and no surviving
owned helpers; failure cleanup does not count as successful shutdown.

Logs, the staged app, and `native-acceptance.json` remain in a generated
`.local/chromium-acceptance-UUID` directory. Set
`COLOSSUS_BROWSER_ACCEPTANCE_EVIDENCE_DIR` to a fresh absolute directory to select
the evidence destination. `COLOSSUS_BROWSER_ACCEPTANCE_PAUSE_MS` holds visual
checkpoints for up to 30 seconds each. On macOS sessions that reject programmatic
activation, set `COLOSSUS_BROWSER_ACCEPTANCE_FOREGROUND_WAIT_MS=60000` to allow
up to one minute to activate the observed test window; the foreground check still
must pass.

This tier covers the in-process human preview; separate-host Core acceptance remains
in [ADR 0008](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work).

### Desktop Chromium development acceptance on Windows

#### Non-admin owned Windows browser automation

Use native Windows x64, PowerShell 7, Rust MSVC, Visual Studio C++ desktop tools,
CMake, Python, the repository Node toolchain and an interactive desktop session.
Run from an ordinary non-elevated shell; the launcher rejects an administrator
token. Prepare the normal Desktop frontend dependencies and assets first as
described in [source setup](setup-testing.md).

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File ./scripts/desktop-browser-windows.ps1
```

`powershell.exe` starts Windows PowerShell 5.1. Build and acceptance require
PowerShell 7 (`pwsh`). `-ExecutionPolicy Bypass` applies only to this invocation;
the launcher does not change the machine or user execution policy.

The command builds/verifies a pinned CEF inventory, creates private development state
and embeds its selection in the Debug sidecar. Assertions cover typed browser actions,
native transfers with exact server-observed bytes, tab allocation/selection/close and
Job/pipe/profile cleanup. Core separately requires semantic observations, clicks,
navigation and a released PNG artifact.

The fixture freezes timer-driven HTTP traffic, requires server-observed silence after
egress drain and resumes the same DOM with a new writer generation. After natural
completion, a second Desktop agent turn must reuse the session/document with a fresh
writer/frame, replace the completed tab and retain its last view before shutdown.

A deterministic local provider uses authenticated SDK/sidecar/worker channels to
authorize the controller, hand its human page to an agent and require native click,
navigation, screenshot, read-only detachment and shutdown receipts. Tab retargeting
must reach the foreground HWND with the same writer and a fresh authenticated viewport.
Viewer-stream loss and viewer closure after completion each need authenticated detach
or retained cleanup evidence. Screen probes read the foreground desktop DC for that
HWND; a frame buffer alone is insufficient. The real presenter checks resize/current
display DPI, overlay occlusion, mouse/key dispatch, Unicode surrogate commits and IME
cancel. Other installed IMEs and monitors need separate acceptance.

Fresh logs, three foreground viewport PNGs and `acceptance.json` are preserved
under the printed `.local/` evidence directory. Failed state remains available
for inspection. `-Component <absolute-private-stage>` selects a previously built
verified source component; `-Launch` opens the resulting development Desktop
after acceptance. Neither option imports personal certificates or requests UAC.

Once `-Launch` has prepared the app, reopen the same test profile without a rebuild
or another acceptance run. This option also supports Windows PowerShell 5.1:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/desktop-browser-windows.ps1 -Relaunch
```

The saved source-app hash must still match. A normal Desktop rebuild replaces
that executable, so a changed binary requires another `-Launch` preparation.

For the native host tier alone:

```powershell
./scripts/ci/browser-windows-native.ps1 -Mode Development
```

Add `-HostProfile Release` to build the non-diagnostic native client and exercise
long managed ancestry with a distinct short private engine profile, without file
logging. Both use the actual token's OS profile and the exclusive native allocator.
The existing owned Windows CI tier uses this profile with the same input,
transfer, frame and cleanup assertions; it does not add another acceptance run.

Both release modes remain false. Chromium's sandbox stays enabled; this lane advertises
no outer containment or private-certificate capability. The separate `-Mode Containment`
CI diagnostic does not make standard installation/use require administrator access.

#### Windows rich interactions and annotations

After preparing a verified component, run the maintained interaction tier from an
ordinary-user PowerShell 7 shell:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File ./scripts/ci/browser-windows-interactions.ps1 -Component 'C:\Users\you\ColossusBrowserComponent-<id>\component' -EvidenceDirectory .local/windows-browser-interactions-attempt-1
```

Use a fresh evidence directory for every attempt. The runner verifies the component
inventory and requires both production modes to remain false. It executes the ignored
native Runtime fixture through the effect gateway, checking guest hover, double/right
click, coordinate input at 200% scaling, canvas and HTML drag-and-drop, live native
annotations, clean and annotated PNGs, constrained developer inspection, and protected
field rejection. It also checks stale/resize fences and owned browser-state cleanup.
A successful attempt retains a bounded `native.log`, `annotations.png` and
`acceptance.json`; failed attempts retain available evidence and their private state
for diagnosis.

The runner keeps production availability and all three release acceptance flags false.
Live Desktop agent acceptance remains separate from this Core tier.
See the [native component instructions](https://github.com/obscuritylabs/Colossus/blob/main/native/browser/README.md#windows-interaction-and-annotation-acceptance)
for action limits and component preparation.

#### Native Windows Chromium preview

Use an interactive Windows x64 session with Visual Studio C++ desktop tools,
PowerShell 7, Python, CMake, Node.js and the repository Rust toolchain. Prepare
the pinned component in the same PowerShell terminal:

```powershell
./scripts/desktop-chromium-preview-windows.ps1 -BuildOnly
npm --prefix apps/desktop run test:browser-chromium-native
```

The launcher sets the verified CEF source and native shim paths for that terminal.
The acceptance runner stages a fresh unsigned bootstrap/client component and
isolated home. It requires actual Windows compositor pixels, OS mouse and keyboard
input, native HWND attachment, browser/controller lifecycle checks, acknowledged
CEF shutdown and no remaining owned helpers.

Renderer isolation evidence uses a retained native process handle and checks its
executable and creation time. The live renderer's restricted token must receive
an access denial when opening an operator-private file. The runner rejects
sandbox-disabling and remote-debugging flags. Failed runs preserve their home and
categorical cleanup evidence. The
[native component guide](https://github.com/obscuritylabs/Colossus/blob/main/native/browser/README.md)
owns bootstrap and staging details.

The premerge `windows-desktop` lane also runs
`scripts/ci/browser-windows-native.ps1` as a required owned-Chromium factory check.
Its eligibility is unchanged: a non-draft PR, an authorized `ci:full` request, and
a successful current-head Colossus PR gate. The script builds pinned sandboxed
CEF and runs the ignored native factory fixture for Low-integrity token and
private profile-label readback, actual multipart upload/download bytes, native
presentation/input and complete owned cleanup. This is a separate browser path
from the lane's WebView2 Desktop checks.

The `windows-owned-chromium-source-acceptance` artifact contains a bounded log,
inventory and diagnostic report, excluding profiles, transferred content and keys.
Accepted modes and production containment stay false. Current execution evidence
belongs to [browser readiness](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work).

### Native Chromium certificate acceptance

After preparing the pinned macOS or Windows preview, run:

```sh
npm --prefix apps/desktop run test:browser-pki-native
```

Provision the private temporary CA/client-identity fixtures through native certificate
controls. Assertions cover server trust, server-observed leaf fingerprint, identity
choice, dismissal, stale review and TLS failures. Keys/passwords stay outside the
Desktop renderer and tool arguments.

Remove only generated fixture certificates/keys through native store controls. Evidence
must distinguish operator-reported removal from verified custody. Linux's unattended
owned-NSS-store provisioning requires independent real CEF acceptance.

Windows ordinary-user source preview also has an authenticated native-review tier.
Build the local TLS server and the explicit native test executable:

```powershell
cargo build --locked -p colossus-sandbox --example windows_pki_server
cargo test --locked -p colossus-sandbox --test browser_windows_pki --no-run
python -B native/browser/scripts/run_windows_pki_probe.py `
  --confirm-fixture-store-changes `
  --directory <new-private-fixture-directory> `
  --component <verified-windows-component> `
  --state-parent <existing-owner-private-directory> `
  --test-executable <browser_windows_pki-executable-printed-by-cargo> `
  --server-executable target/debug/examples/windows_pki_server.exe
```

Run as the ordinary user with OpenSSL and Python `cryptography`. The runner creates
private fixtures, unique nonexportable CNG signing keys and fixture-only CurrentUser
ROOT/MY entries, recording certificate/public-key digests before store changes. The
server advertises only the fixture CA; native review accepts only generated public
identities for the exact supervised request. It checks automatic single-identity mTLS
before a viewer lease, then adds a second identity for explicit choice. Server-observed
login, alternate-identity rejection, cancellation, changed-origin denial, run cancellation
and TLS failures must pass without operating browser certificate-picker windows.
Cleanup must verify absence of the exact certificates/key containers and remove private
fixture files. Retain `store-ledger-*.json` checkpoints and
`windows-pki-acceptance.json`; resolve unverified cleanup as a failure against that ledger.

For the same matrix through Core policy and browser tools, first build the runtime
lib test with `cargo test --locked -p colossus-runtime --lib --no-run`, then pass its
printed executable to the runner with
`--test browser_tools::tests::native_windows::pki::core_authenticates_system_ca_and_reviewed_client_identity_after_policy`.
This proves native selection of OS-owned keys, not per-profile custody.

The trusted in-window picker has a separate synthetic Windows acceptance operator.
After building the renderer and preparing the pinned Windows presenter library paths,
run it with a fresh, nonexistent absolute home:

```powershell
$env:COLOSSUS_HOME = Join-Path $env:USERPROFILE ("ColossusIdentityReviewTest-" + [Guid]::NewGuid().ToString('N'))
cargo run --locked --manifest-path apps/desktop/src-tauri/Cargo.toml `
  --no-default-features --features client-identity-test-bridge `
  --example client-identity-review-acceptance
```

This Debug-only operator checks the main window and bundled review child's native
parent/bounds, resize, guest occlusion, exact-document IPC, one-use choice, Cancel,
expiry and workspace invalidation. Synthetic public metadata creates no store entries
or TLS/private-key evidence; use the matrix above for that. The functional preview
still prohibits the unrelated native test-driver bridge.

### Owned browser control conformance

The shared coordinator, bridge and presentation suites exercise private authority,
stale references, admitted document generations, caller loss and cleanup ownership.
Focused native-admission and profile tests can be run from the repository root:

```sh
cargo test --locked -p colossus-browser
cargo test --locked -p colossus-browser-bridge
cargo test --locked -p colossus-browser-presentation
cargo test --locked -p colossus-worker native_browser::tests
cargo test --locked -p colossus-sidecar-protocol native_browser_bootstrap
cargo test --locked -p colossus-sandbox browser_profiles
cargo test --locked -p colossus-sandbox protected_filesystem::native_profiles
```

Profile tests cover restart ownership, exclusive lease/reset rejection, unknown cleanup,
compatibility, replaced inodes and granted filesystem operations against Chromium-shaped
caches/aliases. They prove private filesystem denial, not encryption or disk quota.

For ignored native CEF/Core conformance, select a fresh inventoried component, locally
installed immutable OCI image and native Docker executable using `COLOSSUS_BROWSER_OCI_ACCEPTANCE_COMPONENT`,
`COLOSSUS_BROWSER_OCI_ACCEPTANCE_IMAGE_ID` and
`COLOSSUS_BROWSER_OCI_ACCEPTANCE_DOCKER`. The compatible host lane must support
CEF's sandbox without disabling it and positively reap every owned helper.
The native component README documents staging and startup prerequisites.

The three-channel Core fixture exercises a policy rejection before native
allocation, actual page accessibility observations, protected-value redaction and
run-finish draining through graceful CEF shutdown and physical state removal:

```sh
cargo test --locked -p colossus-runtime --lib \
  browser_tools::tests::native::contained_chromium_runs_through_runtime_policy_and_drains_before_run_finish \
  -- --ignored --exact --nocapture
```

The four-channel fixture additionally requires authenticated 801×601 BGRA page
pixels, denied human input/focus and invented human generation, and continued agent
observation after detaching the independent read-only viewer. The PKI fixture
enrolls exact Core owners through the policy gateway and verifies server-observed
identities, CA/redirect failures and mixed-origin consent denial before allocation:

```sh
cargo test --locked -p colossus-runtime --lib \
  browser_tools::tests::native_presentation::contained_readonly_frames_share_runtime_page_without_human_input_authority \
  -- --ignored --exact --nocapture
cargo test --locked -p colossus-runtime --lib \
  browser_tools::tests::native_pki::contained_chromium_enrolls_native_pki_for_the_actual_core_owner_after_policy \
  -- --ignored --exact --nocapture
```

The persistent-cookie fixture runs five sequential native contexts, requires
server-observed cookies across restart, absence after reset and in new temporary
sessions, same-profile lease exclusion and graceful shutdown of every context:

```sh
cargo test --locked -p colossus-runtime --lib \
  browser_tools::tests::native_profiles::persistent_profile_restarts_cookie_state_and_reset_clears_it_while_temporary_does_not \
  -- --ignored --exact --nocapture
```

Persistent profiles remain diagnostic-only until encryption and quota acceptance;
ordinary configuration is rejected. [Current browser readiness](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work)
records completed native fixtures, the rejected Linux renderer-custody attempt and
remaining platform gates.

### Plan Mode acceptance

Run `cargo test -p colossus-cli --test plan_mode_smoke --test interactive_plan_smoke`
for non-mutation, durable draft writes, refinement, approval, and once-only consumption.
The interactive suite covers line mode and a real PTY in embedded and worker-backed
runtimes. From `apps/desktop`, run
`npm run test:browser -- tests/browser/plan-workflow.spec.ts` for revision controls,
execution mode handoff and retained planning previews. Live-model and packaged-native
acceptance are separate from these deterministic fixtures.

`cargo test -p colossus-api-runtime --lib public_plan_question_resumes_after_answer_and_persists_one_draft`
uses a loopback provider with the production runtime, event forwarding, public run
repository, and interaction router. It verifies that tool-start delivery precedes the
question, an answer reaches the next model request, and the run completes with exactly
one draft. Run this tier when changing Plan Mode, interactive prompts, or event buffering;
an isolated question-card fixture cannot expose an interaction overtaking queued events.

Use a disposable workspace for live acceptance unless the operator requests active
configuration. Name test sessions and record config source, model, reasoning, limits,
access profile, state path and binary version without credentials. Resolve CLI and
Desktop independently: YAML and saved provider/model resources can differ. Verify
packaged CLI/sidecar match source binaries. Shared-workspace checks must run sequentially
even with different state paths; do not bypass leases or change provider/policy settings
to make them pass.

| Live scenario | Evidence to retain |
| --- | --- |
| Simple planning without inspection | One Draft at revision 1, ordered steps, actual provider/model and run ID |
| Plan a documentation change after reading two named files | Only bounded inspection and one plan write; proposed edits explicitly marked `requires_mutation: true` |
| Ask for a file change and task records while in Plan Mode | A draft describes the work; no file or TaskRecord is created |
| Native UI clarification and refinement | Question answered through the UI; same plan ID advances revision; old revision has no continuation controls |
| Native Run once for a harmless output-only plan | Plan becomes Executed, composer enters Execute mode, repeat execution controls retire, planning preview survives |

Compare repository changes before/after each scenario. Retain canonical plans, tool
names, run results and native screenshots separately from mocks; report model errors,
cancellations and retries. Live samples supplement deterministic negative cases.

### Command approval acceptance

`cargo test -p colossus-cli --test approval_smoke` uses isolated homes and loopback
providers to test missing-reason recovery, full-command details, allow-once/denial and
real PTY Request-tab scrolling in embedded/worker hosts. Markers prove no execution
while pending and exactly one after acceptance. Policy tests bind justification,
executable, arguments and working directory to immutable proof and require sanitized
evidence before requesting a decision.

From `apps/desktop`, `npm run test:approval-runtime` builds a feature-gated example and
sidecar, driving the production review component/native adapter, authenticated worker
and separate broker. Fresh private homes and a credential-free local provider isolate
each run. Only human decisions are supplied; refetch, authorization, policy, permits
and execution stay real. Production links no test bridge. Foreground/managed commands
cover Allow once, remembered exact-command approval, denial and cancellation;
stdout/stderr at 1,024/4,096-byte limits; and zero/nonzero exits. A Windows server on
a dynamic port must survive a later turn, reappear after UI reconnect, expose released
logs and stop from Active shells with its listener closed. A disposable occupied port
must produce visible startup failure without touching existing listeners. macOS/Windows
pre-merge lanes run without scenario retries. Screenshots go to
`apps/desktop/output/playwright`; failure traces are retained. Browser mocks cover
keyboard, compact layouts, accessibility, redaction, full details and stale reviews.

On Unix, the allowed command must run over ten seconds, exit zero and append one marker.
Windows uses immediate PowerShell/.NET markers without cmdlet autoload: AppContainer
setup and its seven-second cleanup reserve share the effect budget. Default interpreter,
isolation, limits, exact-once and no-execution-before-approval assertions remain unchanged.
The test collector waits 45 seconds for the normal 30-second process budget and terminal
publication without extending execution/approval limits. Collection failures expose only
categorical status and pending-interaction count. Process failures expose allowlisted
categories, numeric exits and marker presence, never private provider errors, arguments,
bindings or output. Retain public terminal status, allowlisted reason and outcome certainty;
a timeout need not trigger another model request. Separate `test-results/plugin-runtime`
and `test-results/approval-runtime` directories preserve each failed suite's CI traces.

Both scripts put Tauri output in the `desktop-acceptance/` child of `CARGO_TARGET_DIR`
when set. External-binary staging must not overwrite the tested fresh CLI/sidecar with
older staged binaries. Relative target paths resolve from the repository.

On-screen smoke acceptance must verify the review window opens, blocks external
navigation, invalidates review on close and makes Allow once / Always allow / Deny final
without another OS dialog. Full command/working directory must remain readable. Native
remembered-command tests cover exact matching, workspace identity, redacted-command
rejection, reopen persistence and clearing.

On Windows, the operator-driven
`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib command_review::native_tests::native_review_buttons_and_close -- --ignored --exact --nocapture`
opens three isolated WebView2 review windows. Follow each window's fixture instruction:
Allow once, Always allow, then close the window. The test uses the shipped custom
protocol, verifies blocked external navigation and native decision delivery, and never
executes a command or touches user settings. Build the renderer first and set
`TAURI_CONFIG` to `{"build":{"devUrl":null}}` for bundled assets.

### Desktop plugin runtime acceptance

From `apps/desktop`, run `npm run test:browser:install` once, then
`npm run test:plugin-runtime`. It builds the CLI and opt-in `plugin-test-bridge`, copies
the CLI outside the checkout and drives production React/native components through an
authenticated worker in a private temporary home. Test paths/approval responses replace
OS dialogs only; policy, journal, OCI packaging, trust and IPC remain real, without a
registry prerequisite. Assertions cover canonical worker endpoints (including Windows
verbatim paths) and bounded subprocess shutdown. Stop refresh and close every owned
process before deleting fixtures; preserve original failures during cleanup. Check
rendered management results only after the matching request completes within its IPC
bound. Scenario retries are disabled.

`npm run test:browser` covers mocked interactions and entry through the production
Workspace sidebar at desktop/compact widths, keyboard navigation and unavailable
discovery state. macOS Desktop and Windows runtime pre-merge lanes own the real-worker
tier. Retain failure traces and `output/playwright` screenshots. Native adapter tests
cover path replacement/cancellation. The driver is a feature-gated example; production
renderer checks reject development bridge markers.

### Embedded plugin and selection acceptance

The bundled security-review scratch initializer has stdlib-only Python checks:
`python3 -B -m unittest discover -s scripts/tests -p test_security_review_workspace.py`.
They verify ignored local storage, temporary fallback, private POSIX modes, safe path
handling, and preservation of existing notes without changing source or Git configuration.
`cargo test -p colossus-bundled-plugins --lib` verifies the embedded plugin matches the
portable directory artifact, including the skill's references and helper.

`cargo test -p colossus-cli --test plugins_tui_smoke` exercises a real PTY against
both embedded and authenticated-worker hosts with private offline homes. It covers
completion, rendered core names, skill and
resource inspection, conversation selection removal, lifecycle refresh, errors, and
terminal resizing.

`cargo test -p colossus-cli --test provider_terminal_smoke worker_plugin_inputs`
uses a deterministic loopback provider to observe the actual requests. It checks
metadata-only discovery, selected instruction loading, unchanged tool definitions,
snapshot-bound reads during a global disable, rejected stale selections on later runs,
and selected IDs plus exact manifest digests in audit evidence.

Native Desktop checks need an unlocked platform credential store, a fresh explicit
`COLOSSUS_HOME` and scratch workspace. Store failure blocks acceptance; do not replace
encryption/credentials to hide it. The browser-worker bridge cannot replace native
dialog or OS integration evidence.

The credential-vault matrix in pre-merge acceptance runs deterministic conformance on
Windows, macOS, and Linux, then runs
`cargo test --locked -p colossus-credentials --lib platform_master_key_survives_vault_reopen -- --ignored`
against the real OS store. The test owns synthetic records and a dedicated generated
key, checks exact bytes from a second process, and deletes only that generated key.
Linux needs an unlocked persistent Secret Service collection inside a D-Bus session.
Initialization fault tests cover each durable transition and an acknowledged-write
failure; separate tests cover ownership conflicts, tampering, and unsafe paths.

Desktop's private native UI crate has Windows real-control tests. Its AppKit driver
must run on the process main thread:
`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p colossus-native-credential-ui --features native-test-driver --test native-macos`.
The Windows driver uses an isolated window station and clipboard to exercise native
paste and keyboard messages without changing the user's clipboard:
`cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p colossus-native-credential-ui --features native-test-driver --test native-windows`.
Both platform lanes own these checks.

Linux's native GTK control checks require an operator-owned display or Xvfb. Run
`xvfb-run -a cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p colossus-native-credential-ui --lib linux::tests:: -- --ignored --test-threads=1`
to check secure entry, cancellation, appearance and the provider destination shown
inside the same prompt. Windows tests do not establish GTK or AppKit acceptance.

Manual acceptance additionally pastes 761, 762,
2,560, 2,561, 8,192, and 65,536 bytes, rejects 65,537, tests keyboard navigation,
cancellation and parent closure, and follows save/restart/load through a real MCP
discovery and tool call. Record actual Splunk deployment acceptance separately;
synthetic loopback credentials do not prove a deployment's header limits.

Both Desktop platform lanes also run the native backend acceptance test
`managed_runtime::credential_acceptance::native_vault_restarts_reach_managed_sidecar_mcp`
with `--lib -- --ignored --exact` and `COLOSSUS_ACCEPTANCE_SIDECAR` set to the absolute
path of the prepared matching sidecar. Separate processes save and reopen 8,192- and
65,536-byte synthetic credentials through Desktop's vault, use production bootstrap
construction, and verify exact provider and MCP discovery/tool-call authorization at
a loopback server. The fixture checks renderer metadata, released output, and generated
files for plaintext and removes only its generated platform key and private home.
This backend test is separate from native input and physical Desktop acceptance.

Windows release smoke fixtures use fresh owner-private directories under the current
user profile, not the runner's potentially shared temporary directory. The Windows
pre-merge lane runs `release_install_smoke`, including the same fixture helper and core
bootstrap used by release packaging. The fixture owns all temporary installation,
plugin-home, and bundle paths and restores the caller's environment on completion.
Unix installer acceptance covers both permitted sticky ancestors and rejection of
writable ancestors without sticky protection; BSD mode inspection must retain that bit.
