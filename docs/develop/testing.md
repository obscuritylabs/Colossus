---
title: Test strategy and crate audit
description: Choose the right Rust test boundary and understand the retained test ownership for every Colossus crate.
audience: developer
type: reference
---

# Test strategy and crate audit

Tests are evidence for supported behavior, not a second implementation or a historical
archive. Put each assertion at the lowest boundary that can prove the contract.

## Test levels

| Level | Use it for | Location |
| --- | --- | --- |
| Unit | Pure rules, parsing, state transitions, bounded rendering, and adapter-local failure | Next to the module or in `src/tests.rs` / `src/tests/` |
| Integration | Public crate boundaries, persistence conformance, subprocess protocols, platform isolation, and live adapters | `<crate>/tests/` |
| End to end | Installed CLI, worker, Desktop, SDK, release, and operator workflows | CLI/Desktop suites and CI workflows |

Do not promote a unit test to a process test merely for realism. Do not replace a
security, protocol, persistence, or packaging boundary test with a mock merely for speed.
An ignored live test is acceptable only when its prerequisite is explicit and a named CI
job or documented operator command owns its execution.

## Crate-by-crate audit

This inventory records the August 2026 Rust-cutover cleanup. “Keep” means the suite
protects a current boundary; it does not mean every future assertion is permanent.

| Crate | Test ownership and disposition |
| --- | --- |
| `colossus-access` | Keep profile resolution, action precedence, and tool-ceiling unit tests. |
| `colossus-agent` | Keep turn-loop, tool, cancellation, and observability tests; the separate observability target proves exported signals. |
| `colossus-api-proto` | Keep generated-contract and compatibility tests for the public protocol. |
| `colossus-api-runtime` | Keep service authorization, streaming, enrollment, and repository integration tests. |
| `colossus-api` | Keep server composition and public API lifecycle tests. |
| `colossus-audit` | Keep journal export, retry, recovery, and live WORM acceptance tests. |
| `colossus-cli` | Keep command-level smoke suites because they exercise public parsing and embedded/worker boundaries; remove a suite only when its command or contract is removed. |
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
| `colossus-memory-chroma` | Keep projection/retry tests and the opt-in live Chroma target. |
| `colossus-memory` | Keep canonical lifecycle, scope, Tantivy projection, and fallback tests. |
| `colossus-network` | Keep DNS pinning, redirects, trust roots, response bounds, and authority tests. |
| `colossus-observability` | Keep disabled-by-default, redaction, payload-mode, and exporter tests. |
| `colossus-plugins` | Keep upstream schema/frontmatter, component isolation, OCI archive, registry auth/origin, Sigstore trust, lifecycle lease, MCP overlay, and confinement tests. |
| `colossus-bundles` | Keep retained release-bundle signature, inventory, no-clobber, and installation tests. |
| `colossus-policy` | Keep built-in and OPA decision tests, including opt-in live and mTLS targets. |
| `colossus-ports` | Keep reusable port-conformance helpers; avoid adapter behavior here. |
| `colossus-presentation` | Keep pure document/theme/rendering tests; the obsolete Python theme-import test was removed. |
| `colossus-projection` | Keep deterministic projection, checkpoint, and rebuild tests. |
| `colossus-provider` | Keep provider translation, streaming, limits, malformed output, and redaction tests. |
| `colossus-research` | Keep evidence bounds, citations, lane failures, and deterministic fallback tests. |
| `colossus-runtime` | Keep composition and cross-service security tests; obsolete `research.search` compatibility assertions were removed. |
| `colossus-sandbox` | Keep native/OCI/Windows contract, broker, cleanup, and hostile-input tests. |
| `colossus-sdk` | Keep embedded/native-sidecar/gRPC parity and subprocess lifecycle tests. |
| `colossus-search` | Keep SearXNG/SerpAPI parsing, credentials, bounds, and role-routing tests. |
| `colossus-session` | Keep message, branch, restore, and context-view repository tests. |
| `colossus-sidecar-protocol` | Keep authenticated framing and workspace-identity compatibility tests; they protect deployed sidecars. |
| `colossus-sidecar` | Keep native and Windows bootstrap/lifecycle acceptance targets. |
| `colossus-telemetry` | Keep durable event, query, retention, and bounded-export tests. |
| `colossus-testkit` | Keep shared conformance tests and fixtures used by adapter crates. |
| `colossus-tools` | Keep schema-first validation, gateway adapters, confinement, output, and mutation tests. |
| `colossus-tui` | Keep reducer, layout, input, theme, restoration, and PTY history tests; they cover behavior not proved by snapshots alone. |
| `colossus-update` | Keep release metadata, signature, channel, and atomic-update tests. |
| `colossus-windows-native` | Keep AppContainer/native binding tests on Windows CI. |
| `colossus-windows-process` | Keep Job Object, memory pressure, cleanup, and process-tree tests on Windows CI. |
| `colossus-work` | Keep durable task, decision, plan, and goal lifecycle tests. |
| `colossus-worker-protocol` | Keep versioned authenticated request/prompt/cancellation/replay tests. |
| `colossus-worker` | Keep worker composition, authentication, shutdown, and restart tests. |
| `colossus-workflow` | Keep parsing, control flow, recovery, triggers, idempotency, and compensation tests. |

