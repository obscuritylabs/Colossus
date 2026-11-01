---
title: Feature ownership inventory
description: Locate the canonical implementation, documentation, and acceptance evidence for supported Colossus capabilities.
audience: developer
type: reference
---

# Feature ownership inventory

This page is a navigation index, not a duplicate product specification. Public behavior
belongs in the linked user, operator, and reference pages; executable truth belongs in
the owning crates and tests.

| Capability | Owning implementation | Canonical documentation | Primary evidence |
| --- | --- | --- | --- |
| Access profiles, approvals, policy, and effect release | `colossus-access`, `colossus-policy`, `colossus-runtime` | [Access and approvals](../admin/access-and-approvals.md), [Security architecture](security-architecture.md) | Policy unit/live tests and runtime deny-before-adapter tests |
| Native, OCI, and Windows isolation | `colossus-sandbox`, platform process crates, sidecar crates | [Sandbox](../admin/sandbox.md) | Platform acceptance suites and CLI sandbox targets |
| Durable journal, projections, recovery, and audit export | journal, projection, audit, telemetry crates | [State and recovery](state-recovery.md), [Audit and recovery](../admin/audit-telemetry-recovery.md) | Shared conformance, tamper, crash, outage, and export tests |
| Providers, model routing, and normalized turns | `colossus-provider`, `colossus-agent`, `colossus-runtime` | [Choose a provider](../use/providers/index.md), [Providers and models](../reference/configuration/providers-models.md) | Provider and agent suites plus CLI provider smoke tests |
| Provider-native document inputs | `colossus-contracts`, `colossus-media`, provider adapters, runtime/API artifact admission and client adapters | [Native document inputs](../reference/configuration/providers-models.md#native-document-inputs), [Public API and SDKs](application-sdk.md) | Document envelope and ownership tests, bounded context preparation, native wire projection and diagnostic/replay checks; live-provider acceptance remains separately scoped |
| Sessions and context | `colossus-session`, `colossus-context`, `colossus-runtime` | [Sessions](../use/sessions.md), [context and snapshots](../use/sessions-context.md) | Repository, snapshot, compaction, and branch tests |
| Tasks, decisions, plans, goals, and subagents | `colossus-work`, `colossus-agent`, `colossus-runtime` | [Tasks, decisions, and plans](../use/tasks-decisions-plans.md), [Goals and subagents](../use/goals-subagents.md) | Lifecycle, lineage, budget, cancellation, and recovery tests |
| Memory and lexical projection | `colossus-memory` | [Memories](../use/memories.md), [Context and memory configuration](../reference/configuration/context-memory-research.md) | Canonical lifecycle, Tantivy, fallback, and recovery tests |
| Search and source-backed research | `colossus-search`, `colossus-research`, `colossus-runtime` | [Web search](../use/web-search.md), [Deep research](../use/deep-research.md) | Search adapter, evidence/citation, and CLI research tests |
| Workflows and triggers | `colossus-workflow`, `colossus-runtime` | [Workflow authoring](../extend/workflows/authoring.md), [Triggers and recovery](../extend/workflows/triggers-recovery.md) | Parser, control-flow, idempotency, compensation, and restart tests |
| Agent Plugins, integrations, standalone MCP, and release bundles | extension crates and runtime adapters | [Extend Colossus](../extend/index.md) | Upstream schema, confinement, OCI, Sigstore, registry, lifecycle, and live-adapter tests |
| Conditional documentation guidance | runtime configuration and instruction snapshots, bundled documentation skill | [Skills and plugins](../use/skills-plugins.md), [Plugin configuration](../reference/configuration/extensions.md) | Documentation policy, instruction precedence, bounded resource reads, and sidecar configuration tests |
| Public API, worker, SDKs, and Desktop | API, gRPC, worker, SDK crates and `apps/desktop` | [Public API and SDKs](application-sdk.md), [Colossus Desktop](../desktop/index.md) | Protocol, enrollment, parity, sidecar, SDK, and Desktop tests |
| Draft prompt suggestions | Runtime/API prediction service, worker/SDK/gRPC and Cloud adapters, `colossus-presentation`, `colossus-tui`, `apps/ui` | [Terminal UI](../use/terminal-ui.md), [Desktop work](../desktop/work.md), [Public API and SDKs](application-sdk.md) | Audited auxiliary inference, single-attempt settlement and usage isolation tests; capability and transport parity; editable-draft keyboard and lifecycle tests |
| Human App Shots and Desktop companion | `native/computer`, Desktop `desktop_extras` and renderer | [App Shots](../desktop/work.md#capture-another-windows-app), [Desktop settings](../desktop/settings.md), [ADR 0013](adr/0013-human-app-shots.md) | Synthetic Windows capture example and retained helper cleanup; preview, destination fencing and preference tests; isolated companion window capability |
| Managed local worktrees | `colossus-worktrees`, CLI and Desktop adapters | [Managed worktrees](../use/worktrees.md), [ADR 0012](adr/0012-managed-worktrees.md) | Offline repository, cross-process lease, ownership/link and cleanup tests; command-level CLI and rendered settings tests |
| Windows human browser sign-in controls | Desktop browser manager and native WebView2 adapter | [Desktop browser](../desktop/tools.md), [Browser boundary](adr/0003-desktop-browser-boundary.md) | Isolated synthetic profile restart/clear, user-agent, TLS and client-certificate acceptance harness; profile lock and redacted diagnostics tests. Agent Chromium availability remains governed separately below. |
| Hosted execution, Web Chat and managed attachments | `colossus-cloud`, `colossus-cloud-chat`, `colossus-cloud-executor`, server `hosted` modules and `apps/web` | [Hosted execution](adr/0009-hosted-execution.md), [Hosted operation](../cloud/work.md#start-a-hosted-conversation), [Cloud development](cloud-control-plane.md#develop-hosted-personal-chat) | Owner-bound migration and lease tests, HTTP attachment/isolation acceptance, shared SQL two-replica recovery and Web browser tests; arbitrary hosted workspace provisioning remains unavailable |
| Single-host Compose deployment with Chat by default | `deploy/compose`, shared `deploy/control-plane/controlled-project.ps1`, release bundle workflow | [Install with Compose](../cloud/deployment/compose.md) | Windows/Linux initialization, PKI and retained-secret checks; published-image HTTPS, authentication, Echo/SSE and retained conversation after ordinary Compose restart |
| Agent messages, durable inboxes, and incoming A2A | `colossus-communication`, agent/runtime/API/SDK crates, `apps/ui`, `colossus-a2a-server` | [ADR 0007](adr/0007-agent-communication.md), [Inspect agent messages](../use/agent-communication.md), [Host A2A](agent-communication.md) | Canonical replay and attempt tests, provider input/hash and budget tests, owner/scope/recovery tests, A2A wire tests, host checks and rendered inbox tests |
| Owned browser control and native Chromium development adapters | `colossus-contracts`, `colossus-ports`, `colossus-browser`, `colossus-browser-bridge`, `colossus-browser-presentation`, `colossus-sandbox`, runtime/worker/SDK browser adapters, `native/browser`, Desktop native adapter | [Owned Chromium browser](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work) | Shared authority, private transport, host cleanup, artifact release and Linux profile protection tests; actual Core open/snapshot/run-finish, four-channel read-only presentation, five-host profile restart/reset and per-request PKI conformance. The Mac temporary HTTP Core diagnostic passed real page control, screenshots, Retina frames and direct-host shutdown; its complete helper/crash/HTTPS/PKI and Desktop foreground acceptance remain outstanding. Linux native NSS TLS and macOS human preview retain their own scopes. Linux direct-key-denial and Windows/broker acceptance, same-page agent-to-human transfer, encrypted/quota-bound persistent profiles and management, multi-origin human SSO and publisher-accepted packages remain incomplete. Production browser availability stays disabled. |
| Offline Desktop and local TUI dictation | `crates/colossus-native-dictation`, `apps/desktop/src-tauri/src/dictation.rs` | [Evaluate offline Desktop dictation](desktop-dictation.md) | Bounded model/audio/revision tests; continuous send and cancellation tests; native macOS/Windows measurements and permission acceptance still required |
| Terminal and presentation | `colossus-presentation`, `colossus-tui`, interface adapters | [Terminal UI](../use/terminal-ui.md), [Extensions and presentation](extensions-presentation.md) | Semantic document, reducer, layout, PTY, and restoration tests |
| Fleet human terminals | `colossus-terminal`, API/SDK/gRPC, connector/Control Plane, `apps/web` | [Enable a web shell](../cloud/connect-runtimes.md#enable-a-web-shell), [ADR 0011](adr/0011-fleet-human-terminals.md) | Native PTY input, ownership and replay tests; authenticated two-replica Fleet transport and web terminal acceptance |
| Releases and updates | `colossus-update`, CLI installers, release workflows | [Core release operations](releasing.md), [Upgrade and compatibility](../get-started/upgrade-compatibility.md) | Release-contract, clean-install, signature, channel, and platform jobs |

Release OCI distribution supports the full version tag and `-windows`, `-linux`,
and `-mac` subsets through the release publisher and ORAS. CLI/TUI binaries,
VS Code packages and published Desktop packages follow their platform; shared
API/SDK assets remain in every subset, and web/Control Plane Linux bundles follow
Linux. Client installation and runtime behavior are unchanged. Evidence is in
`scripts/ci/release-oci.test.mjs` and the publisher's digest-pinned pull verification;
registry publication remains a hosted release operation.

## Interface coverage

The [client owners and daily coordinator](agent-maintenance.md) maintain material
coverage differences here. These records were reviewed against
[`ad338c4f`](https://github.com/obscuritylabs/Colossus/commit/ad338c4f8a94eed27f5ab735a433612747e9b56f)
and describe checked-in behavior and existing test evidence, not a fresh execution
of those tests or acceptance on every platform. Recheck the implementation and
active follow-ups before assigning a fix.
The documentation-guidance row was additionally reviewed against
[`8e3d39b8`](https://github.com/obscuritylabs/Colossus/commit/8e3d39b8d69f6429ef7263a03685cd549800261a).
Coverage is semantic: public Draft continuation and terminal approval/execution are
different workflows with explicit revision binding.
The suggestion, worktree, App Shot, browser sign-in and companion rows were
reviewed for [PR #354](https://github.com/obscuritylabs/Colossus/pull/354) at
[`738f311a`](https://github.com/obscuritylabs/Colossus/commit/738f311a30cf8428696c873eb177b6a64d984af6).
Their pending native checks remain separate from implementation and historical tests.

| Capability | Public API / SDK | CLI / TUI | Desktop | VS Code | Web |
| --- | --- | --- | --- | --- | --- |
| Standalone Goal creation | Supported; bounded iterations and caller tool ceiling | Supported; CLI and full-screen TUI `/goal` creation/completion acceptance in embedded and worker hosts | Supported; composer and native request validation evidence | Missing; composer exposes Plan, Execute, and Research | Supported on connected runtimes; capability and budget-bound composer |
| Revision-bound Plan workflow | Supported; source-run-bound Draft continuation | Supported; revision-bound refinement, approval, and execution | Supported; exact-revision Plan continuation | Partial; canonical Plan inspection exists, continuation controls missing | Supported; source-run and revision-bound continuation |
| Human managed-shell discovery, read, and Stop | Supported; scoped process-session requests | Unverified; dedicated human controls not evidenced by agent-tool tests | Supported; active-shell inspection and Stop | Missing; process-supervision controls unimplemented | Unverified; Fleet human terminals are a distinct workflow |
| Conditional documentation guidance | Partial; explicit skill selection and bounded reads; policy configuration belongs to the host, without a public editor | Supported through YAML policy and existing `@` / `/plugin` selection | Supported; advanced Plugins controls and validated native overrides; feature-specific browser acceptance unverified | Partial; inherits host policy; guidance settings and skill selection controls absent | Partial; connected runtimes inherit policy; discovery-only plugin UI and no composer skill selections; Hosted-specific acceptance unverified |
| Editable next-prompt suggestions | Supported: capability-gated, directly owned completed-run prediction with separate usage and durable single-attempt settlement | Supported: local default, opt-in model mode, canonical preference synchronization and draft guards | Supported: editable suggestions, settings and stale/draft guards | Missing: generated prediction SDK is available; extension composer controls are absent | Supported: local and opt-in model suggestions with draft/source guards |
| Managed local Git worktrees | Intentional difference: local Rust service owns filesystem custody; no remote worktree RPC is introduced | Supported: create/list/settings/archive/remove commands; open starts the ordinary TUI in the verified checkout | Supported: local Worktrees settings, create/open and guarded lifecycle | Missing: no human worktree controls | Intentional difference: no browser-host filesystem checkout management; remote provisioning would need a separate owned API |
| Windows App Shots to a chat draft | Intentional difference: capture authority stays with the local Desktop host; attachment transport does not grant capture authority | Intentional difference: no human shortcut/preview workflow; agent computer tools are separate | Partial: Windows capture, preview, text choice and explicit draft attachment implemented; installed signed shortcut-to-draft acceptance pending | Intentional difference: Windows Desktop-first feature; no extension capture workflow | Intentional difference: no native external-window capture shortcut or cookie/profile sharing |
| Human browser sign-in controls | Intentional difference: human native profiles and sign-ins belong to the Desktop host; existing agent browser contracts do not expose their cookies | Intentional difference: agent browser operations do not establish a human profile/settings workflow | Partial: Windows User-Agent, encrypted opt-in sign-in persistence, reset and redacted diagnostics implemented; final rebuilt-byte/platform acceptance pending | Intentional difference: no corresponding human native-profile host | Intentional difference: ordinary browser cookies remain owned by the user's browser; no Desktop profile sharing |
| Developer companion | Intentional difference: floating window and preferences are local Desktop presentation | Intentional difference: no floating native companion window | Partial: status/preferences and scoped native actions implemented; Windows border fix implemented; native visual verification pending | Intentional difference: not an extension feature | Intentional difference: browser page does not own an OS floating companion |
| Compose deployment with Chat by default | Existing Control Plane API exercised for bootstrap, authentication and retained Chat; public runtime SDK contracts unchanged | Intentional difference: one-time PowerShell initialization followed by ordinary Compose commands; native CLI/TUI runtimes remain separate Fleet enrollments; no new Hosted Chat client command | Intentional difference: separately installed client; no embedded Compose installer or new Hosted Chat transport; existing Fleet GUI acceptance unverified | Intentional difference: separately installed extension; no embedded Compose installer or new Hosted Chat transport; existing Fleet GUI acceptance unverified | Supported: default Hosted Chat with Echo, verified HTTPS and SSE through the existing web app; API smoke tested, full rendered Chat acceptance unverified |
| Provider-native document attachments | Supported: existing caller-owned artifact/run-input transport; format validation and selected-model document capability stay in the host; SDK capability metadata does not grant document authority | CLI: native `--attach` through the owned artifact path; Partial TUI: canonical document history renders, but `/attach` remains image-only | Implemented: native document attachment and model preference adapters; current-source rendered/native acceptance remains separately required | Missing: no human file attachment picker; public transport support alone is insufficient | Missing: Hosted attachment picker remains text-only; no native document workflow |

Evidence and owned follow-ups:

- **Native documents:** `documentInputs` is separate from image support and defaults
  Off; CLI setup exposes `--document-inputs off|auto|on`. Responses supports PDF,
  DOCX, XLSX and PPTX; Chat Completions and native
  Anthropic support PDF only. Codex and Echo do not support this path. Encrypted
  artifacts retain bytes; canonical messages and provider replay retain verified
  metadata, with inline resolution only inside the provider permit. This adds no
  local full document parser or provider Files upload/delete lifecycle.
  Admitted initial human input uses durable ordered public Message updates so
  attachment metadata can be reconstructed after reconnect; client drafts are not
  history authority. API/runtime owners retain canonical identity, ownership and
  append-once recovery checks.
  History consumers preserve one human turn: Desktop suppresses its local prompt
  when a durable User Message arrives; CLI/TUI use canonical transcripts; VS Code
  ignores incremental Message frames and reloads its canonical view. Web/Hosted
  retains the submitted human row and ignores the duplicate runtime User frame.
  Activity projection coalesces canonical scalar and released User records by
  session-message sequence, retaining ordered attachment labels. API/projection
  and Cloud replay/reload tests protect these boundaries; client native/GUI and
  vendor acceptance remain separately scoped.
  Shared continuation explicitly includes admitted attachment context. New branch
  copies check current source-session ownership/continuation authority even when
  the caller owns an intermediate run; an authorized independent snapshot retains
  its copied context after later revocation. Direct artifact downloads remain
  owner-bound. API/runtime and test owners retain the active/revoked branch cases.
  A durable verified branch-prefix receipt excludes copied User records from new
  input publication, including failure before the new prompt commits; missing or
  conflicting proof preserves recovery uncertainty.
  Public API execution and Plan creation/refinement accept the supported formats;
  approved-plan execution rejects new documents and uses its saved revision.
  Research and standalone Goal keep their existing text-only request contracts.
  `test_owner` owns live-provider extraction and same-session/restart evidence with
  matching source; passing loopback wire tests is not live vendor acceptance.
  `desktop_owner` and `test_owner` own rendered/native Desktop acceptance.
  `cli_owner` owns native document submission through TUI `/attach`; `test_owner`
  owns its acceptance after that adapter expands beyond images.
  `vscode_owner` owns a scoped file-picker/upload workflow and its public SDK
  acceptance; `maintenance_coordinator` owns assigning the Web/Hosted document
  workflow to its canonical API/client owners before claiming that coverage.
- **Compose:** Hosted Chat is the web workflow. `deploy/compose/test-init.ps1`,
  `deploy/compose/smoke.cjs` and `.github/workflows/compose-smoke.yml` own
  initialization and smoke checks. Acceptance uses published `v0.11.11-preview.5`
  images and checks the same thread, task and assistant response after restart.
  Compose owns lifecycle; no custom backup/restore/upgrade framework is supplied.
  This does not establish current-source container or hosted final-head acceptance.
  `test_owner`, coordinated with `desktop_owner`, `vscode_owner` and `cli_owner`,
  owns separately scoped Fleet client connection/enrollment and rendered Web Chat
  acceptance against Compose before those workflows are claimed as verified.
  The same-host anchor intentionally lacks independent administrator custody;
  production checkpoint isolation remains owned by the Helm/Hosted operator guide.
- **Suggestions:** canonical service and cancellation/transport tests in `crates/colossus-runtime/src/prompt_predictions/`, API ownership tests, TUI next-prompt/workspace-preference tests and Desktop/Web renderer tests. `vscode_owner`: expose editable opt-in suggestions through the existing SDK with source/draft/opt-out guards; final scope and acceptance remain a product decision.
- **Worktrees:** `docs/use/worktrees.md`, `crates/colossus-worktrees` lifecycle/ownership/link/Unix-private-directory tests, CLI commands and Desktop worktree renderer tests. `vscode_owner` and `maintenance_coordinator`: assess a local extension workflow and any required owned API before implementation; Web remote provisioning remains separate from this local feature.
- **App Shots:** `docs/develop/adr/0013-human-app-shots.md`, native capture tests, source acceptance and Desktop preview/draft tests. `desktop_owner` and `test_owner`: complete final-source ordinary-user conformance, signed-installed shortcut → preview → current-chat draft acceptance and original-installation restoration. Future client/platform expansion needs a separate scope decision.
- **Browser:** existing source WebView2 five-phase receipts prove their historical scope; runtime/profile tests cover policy, isolation and persistence. `desktop_owner` and `test_owner`: validate final rebuilt images with fresh owned certificate fixtures, record Windows native diagnostics and retain platform-specific limitations. Historical native bytes are not current-byte acceptance.
- **Companion:** exact capability/caller-label tests, preference/recovery tests and renderer cases. `desktop_owner` and `test_owner`: verify the newly created shadowless Windows companion; `maintenance_coordinator`: retain macOS/Linux native acceptance as unverified until measured.

- **Documentation guidance:** [runtime policy tests](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-runtime/src/documentation_policy_tests.rs),
  [Desktop settings descriptors](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src/components/ManagedSettingsPane.tsx),
  [generic settings rendering tests](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src/components/ManagedSettingsPane.test.ts),
  and [native policy validation](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-sidecar/src/managed.rs).
  `desktop_owner` and `test_owner`: add feature-specific browser acceptance for
  guidance settings. `vscode_owner`: expose Managed Local guidance settings and
  explicit skill selection through existing contracts. `maintenance_coordinator`:
  decide whether remote policy editing is an intentional host-only difference
  before assigning API work; assess Web/Hosted applicability and assign its owner.
- **Goal:** [public API tests](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-api-runtime/src/service_tests/standalone_goal.rs),
  [CLI tests](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-cli/tests/agent_smoke.rs),
  [TUI full-screen acceptance](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-cli/tests/provider_terminal_smoke.rs),
  [Desktop composer tests](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src/components/WorkComposer.test.ts),
  and [Web composer tests](https://github.com/obscuritylabs/Colossus/blob/main/apps/web/src/RunComposer.test.tsx).
  `vscode_owner`: expose bounded standalone Goal creation through the existing public
  contract and verify cancellation and capability availability.
- **Plans:** [public API conformance](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-api-runtime/src/service_tests.rs),
  [terminal workflow tests](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-cli/tests/interactive_plan_smoke.rs),
  [Desktop browser acceptance](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/tests/browser/plan-workflow.spec.ts),
  and [Web continuation tests](https://github.com/obscuritylabs/Colossus/blob/main/apps/web/src/ThreadDetail.test.tsx).
  `vscode_owner`: add continuation against the exact visible source run and Plan
  revision, including stale-revision rejection and uncertain-outcome handling.
- **Shells:** [public API conformance](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-api-runtime/src/service_tests/process_sessions.rs)
  and [Desktop browser acceptance](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/tests/browser/active-shells.spec.ts).
  `vscode_owner`: add scoped inspection/read/Stop through the public SDK.
  `cli_owner`: determine the intended human workflow and either supply operator
  controls with acceptance evidence or document a reason for the difference.
  `maintenance_coordinator`: assess the Web process-session workflow separately
  from Fleet human terminals and assign any applicable gap to its canonical owner.

The [extension's documented limitations](https://github.com/obscuritylabs/Colossus/blob/main/apps/vscode/README.md#work-in-the-editor)
also include memory, workflows, plugins, and session topology. They need individual
public-API and client-coverage assessment; this seed inventory is not exhaustive.
`maintenance_coordinator` owns triage and deduplication against existing issues/PRs before
implementation. Named role follow-ups above are repository assignments, not claims
that external tickets have been opened.

Colossus is alpha software. Configuration, protocols, and supported deployment surfaces
may change before a stable release. Compatibility retained for deployed Rust state or
protocols is documented at its owner; the frozen Python 0.5 runtime remains only on the
historical branches described in [ADR 0001](adr/0001-rust-runtime-cutover.md).
