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

Ordinary `npm run test:browser` runs mocked interface interaction cases separately.
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