The CLI integration directory contains intentionally separate suites for agent,
approval, audit export, authentication, bootstrap/install, bundles, configuration,
context, documentation examples, integrations, MCP, native/OCI/Windows sandboxing,
plugins, plans, providers, release installation, research, search, rejection,
worker, and workflow behavior. Their separation lets CI select expensive prerequisites
without weakening the public-boundary assertions.

Linux workspace-identity changes require focused provider-seam tests in both
`colossus-home` and `colossus-runtime`. Preserve a known version-4 birthtime digest;
prove that missing NFS birthtime selects version 5; and prove that transient device,
inode, mount-ID, and mount-point fields are not independently hashed when the filesystem
scope and kernel-supplied opaque handle remain identical. Do not assume that the opaque
handle itself remains stable across an inode remap. Prove separation across filesystem
scope, handle type, length, and bytes. Cover bounded handle sizing, unsupported
syscalls/filesystems, malformed or changing results, missing or ambiguous scope, and
descriptor/stat metadata disagreement as fail-closed cases. Runtime tests must
independently reproduce the expected identity kind and reject replacement before
repository, tool, or effect access. An inode-only bootstrap token must not authorize a
version-5 identity even when its device and inode match. Unsupported identity scope on
an unrelated NFS volume must not prevent selecting a supported workspace, while
malformed record structure and duplicate device matches remain rejected.
Version-5 revalidation must accept the same scoped digest across changed client
device/inode values, reject changed digests, and retain version-4 metadata checks.
A live NFS acceptance test may supplement these
contracts, but cannot replace the deterministic negative cases.

## Removal criteria

Remove or rewrite a test when its product behavior has been deliberately removed, it
duplicates stronger evidence at the same boundary, it asserts implementation detail
without a contract, or its fixture describes a format Colossus explicitly rejects. Keep
historical storage or protocol readers only while current deployments can present those
formats; record a later removal decision before deleting that evidence.

## Verification tiers

During iteration, run the changed crate's library tests and directly affected targets.
Then use `cargo xtask dev`, `cargo xtask check rust`, and finally
`cargo xtask pr --base origin/main`. See [Source setup and test tiers](setup-testing.md)
for prerequisites and CI mapping.

### Desktop provider setup acceptance

Prepare current bundled binaries with `cargo xtask desktop prepare --profile debug`,
then run `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib native_catalog_ -- --ignored`.
This operator-owned native acceptance tier requires loopback access and the platform
credential store. It uses disposable private homes, the verified bundled sidecar,
authenticated worker IPC, and the production provider gateway. It proves first-run
Chat Completions and Responses model discovery without a selected model, card limits,
absent authorization for unauthenticated endpoints, real encrypted-vault credential
forwarding on Windows/macOS, and successful retries after malformed catalogs and
HTTP 401. Successful runs verify removal of their exact generated runtime keys and
homes; cleanup failures fail the test. During an existing test failure, cleanup
diagnostics preserve that original failure and may leave generated resources for
inspection. It does not
automate native consent or credential-entry dialogs; those still require on-screen
acceptance. Browser mocks alone do not exercise this native boundary.

### Desktop embedded browser preview

From `apps/desktop`, run `npm run test:browser-native` to build and exercise the
feature-gated native browser harness. It requires a graphical Windows/macOS session
and loopback sockets. It creates an owner-private UUID home under the user profile,
uses an isolated main WebView profile, and removes only that exact generated directory
after the process exits. It never uses saved Desktop credentials or workspaces.

The harness checks real engine history, temporary cookie sharing/isolation, foreign
workspace rejection, guest IPC/app-origin denial, native permission denial, suppressed
script dialogs, popups, downloads, and clear/close behavior. Its fixture evaluation is
compiled only with `browser-test-bridge`; no generic evaluation IPC command exists.
The macOS and Windows pre-merge lanes own this acceptance tier.

