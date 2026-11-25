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
| SQLite and embedded Turso journal evaluation | `colossus-journal-sql` | [Embedded SQL adapter evaluation](state-recovery.md#embedded-sql-adapter-evaluation) | Rust adapter and benchmark entry points. CLI, TUI, VS Code, Desktop, public API/SDK, and web use the existing runtime storage choices while the runtime storage owner evaluates adoption and migration. |
| Providers, model routing, and normalized turns | `colossus-provider`, `colossus-agent`, `colossus-runtime` | [Choose a provider](../use/providers/index.md), [Providers and models](../reference/configuration/providers-models.md) | Provider and agent suites plus CLI provider smoke tests |
| Sessions and context | `colossus-session`, `colossus-context`, `colossus-runtime` | [Sessions](../use/sessions.md), [context and snapshots](../use/sessions-context.md) | Repository, snapshot, compaction, and branch tests |
| Human conversation export | `colossus-session` indexed reader, `colossus-contracts` DTOs, `colossus-api-runtime` disclosure and `colossus-presentation` formatter; client host adapters | [Export a conversation](../use/sessions.md#export-a-conversation), [API contract](application-sdk.md#canonical-conversation-export) | Fixed-snapshot, disclosure, cursor, byte-bound, shared-formatting, staging and host-adapter tests; client browser tests cover their rendered workflows |
| Tasks, decisions, plans, goals, and subagents | `colossus-work`, `colossus-agent`, `colossus-runtime` | [Tasks, decisions, and plans](../use/tasks-decisions-plans.md), [Goals and subagents](../use/goals-subagents.md) | Lifecycle, lineage, budget, cancellation, and recovery tests |
| Memory and lexical projection | `colossus-memory` | [Memories](../use/memories.md), [Context and memory configuration](../reference/configuration/context-memory-research.md) | Canonical lifecycle, Tantivy, fallback, and recovery tests |
| Search and source-backed research | `colossus-search`, `colossus-research`, `colossus-runtime` | [Web search](../use/web-search.md), [Deep research](../use/deep-research.md) | Search adapter, evidence/citation, and CLI research tests |
| Workflows and triggers | `colossus-workflow`, `colossus-runtime` | [Workflow authoring](../extend/workflows/authoring.md), [Triggers and recovery](../extend/workflows/triggers-recovery.md) | Parser, control-flow, idempotency, compensation, and restart tests |
| Always On Agent | Runtime standing-chat journal, local authenticated worker, managed Desktop host | [Desktop schedules](../desktop/schedules.md), [CLI reference](../reference/cli.md) | Owner and revision checks, durable wake reconciliation, authenticated CLI worker smoke, and Desktop request mapping |
| Agent Plugins, integrations, standalone MCP, and release bundles | extension crates and runtime adapters | [Extend Colossus](../extend/index.md) | Upstream schema, confinement, OCI, Sigstore, registry, lifecycle, and live-adapter tests |
| Conditional documentation guidance | runtime configuration and instruction snapshots, bundled unabridged documentation reference | [Skills and plugins](../use/skills-plugins.md), [Plugin configuration](../reference/configuration/extensions.md) | Offline source-fidelity/completeness verification, documentation policy, instruction precedence, bounded multipart resource reads, and sidecar configuration tests |
| Public API, worker, SDKs, and Desktop | API, gRPC, worker, SDK crates and `apps/desktop` | [Public API and SDKs](application-sdk.md), [Colossus Desktop](../desktop/index.md) | Protocol, enrollment, parity, sidecar, SDK, and Desktop tests |
| Draft prompt suggestions | Runtime/API prediction service, worker/SDK/gRPC and Cloud adapters, `colossus-presentation`, `colossus-tui`, `apps/ui` | [Terminal UI](../use/terminal-ui.md), [Desktop work](../desktop/work.md), [Public API and SDKs](application-sdk.md) | Audited auxiliary inference, single-attempt settlement and usage isolation tests; capability and transport parity; editable-draft keyboard and lifecycle tests |
| Human App Shots and Desktop companion | `native/computer`, Desktop `desktop_extras` and renderer | [App Shots](../desktop/work.md#capture-another-windows-app), [Desktop settings](../desktop/settings.md), [ADR 0013](adr/0013-human-app-shots.md) | Synthetic Windows capture example and retained helper cleanup; preview, destination fencing and preference tests; isolated companion window capability |
| Managed local worktrees | `colossus-worktrees`, CLI and Desktop adapters | [Managed worktrees](../use/worktrees.md), [ADR 0012](adr/0012-managed-worktrees.md) | Offline repository, cross-process lease, ownership/link and cleanup tests; command-level CLI and rendered settings tests |
| Windows human browser sign-in controls | Desktop browser manager and native WebView2 adapter | [Desktop browser](../desktop/tools.md), [Browser boundary](adr/0003-desktop-browser-boundary.md) | Isolated synthetic profile restart/clear, user-agent, TLS and client-certificate acceptance harness; profile lock and redacted diagnostics tests. Agent Chromium availability remains governed separately below. |
| Hosted execution, Web Chat and managed attachments | `colossus-cloud`, `colossus-cloud-chat`, `colossus-cloud-executor`, server `hosted` modules and `apps/web` | [Hosted execution](adr/0009-hosted-execution.md), [Hosted operation](../cloud/work.md#start-a-hosted-conversation), [Cloud development](cloud-control-plane.md#develop-hosted-personal-chat) | Owner-bound migration and lease tests, HTTP attachment/isolation acceptance, shared SQL two-replica recovery and Web browser tests; arbitrary hosted workspace provisioning remains unavailable |
| Single-host Compose deployment with Chat by default | `deploy/compose`, shared `deploy/control-plane/controlled-project.ps1`, release bundle workflow | [Install with Compose](../cloud/deployment/compose.md) | Windows/Linux initialization, PKI and retained-secret checks; published-image HTTPS, authentication, Echo/SSE and retained conversation after ordinary Compose restart |
| Agent messages, durable inboxes, and incoming A2A | `colossus-communication`, agent/runtime/API/SDK crates, `apps/ui`, `colossus-a2a-server` | [ADR 0007](adr/0007-agent-communication.md), [Inspect agent messages](../use/agent-communication.md), [Host A2A](agent-communication.md) | Canonical replay and attempt tests, provider input/hash and budget tests, owner/scope/recovery tests, A2A wire tests, host checks and rendered inbox tests |
| Owned browser control and native Chromium development adapters | `colossus-contracts`, `colossus-ports`, `colossus-browser`, `colossus-browser-bridge`, `colossus-browser-presentation`, `colossus-sandbox`, runtime/worker/SDK browser adapters, `native/browser`, Desktop native adapter | [Owned Chromium browser](adr/0008-owned-chromium-browser.md#current-enablement-and-remaining-work) | Shared authority, private transport, host cleanup, artifact release and Linux profile protection tests; actual Core open/snapshot/run-finish, four-channel read-only presentation, five-host profile restart/reset and per-request PKI conformance. The Mac temporary HTTP Core diagnostic passed real page control, screenshots, Retina frames and direct-host shutdown; its complete helper/crash/HTTPS/PKI and Desktop foreground acceptance remain outstanding. Linux native NSS TLS and macOS human preview retain their own scopes. Linux direct-key-denial and Windows/broker acceptance, same-page agent-to-human transfer, encrypted/quota-bound persistent profiles and management, multi-origin human SSO and production-accepted packages remain incomplete. Windows x64 stable and preview installers select the ordinary-user functional edition; exact signed installed acceptance remains separate. Production component modes stay disabled. |
| Offline Desktop and local TUI dictation | `crates/colossus-native-dictation`, `apps/desktop/src-tauri/src/dictation.rs` | [Evaluate offline Desktop dictation](desktop-dictation.md) | Bounded model/audio/revision tests; continuous send and cancellation tests; native macOS/Windows measurements and permission acceptance still required |
| Terminal and presentation | `colossus-presentation`, `colossus-tui`, interface adapters | [Terminal UI](../use/terminal-ui.md), [Extensions and presentation](extensions-presentation.md) | Semantic document, reducer, layout, PTY, and restoration tests |
| Local terminal support reports | `colossus-cli` support report collector and native save; `colossus-tui` slash command | [Save a local support report](../admin/troubleshooting.md#save-a-local-support-report), [Report format](../reference/cli.md#local-support-report) | Early CLI dispatch, bounded allowlisted JSON, and destination safety checks |
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

Remote MCP Tasks first targets ordinary Execute runs through the local CLI and
TUI with file-backed redb. Existing tool activity shows the call as pending
and delivers its released terminal outcome. The `colossus-mcp` and runtime owners
maintain the authorized follow-up and restart receipts. The CLI owner tracks
standalone `mcp call` support. The
Desktop and VS Code owners track dedicated controls or acceptance for connected
clients. The application SDK and web owners track hosted/Fleet placement under
[#372](https://github.com/obscuritylabs/Colossus/issues/372). The first placement
uses the existing run and tool result contracts.

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
| Inspect a GitHub pull request and its head checks | The granted native operation can run through existing agent turns. A dedicated human action requires an authenticated API and SDK contract. | The CLI uses `integrations call`. Embedded and authenticated worker TUIs use `/integration call`. See [Integrations](../extend/integrations.md#inspect-a-pull-request-and-its-head-checks). | The bundled Managed Local TUI uses the shared command. Packaged acceptance is pending. The native Git inspector reads the local checkout. | A dedicated editor action requires the human API. | A dedicated web action requires the human API and a deployment that grants the relevant repository authority. |
| Local terminal support reports | Intentional difference. Collection and saving belong to the local terminal host. | CLI saves before runtime startup. The running TUI records its host mode and any last-observed worker status. | Settings has its own [Export diagnostics](../desktop/settings.md#desktop-preferences-and-support). Managed Local uses the shared TUI binary. Installed-bundle acceptance is pending. | Terminal support reports are unavailable in the extension. | Intentional difference. Hosted diagnostics require a separate service-side decision. |
| Packaged owned Chromium functional edition | Existing typed browser contracts, caller authority and capability discovery; installed edition determines availability | Standalone archives do not include a browser host; this change adds no separate CLI/TUI browser payload | Windows x64 stable and preview installers select the functional edition with sandbox and compiler-bound signed component; exact signed installed acceptance remains separate. Human WebView2 sign-in stays separate. macOS/Linux retain their existing boundaries. | No browser host or native presenter is shipped | No native browser host is shipped; the user's browser remains separate |
| Progressive Desktop thread history | Existing authenticated run discovery and bounded pages; full run reads retain their contract | Existing session browsers and history commands retain their workflows | Supported: older pages load while scrolling, thin summaries survive beyond the former 200-entry cutoff, and a virtual list bounds rendered rows; reopening an evicted native binding revalidates its source | Existing extension history workflow; no host change | Existing web history workflow; no host change |
| Unsaved TUI conversations | Intentional difference: application runs use their own server-managed session lifecycle | Supported in embedded and authenticated worker hosts. Starting work saves a draft for History, resume, and export. The CLI can also explicitly save an empty session. | Supported in the bundled Managed Local TUI. Native conversations follow the Desktop composer workflow. Packaged TUI acceptance pending. | Intentional difference: the extension owns its composer and conversation lifecycle | Intentional difference: the web composer uses application runs and their server-managed session lifecycle |
| Native session discovery and transcript paging | The app API and SDK retain their caller-scoped run queries and cursor-based transcript export. | CLI discovery and resume use indexed summaries. Embedded and worker TUIs open one message page and defer exact context and run usage. Shared journal read budgets and native host tests cover these paths. | The bundled Managed Local Terminal uses the same TUI. The native chat view uses the app API. | The extension uses caller-scoped run queries. | Web conversations use caller-scoped run queries. |
| Undo and redo unsent TUI prompt edits | Intentional difference: edit history belongs to each local composer. Submission and approval use the existing run contracts. | Supported in the shared TUI used by embedded and authenticated worker hosts. See [composer keys](../reference/tui.md#compose-and-navigate). Line-mode CLI leaves editing to the terminal. | Bundled Managed Local TUI uses the shared composer. Packaged key delivery remains unverified. The native chat composer owns its editing behavior. | Intentional difference: the extension owns its editor input | Intentional difference: the web composer owns its browser input |
| Human conversation export | Supported: optional `conversations.transcript.v1` read-only paging through an authorized source run; broader `SessionService` remains unhosted | Supported: `sessions export`, `/session export` and `/export` in embedded and worker hosts | Implemented: native Save and bounded Copy with progress/cancellation; installed native Save/clipboard acceptance pending | Missing: no human export command | Implemented: full Hosted/Fleet export with bounded download/Copy and authorized live source; loaded-history actions remain the offline excerpt fallback |
| Copy rendered terminal text | Intentional difference: clipboard authority belongs to interactive terminal hosts | Supported: standalone local Windows TUI confirms a native clipboard write. Remote Windows and other platforms request terminal clipboard delivery. Line-mode CLI uses terminal selection. | Intentional difference: F6 enables terminal-native selection followed by the Desktop Copy shortcut | Intentional difference: the extension has its own conversation UI and the editor owns its native selection | Intentional difference: Fleet terminal output carries no clipboard authority |
| Standalone Goal creation | Supported; bounded iterations and caller tool ceiling | Supported; CLI and full-screen TUI `/goal` creation/completion acceptance in embedded and worker hosts | Supported; composer and native request validation evidence | Missing; composer exposes Plan, Execute, and Research | Supported on connected runtimes; capability and budget-bound composer |
| Always On check-ins in an owned local conversation | Standing-chat controls use authenticated local worker IPC. Desktop's inventory reads status from owner-scoped running workers. Wake runs use the existing owned run API. | CLI enrollment, status, pause, and manual wake. TUI opens the same session using its ordinary session selector. | Supported for Managed Local: enrollment and wake controls, cross-Workspace inventory with unavailable status, and a single enrolled thread above Pinned. Fleet uses its existing remote runtime workflows. | Missing: no Always On controls. | Missing: no Always On controls. |
| Read-only live tail of an open background conversation | Existing transcript and ordered run-feed contracts supply released messages and status. Pinning is local presentation state. | Full-screen TUI supports one same-workspace, same-runtime companion. Intentional difference for one-shot CLI and inline TUI: both keep a single transcript and use ordinary inspection or terminal scrollback. | Partial: live Asides support separate follow-up conversations. Pinning a peer conversation is missing. | Missing: the extension follows the selected conversation. | Partial: Side Chat shows a related conversation alongside its parent. Pinning an arbitrary open peer is missing. |
| Anthropic tool replay recovery | Ordinary Execute can continue a verified oversized replay using saved tool evidence. The keyless missing-checkpoint path also remains available. | Existing CLI resume and TUI continuation use the same runtime recovery. | Managed Local inherits protected replay overflow recovery. | Existing Execute conversations inherit the runtime behavior. | Connected runtimes inherit the ordinary Execute recovery. |
| Transient provider HTTP recovery | The provider owns bounded retry. Run updates expose attempt, limit, and HTTP status through the existing SDK contract. | CLI uses runtime retry. TUI shows progress and offers explicit conversation continuation after eligible failures. | The active conversation shows progress and offers Retry for an eligible Execute failure in the saved session. Installed acceptance is unverified. | Shows the retry count. A dedicated Retry control is missing. | Connected runtimes use provider retry. Dedicated progress and Retry controls are missing. |
| Revision-bound Plan workflow | Supported; source-run-bound Draft continuation | Supported; revision-bound refinement, approval, and execution | Supported; exact-revision Plan continuation | Partial; canonical Plan inspection exists, continuation controls missing | Supported; source-run and revision-bound continuation |
| Human managed-shell discovery, read, and Stop | Supported; scoped process-session requests | Unverified; dedicated human controls not evidenced by agent-tool tests | Supported; active-shell inspection and Stop | Missing; process-supervision controls unimplemented | Unverified; Fleet human terminals are a distinct workflow |
| Conditional documentation guidance | Partial; explicit skill selection and bounded reads; policy configuration belongs to the host, without a public editor | Supported through YAML policy and existing `@` / `/plugin` selection | Supported; advanced Plugins controls and validated native overrides; feature-specific browser acceptance unverified | Partial; inherits host policy; guidance settings and skill selection controls absent | Partial; connected runtimes inherit policy; discovery-only plugin UI and no composer skill selections; Hosted-specific acceptance unverified |
| Editable next-prompt suggestions | Supported: capability-gated, directly owned completed-run prediction with separate usage and durable single-attempt settlement | Supported: local default, opt-in model mode, canonical preference synchronization and draft guards | Supported: editable suggestions, settings and stale/draft guards | Missing: generated prediction SDK is available; extension composer controls are absent | Supported: local and opt-in model suggestions with draft/source guards |
| Managed local Git worktrees | Intentional difference: local Rust service owns filesystem custody; no remote worktree RPC is introduced | Supported: create/list/settings/archive/remove commands; open starts the ordinary TUI in the verified checkout | Supported: local Worktrees settings, create/open and guarded lifecycle | Missing: no human worktree controls | Intentional difference: no browser-host filesystem checkout management; remote provisioning would need a separate owned API |
| Windows App Shots to a chat draft | Intentional difference: capture authority stays with the local Desktop host; attachment transport does not grant capture authority | Intentional difference: no human shortcut/preview workflow; agent computer tools are separate | Partial: Windows capture, preview, text choice and explicit draft attachment implemented; installed signed shortcut-to-draft acceptance pending | Intentional difference: Windows Desktop-first feature; no extension capture workflow | Intentional difference: no native external-window capture shortcut or cookie/profile sharing |
| Human browser sign-in controls | Intentional difference: human native profiles and sign-ins belong to the Desktop host; existing agent browser contracts do not expose their cookies | Intentional difference: agent browser operations do not establish a human profile/settings workflow | Partial: Windows User-Agent, encrypted opt-in sign-in persistence, reset and redacted diagnostics implemented; final rebuilt-byte/platform acceptance pending | Intentional difference: no corresponding human native-profile host | Intentional difference: ordinary browser cookies remain owned by the user's browser; no Desktop profile sharing |
| Developer companion | Intentional difference: floating window and preferences are local Desktop presentation | Intentional difference: no floating native companion window | Partial: status/preferences and scoped native actions implemented; Windows border fix implemented; native visual verification pending | Intentional difference: not an extension feature | Intentional difference: browser page does not own an OS floating companion |
| Compose deployment with Chat by default | Existing Control Plane API exercised for bootstrap, authentication and retained Chat; public runtime SDK contracts unchanged | Intentional difference: one-time PowerShell initialization followed by ordinary Compose commands; native CLI/TUI runtimes remain separate Fleet enrollments; no new Hosted Chat client command | Intentional difference: separately installed client; no embedded Compose installer or new Hosted Chat transport; existing Fleet GUI acceptance unverified | Intentional difference: separately installed extension; no embedded Compose installer or new Hosted Chat transport; existing Fleet GUI acceptance unverified | Supported: default Hosted Chat with Echo, verified HTTPS and SSE through the existing web app; API smoke tested, full rendered Chat acceptance unverified |

Evidence and owned follow-ups:

- **GitHub pull request checks.** `colossus-integrations` owns the read operation
  and its evidence bounds. The authorized connection and runtime policy govern
  access. The CLI fixture in
  `crates/colossus-cli/tests/native_integrations_smoke/github_pull_request.rs`
  exercises exact-head, stale, and incomplete evidence. The PTY fixture in
  `crates/colossus-cli/tests/integrations_tui.rs` exercises both TUI hosts.
  `desktop_owner` and `test_owner` own acceptance in the packaged Managed Local
  TUI. `maintenance_coordinator` owns triage of a dedicated human API and the
  applicable Desktop, VS Code, and web workflows.

- **Local terminal support reports:** `crates/colossus-cli/src/support_report.rs`
  owns the report and its save operation. `crates/colossus-cli/tests/support_report.rs`
  exercises a save before runtime startup. TUI tests in
  `crates/colossus-tui/src/support_report/tests.rs` cover destination parsing,
  cancellation, and preservation of conversation state. The `support_report_tui`
  CLI test target covers saving through a real terminal in embedded and
  authenticated worker modes, including an observed disconnect. `desktop_owner`
  and `test_owner` own acceptance through the TUI in a newly packaged Managed
  Local installation.
  `vscode_owner` owns assessing a local extension-host adapter and
  `maintenance_coordinator` owns triaging any Hosted diagnostics workflow under
  [issue #388](https://github.com/obscuritylabs/Colossus/issues/388).
- **Unsaved TUI conversations:** The [Sessions guide](../use/sessions.md#start-a-clean-conversation)
  describes when the TUI saves a draft. The [Terminal UI guide](../use/terminal-ui.md#write-and-review-a-turn)
  explains how sandbox acknowledgement affects settings saves and prompt history.
  Regressions in `crates/colossus-cli/src/tui_host/preferences_concurrency_tests.rs`
  exercise both embedded and authenticated worker hosts, including idle launches,
  a scoped settings save, rejected empty input, history ordering around work
  acknowledgement, retry with the same ID, and exact resume. `desktop_owner` and
  `test_owner` own acceptance through the bundled TUI in the installed Managed Local
  application.
- **TUI draft edit history:** The [TUI key reference](../reference/tui.md#compose-and-navigate)
  defines the text and paste workflow. `colossus-tui` owns the per-session history
  and retains a paste payload with its exact marker range. Composer tests cover
  text, cursor, paste binding, and history bounds. The `pty_history` target covers
  terminal key delivery. `desktop_owner` and `test_owner` own packaged Managed
  Local terminal acceptance.
- **Windows functional browser:** [Release packaging](releasing.md#developer-previews-and-desktop)
  and [ADR 0008](adr/0008-owned-chromium-browser.md) distinguish application channel
  from browser authority. `scripts/ci/browser-release-policy.test.ps1` exercises
  the actual stable/preview signing, unsigned validation and compiler-seal
  admission guards; `scripts/ci/desktop-contract.test.mjs` covers release step
  ordering and explicit selection. Sidecar `windows_browser_release_policy`
  tests cover the shared Release/channel/development admission rule. These fixtures
  establish source contracts, not installed browser acceptance. `desktop_owner`
  and `test_owner` own exact signed installed input, screenshots, background/restore
  and cleanup acceptance; `ci_cd_owner` owns publisher evidence. `cli_owner`,
  `vscode_owner` and `maintenance_coordinator` retain any future client distribution
  assessment; no additional client payload is established by these Windows changes.
- **Conversation export:** The [canonical guide](../use/sessions.md#export-a-conversation)
  and [SDK contract](application-sdk.md#canonical-conversation-export) own behavior.
  `crates/colossus-session/src/transcript/tests.rs` covers indexed work bounds,
  original text and multipart order, private/peer filtering, fixed appends, archive
  and reader recreation. `crates/colossus-api-runtime/src/service_tests/transcript.rs`
  covers original-session disclosure, revocation and caller/source-bound cursors;
  gRPC and SDK tests cover wire fidelity and capability discovery.
  `crates/colossus-presentation/src/transcript_export/tests.rs` covers all formats,
  completion, bounds and safe metadata. CLI smoke tests in `agent_smoke.rs` and
  `worker_smoke.rs` cover the command in embedded/worker hosts; interactive and
  staging tests cover quoted paths, cancellation and no-clobber publication.
  Desktop native `conversation_export/tests.rs` covers full-source paging,
  substituted/changed snapshots, bounded Copy and destination collisions;
  `apps/desktop/tests/browser/conversation-export.spec.ts` covers format choices,
  cancellation, stale results, keyboard focus and compact themed layouts.
  Cloud server `transcript_export/tests.rs` and transport authorization tests cover
  bounds and source selection; `apps/web/src/transcript-export.test.ts` and
  `apps/web/tests/browser/transcript-export.browser.ts` cover complete response
  handling, formats, cancellation and keyboard/layout behavior. These renderer
  fixtures do not establish installed native or deployed full-stack acceptance.
  `desktop_owner` and `test_owner` own installed native Save/clipboard acceptance;
  `test_owner` owns deployed Hosted/Fleet end-to-end acceptance. `vscode_owner`
  owns a future export command through the public read capability, including
  safe destination handling and explicit failures. No extension workflow is
  established by generated contracts alone.
- **Desktop thread history:** The [Desktop guide](../desktop/work.md#return-to-a-conversation)
  describes progressive paging and retry. Native adapter tests in
  `apps/desktop/src-tauri/src/conversation_runs/revalidation_tests.rs` cover bounded
  source recovery and thin history output. Renderer regressions cover paging,
  metadata retention, and routing; `apps/desktop/tests/browser/thread-history.spec.ts`
  exercises long histories and a bounded rendered window. `desktop_owner` and
  `test_owner` own native packaged acceptance. Renderer fixtures and native adapter
  tests establish their stated contracts; installed application startup remains a
  separate platform check.
  `desktop_owner` and the public API owner retain the follow-up for deep persisted
  search and archived results without a reusable discovery receipt; released
  history can remain read-only when bounded ownership checks are inconclusive.
- **Provider HTTP recovery:** The [provider retry policy](../reference/configuration/providers-models.md#automatic-provider-retries)
  and [Desktop recovery guide](../desktop/work.md#recover-from-a-provider-error)
  describe the shared retry and Desktop continuation. The provider retry tests in
  `crates/colossus-provider/src/retry_tests.rs` exercise the HTTP response path.
  The Desktop timeline tests in `apps/desktop/src/components/RunTimeline.test.ts`
  cover retry progress presentation. Exact-source acceptance for HTTP 500 and
  Desktop's manual control is pending. `desktop_owner` and `test_owner` own
  installed Desktop acceptance. `vscode_owner` tracks the extension's manual
  recovery control, and `maintenance_coordinator` tracks Web recovery presentation.
- **Pinned TUI transcript:** [Sessions](../use/sessions.md#follow-another-open-session)
  and [TUI keys](../reference/tui.md#workspace-sessions) document F2 and the compact
  fallback. `crates/colossus-tui/src/workspace/tests/companion_tests.rs` covers
  viewport and draft preservation, a waiting decision, archival, and late
  events. `crates/colossus-cli/tests/concurrent_sessions_tui.rs` exercises the
  rendered workflow with the embedded runtime and an authenticated worker.
  `desktop_owner` and `vscode_owner` own assessing equivalent peer-conversation
  controls using the existing feed. `maintenance_coordinator` owns evaluating
  that workflow for Web Chat.
- **TUI copy:** Selection tests exercise mouse drag and both copy chords against
  a stub clipboard. The PTY selection fixture verifies exact OSC 52 delivery and
  the request status. The Windows native fixture checks Unicode readback and
  rejection of invalid data using its own window station and clipboard. The
  Windows Fleet PTY fixture checks that a shell inherits terminal-managed
  clipboard routing from the ConPTY launcher shared with Desktop.
  `cli_owner` and `test_owner` own release-binary paste acceptance.
  [TUI commands and keys](../reference/tui.md#compose-and-navigate)
- **Always On:** Runtime tests cover stable wake identity, schedule advancement,
  owner checks, and pause. The CLI worker smoke covers manual wake and continuation
  in one session. The Desktop browser fixture exercises inventory navigation and
  conversation controls through a mocked native transport. `desktop_owner` owns
  packaged Desktop acceptance. `cli_owner` owns follow-up TUI controls.
  `maintenance_coordinator` owns any later evaluation of extension or web
  enrollment and whether it requires a distinct authority.
  [Desktop workflows and schedules](../desktop/schedules.md#find-enrolled-conversations)
- **Anthropic replay recovery:** Runtime tests in
  `crates/colossus-runtime/src/anthropic_recovery_tests.rs` cover keyless replay
  recovery, protected restart, an oversized signed replay, and a saved exchange
  whose canonical content also exceeds the budget. The requests preserve current
  guidance, offer no tools during recovery, and allow ordinary tool access on a
  later turn. Plan, Goal, workflow, and delegated runs keep their exact replay
  requirements. Client owners and `test_owner` own rendered and connected client
  acceptance for this behavior.
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
  [offline snapshot tests](https://github.com/obscuritylabs/Colossus/blob/main/scripts/tests/test_google_style_guide.py),
  [Desktop settings descriptors](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src/components/ManagedSettingsPane.tsx),
  [generic settings rendering tests](https://github.com/obscuritylabs/Colossus/blob/main/apps/desktop/src/components/ManagedSettingsPane.test.ts),
  and [native policy validation](https://github.com/obscuritylabs/Colossus/blob/main/crates/colossus-sidecar/src/managed.rs).
  The bundled reference supplies the complete captured guide text through the same
  host-owned resource reader for CLI, TUI, Desktop, API/SDK, VS Code, and connected
  Web runtimes. Ordered parts preserve reader limits; external image files retain
  upstream URLs and alt text. Snapshot tests verify source hashes, navigation/link
  coverage, lossless article reconstruction, annotations, and JSON output budgets.
  The existing client-control gaps in the table remain; this content replacement
  adds no client settings or transport surface. Runtime and rendered acceptance
  still require their normal gates and do not follow from snapshot verification.
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