Set `COLOSSUS_BROWSER_INTERACTIVE_ACCEPTANCE=1` in an interactive desktop session
to additionally require foreground keyboard/focus acceptance. Ordinary CI can have no
foreground OS window; it checks that this condition denies the viewport lease and
reports the interactive gate as outstanding. That result does not establish on-device
keyboard, overlay, or focus behavior.

Run native contracts from the repository root:

```sh
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --workspace --lib --features browser-preview
```

From `apps/desktop`, run `npm run test:browser -- tests/browser/browser-pane.spec.ts`
for controls, composer preservation, responsive layout, and accessibility. Fixture
pages do not prove native isolation or site compatibility. The remaining release
matrix is recorded in [ADR 0003](adr/0003-desktop-browser-boundary.md).

For developer review, prepare debug sidecars, then from `apps/desktop` run:

```sh
npm run tauri -- dev --features browser-preview -- --locked
```

The feature is off in normal builds until the native release gates pass.

### Plan Mode acceptance

Run `cargo test -p colossus-cli --test plan_mode_smoke --test interactive_plan_smoke`
for non-mutation, durable draft writes, refinement, approval, and once-only consumption.
The interactive suite covers line mode and a real PTY in embedded and worker-backed
runtimes. From `apps/desktop`, run
`npm run test:browser -- tests/browser/plan-workflow.spec.ts` for revision controls,
execution mode handoff, and retained planning previews. These deterministic providers
and browser fixtures prove contracts; they do not prove a live model follows the
instructions or that a packaged native application is correctly connected.

`cargo test -p colossus-api-runtime --lib public_plan_question_resumes_after_answer_and_persists_one_draft`
uses a loopback provider with the production runtime, event forwarding, public run
repository, and interaction router. It verifies that tool-start delivery precedes the
question, an answer reaches the next model request, and the run completes with exactly
one draft. Run this tier when changing Plan Mode, interactive prompts, or event buffering;
an isolated question-card fixture cannot expose an interaction overtaking queued events.

For live acceptance, use a disposable workspace by default. When an operator explicitly
requests their active configuration, use clearly named test sessions and record the
resolved config source, model, reasoning, limits, access profile, state location, and
binary version without exporting credentials. Resolve CLI and Desktop independently:
workspace YAML and Desktop's saved provider/model resources may differ. Confirm the
packaged CLI and sidecar match the source binaries. Run the CLI and Desktop checks
sequentially when they share a workspace; workspace ownership is exclusive even with
different state paths. Do not disable the lease or change provider/policy settings to
make the test pass.

| Live scenario | Evidence to retain |
| --- | --- |
| Simple planning without inspection | One Draft at revision 1, ordered steps, actual provider/model and run ID |
| Plan a documentation change after reading two named files | Only bounded inspection and one plan write; proposed edits explicitly marked `requires_mutation: true` |
| Ask for a file change and task records while in Plan Mode | A draft describes the work; no file or TaskRecord is created |
| Native UI clarification and refinement | Question answered through the UI; same plan ID advances revision; old revision has no continuation controls |
| Native Run once for a harmless output-only plan | Plan becomes Executed, composer enters Execute mode, repeat execution controls retire, planning preview survives |

Compare repository changes before and after each planning scenario. Retain canonical
plan records, tool-call names, run results, and native screenshots separately from mock
test results. Record model errors and cancellations as observed; do not turn retries
into an unqualified first-attempt pass. Live samples supplement the deterministic
negative cases and cannot guarantee every future model response.

### Command approval acceptance

`cargo test -p colossus-cli --test approval_smoke` uses isolated homes and deterministic
loopback providers to exercise missing-reason recovery, built-in full-command details,
allow-once and denial, and real PTY Request-tab scrolling with both embedded and worker
hosts. Marker files prove that no command executes while approval is pending and that
accepted commands execute only once. Policy tests bind justification, executable,
arguments, and working directory to the immutable proof and verify sanitized evidence
is written before the decision is requested.

From `apps/desktop`, `npm run test:approval-runtime` builds a feature-gated acceptance
example and the real sidecar. It drives the production review component through the
production native approval adapter, authenticated worker, and separate approval broker.
Allow, deny, and cancellation use fresh private homes with a credential-free local
provider. The test substitutes only the human OS-dialog decision; pending-interaction
refetch, authorization, policy, permits, and process execution remain real. No test bridge
is linked into production. The macOS and Windows pre-merge lanes run this tier with no
scenario retries. Screenshots are in `apps/desktop/output/playwright`, with browser traces
retained on failure. Mocked browser tests separately cover keyboard operation, compact
layouts, accessibility, redaction, full details, and stale review state.

On Unix, the allowed command deliberately runs for more than ten seconds, then must
exit successfully and append exactly one marker. Windows uses immediate markers:
AppContainer setup and its seven-second cleanup reserve share the effect budget.
The Windows fixture uses core PowerShell/.NET marker writes without cmdlet module
autoload; the normal default interpreter, process isolation, and limits are unchanged.
Both platforms retain zero-exit, exact-once, and no-execution-before-approval checks.
The test-only activity collector allows
45 seconds to observe the normal 30-second process budget and terminal publication;
it does not extend runtime execution or approval limits. Collection failures report
only categorical run status and pending-interaction count, not private challenges.
The plugin and approval runners use separate `test-results/plugin-runtime` and
`test-results/approval-runtime` directories so a subsequent suite cannot erase a
failed suite's traces before the CI artifact upload.
On process failure, the fixture reports only allowlisted failure categories,
numeric exit codes, and whether its start/completion markers exist. Private
provider error text, command arguments, bindings, and output are never echoed.
The collector also retains the public terminal status, allowlisted reason, and
outcome certainty, since a terminal timeout need not produce another model request.

Both native acceptance scripts isolate Tauri's build output from the runtime under
test. With `CARGO_TARGET_DIR` set, Tauri uses its `desktop-acceptance/` child: Tauri's
external-binary staging must never overwrite the freshly compiled CLI or sidecar
with a previously staged binary. Relative target paths resolve from the repository.

Native on-screen smoke testing must additionally verify that the isolated command review
window opens, external navigation is blocked, closing it invalidates review, and final
OS confirmation identifies the target, reason, and review binding without presenting a
truncated command as complete. Browser acceptance does not substitute for this check.

### Desktop plugin runtime acceptance

From `apps/desktop`, run `npm run test:browser:install` once, then
`npm run test:plugin-runtime`. The command builds the CLI and the explicitly opt-in
`plugin-test-bridge` example. The test copies the CLI out of the checkout, creates a
private temporary home, and drives production React components through the production
native plugin adapter into an authenticated worker. Test-owned paths and approval
responses replace only OS dialogs; runtime policy, journal, OCI packaging, trust, and
IPC authentication stay real. The test has no registry prerequisite.
The tier first checks that the bridge derives the worker's canonical state endpoint
(including Windows verbatim paths), and tests bounded subprocess shutdown. Browser
refresh is stopped and every owned process is closed before deleting the private
fixture; cleanup diagnostics must not replace the original scenario failure.
Management assertions wait for the matching native request to finish within its IPC
bound before checking the rendered result. This tier disables scenario retries so a
passing CI result cannot conceal a failed first attempt.

Ordinary `npm run test:browser` runs mocked interface interaction cases separately.
It also enters Plugins through the production Workspace sidebar at desktop and compact
widths, using keyboard navigation and checking the explicit unavailable state for a
target without discovery support. A standalone plugin-component fixture cannot prove
that the management screen is reachable from the application shell.
The macOS Desktop and Windows runtime pre-merge lanes also run the real-worker tier.
Browser traces are retained on failure; plugin screenshots are written under
`output/playwright`. Native adapter unit tests cover path replacement and cancellation.
The driver is a feature-gated Cargo example, not a production binary or command;
production renderer checks reject development bridge markers.

### Embedded plugin and selection acceptance

`cargo test -p colossus-cli --test plugins_tui_smoke` exercises a real PTY against
both embedded and authenticated-worker hosts with private offline homes. It covers
completion, rendered core names (including the former `Item 1` regression), skill and
resource inspection, conversation selection removal, lifecycle refresh, errors, and
terminal resizing.

`cargo test -p colossus-cli --test provider_terminal_smoke worker_plugin_inputs`
uses a deterministic loopback provider to observe the actual requests. It checks
metadata-only discovery, selected instruction loading, unchanged tool definitions,
snapshot-bound reads during a global disable, rejected stale selections on later runs,
and selected IDs plus exact manifest digests in audit evidence.

Native Desktop checks need an unlocked platform credential store. A credential-store
failure is a blocked native check, not an offline-runtime pass; do not replace encryption
or platform credentials to hide it. Use a fresh explicit `COLOSSUS_HOME` and a scratch
workspace for manual acceptance. The browser-to-worker bridge is separate evidence and
does not substitute for native dialogs or operating-system integration.

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
Both platform lanes own these checks. Manual acceptance additionally pastes 761, 762,
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
