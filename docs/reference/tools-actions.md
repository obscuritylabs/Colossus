---
title: Tools and action classes
description: Built-in tool families, effect boundaries, and access behavior.
audience: developer
type: reference
---

# Tools and action classes

`config effective` is the authority for all active and hidden candidates.
`tools list` is the authority for the current model-visible catalog, JSON Schemas,
source, family, action class, effect identity, decision, prerequisites, mutation labels,
and output bounds.

| Family | Tools | Boundary |
| --- | --- | --- |
| Utility | `echo`, `user.ask`, `tool.search`, `trace.show` | Pure; `user.ask` requires an interactive interface |
| Filesystem | `filesystem.list`, `filesystem.read`, `filesystem.search`, `filesystem.write`, `filesystem.replace` | Declared canonical roots under isolation; exact host paths under ambient authority; reads quarantined and writes atomic |
| Git and process | `git.status`, `git.diff`, `git.show`, `shell.run` | Normally an exact executable, workspace cwd, isolated environment, and enforced resource limits; acknowledged `danger_full_access` uses ambient host resources with supervised timeout/output and best-effort Unix detached-descendant cleanup/accounting |
| Patch | `patch.preview`, `patch.apply`, `patch.reverse` | Preview read; apply/reverse write; declared roots or ambient host paths |
| Trace export | `trace.export` | Bounded metadata-only write; workspace-confined under isolation and host-wide under ambient authority |
| Repository context | `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary` | Workspace-confined under isolation; absolute and traversing host paths accepted under ambient authority |
| Sessions | `session.set_title` | Updates the current session's canonical title through the effect gateway; no session ID is accepted from the model |
| Tasks | `task.create`, `task.update`, `task.list` | Canonical session work |
| Decisions | `decision.create`, `decision.update`, `decision.list`, `decision.archive`, `decision.supersede` | Binding canonical decisions |
| Plans | `plan.create`, `plan.update`, `plan.show`, `plan.approve_request` | Session-scoped, revision-aware lifecycle; the update target is bound by the runtime |
| Goals | `goal.show`, `goal.update` | Active goal lineage only |
| Subagents | `agent.delegate`, `agent.result`, `agent.list` | Durable child jobs; recursive delegation denied |
| Agent messages | `agent.participants`, `agent.send_message`, `agent.inbox`, `agent.await_message` | Journal-backed exact attempts; parent/child edges, bounded peer text and waits, ordinary policy/disclosure release |
| Memories | `memory.create`, `memory.update`, `memory.list`, `memory.search`, `memory.archive`, `memory.supersede` | Canonical lifecycle; retrieval post-gated |
| Context | `context.show`, `context.compact`, `context.snapshots`, `context.restore` | Encrypted immutable snapshots |
| Plugins | `plugin.list`, `plugin.inspect`, `plugin.skill.read`, `plugin.resource.list`, `plugin.resource.read` | Bounded metadata, selected Agent Skill instructions, and contained resources from the run snapshot |
| Search and fetch | `web.search`, `web.fetch`, `docs.fetch`, `network.http` | Search needs an explicit route; generic fetch needs host activation plus declared or ambient HTTP(S) authority; quarantined output |
| Browser | `browser.open`, `browser.status`, `browser.tabs`, `browser.tab.open`, `browser.tab.select`, `browser.tab.close`, `browser.navigate`, `browser.back`, `browser.forward`, `browser.reload`, `browser.stop`, `browser.snapshot`, `browser.screenshot`, `browser.click`, `browser.upload`, `browser.download`, `browser.fill`, `browser.select`, `browser.press`, `browser.scroll`, `browser.wait`, `browser.close` | Owned run-scoped sessions; exact origin ceilings, generation-bound control and fresh document/element handles; unavailable without an accepted host backend |
| MCP | `mcp.servers`, `mcp.search`, `mcp.tools`, `mcp.call` | Configured stdio or Streamable HTTP servers and exact-name or star-pattern tool allowlists |
| Integrations | Connected operation names | Configured, trusted, and selected only |
| Workflows | `workflow.definition.list`, `workflow.definition.get`, `workflow.schedule.list`, `workflow.schedule.get`, `workflow.schedule.create`, `workflow.task.schedule`, `workflow.schedule.set_enabled`, `workflow.schedule.delete` | Registered hash-pinned definitions; caller-owned calendar/interval workflow schedules and plain-language tasks; persistent mutations use policy, review, one-use permits, and quarantined results |

Schedule create and enabled-state control are Administration actions. Both require
approval under Allow all and Development defaults, including initially paused creation
and disable. Reads remain Read actions. Exact overrides and external policy retain
their authority. Risk auto does not automatically approve persistent schedule controls.
The strict tools accept no owner, application, Workspace, session, or run provenance;
the host binds these from active authenticated run and delegation evidence. Application
scopes remain an independent requirement. Schedule fields are immutable except enabled
state, and controls require the canonical revision returned by an authorized read.
Agent schedule input snapshots are limited to 48 KiB so the complete immutable inputs
fit in the approval review.
`workflow.definition.read` is the Read action shared by registered-definition listing
and inspection.
`workflow.definition.register` is an Administration action for validated definition
registration; Desktop operators use the separately scoped authenticated import API.
`workflow.run.read` and `workflow.run.start` describe independent workflow-run
inspection and allocation; their authenticated API scopes are distinct from chat runs.

Every tool schema denies unknown fields. Tool availability does not imply permission.
The access profile and exact overrides decide visibility and the built-in decision;
policy, approval, trust, the Safety Kernel, permits, sandbox obligations, quarantine, and
post-effect release remain independent.

`repo.file_summary` applies both the requested line ceiling and a 64 KiB serialized
result ceiling. Its preview and structural-hint collections are byte-bounded before
they enter durable tool history; `preview_truncated` is true when either the line or
encoded-byte ceiling was reached. This keeps generated or minified long lines from
consuming an entire model input budget.

## Browser tools

The first-party browser catalog is a runtime foundation. Shipping CLI and Desktop
composition leave browser automation unavailable by default. An explicitly included
browser tool remains hidden when the host has no accepted backend; access overrides
cannot supply one. A backend exposes only its supported actions and presentation
modes. Desktop's embedded Chromium developer preview does not enable these runtime
tools. The remaining native integration and release gates are recorded in the
[owned Chromium browser ADR](../develop/adr/0008-owned-chromium-browser.md).

Each browser tool uses its exact tool name as both effect action and capability.
`browser.status`, `browser.tabs`, `browser.snapshot`, `browser.screenshot`, and
`browser.wait` are Read actions. `browser.tab.select`, `browser.tab.close`, `browser.stop`, and `browser.close`
are Local state actions. All remaining browser tools are External network actions,
including key input and scrolling, which can invoke page handlers. All outputs pass
through quarantine and post-effect release. Ordinary observations have a 64 KiB
ceiling; screenshot and download tool output ceilings are 4 MiB.

`browser.open` takes `mode` (`embedded` or `headless`), `allowed_origins` (one to 32
unique HTTP(S) origins), and an optional `initial_url`. Requested origins narrow the
configured sandbox and run authority; they grant no additional network access.
URLs are limited to 4096 UTF-8 bytes and reject user information. Origins contain
no path other than an optional trailing slash, query, or fragment.

The optional `profile` is a closed selector: `{"kind":"temporary"}` (the default)
or `{"kind":"workspace","id":"bp_…"}` for an existing opaque native profile.
The ID has 32 lowercase hexadecimal digits after `bp_`; native admission checks its
authenticated workspace/application ownership and exclusive lease. It never accepts
a cache path or personal browser profile. Workspace persistence is diagnostic-only
and rejected by production composition until encrypted storage and disk quota are
accepted. Profile creation, listing and reset have no model or native SDK command.

The host derives actor, application, conversation, workspace, and run ownership.
Model arguments carry only opaque returned browser handles. `session_id`, `tab_id`,
`document_id`, `snapshot_id`, and `element_id` use the respective prefixes `bs_`,
`bt_`, `bd_`, `bn_`, and `be_`, followed by 32 lowercase hexadecimal digits.
`control_generation` is a positive integer returned by the host. It must be current;
control transfer, cancellation, expiry, and uncertain effects invalidate old control.

| Tools | Required arguments beyond the action-specific fields |
| --- | --- |
| `browser.status`, `browser.tabs` | `session_id` |
| `browser.close`, `browser.tab.open`, `browser.tab.select`, `browser.tab.close` | `session_id`, `control_generation`; tab select/close also require `tab_id` |
| `browser.navigate`, `browser.back`, `browser.forward`, `browser.reload`, `browser.stop`, `browser.snapshot`, `browser.screenshot`, `browser.press`, `browser.scroll`, `browser.wait` | `session_id`, `control_generation`, `tab_id`, `document_id` |
| `browser.click`, `browser.upload`, `browser.download`, `browser.fill`, `browser.select` | The document arguments plus fresh `snapshot_id` and `element_id` |

`browser.tab.open` accepts an optional `url`; `browser.navigate` requires `url`.
`browser.snapshot` requires `max_nodes` from one to 1024. `browser.fill` requires
`text` bounded to 8192 UTF-8 bytes. It refuses recognized password, credential, and
other secret fields; protected native entry is a separate host concern.
`browser.select` requires one to 32 unique `values`, each bounded to 1024 UTF-8 bytes.

`browser.press` requires one `key`: `enter`, `tab`, `escape`, `backspace`, `delete`,
`arrow_up`, `arrow_down`, `arrow_left`, `arrow_right`, `home`, `end`, `page_up`,
`page_down`, or `space`. `browser.scroll` requires integer `x` and `y`, each from
-10000 to 10000. `browser.wait` requires `timeout_ms` from one to 30000 and a
`condition` of either `{"kind":"load"}` or
`{"kind":"element_visible","element":{"document_id":"…","snapshot_id":"…","element_id":"…"}}`.
Element references must belong to the current tab, document, and snapshot.

`browser.screenshot` captures the current viewport into a PNG of at most 4 MiB.
Actual PNG bytes remain private until mandatory post-effect policy permits release.
The result contains verified owner-only artifact metadata, and supported model
continuations consume a typed image reference. No path or encoded bytes appear in
its arguments or JSON result; private transfer chunks never exceed 64 KiB. Screenshot
availability requires installed native capture evidence and a trusted artifact publisher.

`browser.upload` requires an `artifact_id` of `artifact-` followed by 64 lowercase
hexadecimal digits, plus a fresh ordinary file-input element. Trusted composition
resolves an authorized nonempty RunInput or RunOutput artifact of at most 4 MiB
and checks its complete bytes before policy permits website input. Private-key and
certificate-store formats are refused. There is no path or encoded payload argument.

`browser.download` requires a fresh link element and accepts no URL, save path or
destination argument. Native code owns the current link URL and private staging;
redirects remain within the immutable origin envelope. The complete file of at most
4 MiB enters post-effect policy before an owner-bound RunOutput artifact is released.
Empty downloads are supported. Unsolicited page downloads and save dialogs remain
blocked. Released download artifacts can be upload inputs under fresh authorization;
private ordered transfer chunks never exceed 64 KiB.

Browser schemas reject unknown fields and expose no JavaScript execution, raw CDP,
engine endpoint, certificate/key material, or password input. Profile management
and cross-run attachment have no model tool in this catalog. Same-page return from
agent to human control is not implemented; terminal cancellation and run completion
close the owned native context.
An unknown effect is not retried automatically; use released status and the
current generation to recover or close an owned session. Run completion supervises
native cleanup.

## Plan Mode catalog and lifecycle actions

Plan Mode narrows the already-resolved tool catalog; it never widens access. A Create
turn exposes `plan.create`, while an Update turn exposes `plan.update`. The latter schema
contains only replacement content and steps: the runtime binds the exact plan ID and
expected revision, so the model cannot redirect the write.

The remaining Plan Mode allowlist is:

- `echo`, `tool.search`, `session.set_title`, and interactive `user.ask`;
- `filesystem.list`, `filesystem.read`, `filesystem.search`, `git.status`, `git.diff`,
  `git.show`, `repo.map`, `repo.symbol_search`, `repo.references`,
  `repo.file_summary`, and `patch.preview`;
- `context.show`, `context.snapshots`, `plugin.resource.read`,
  `task.list`, `decision.list`, `plan.show`, `memory.list`, `memory.search`,
  `agent.result`, and `agent.list`.

Normal access resolution and prerequisites can remove entries from that list. Plan Mode
never offers filesystem writes, patch application, command/process execution, approval,
networking, delegation, task creation or updates, plan execution, or plan discard.

`plan.discard` is an operator-only Local State action rather than a model tool.
`plan.approve_request` remains Administration. Direct execution and approved-plan Goal
handoff both use the `plan.execute` Execution action. Update, discard, approval, and
execution all cross the ordinary effect gateway; terminal commands do not bypass access,
policy, approval, permits, or audit.

## `shell.run`

Published CLI and Desktop builds include a pinned `rg` for command searches. When
`shell.run` has execute authority for that exact file, `rg` resolves to the managed
copy before an ambient executable. Its presence does not change the approval and
sandbox rules for `shell.run`. For ordinary workspace search, `filesystem.search`
remains available without process execution, including in Plan Mode. It accepts
regular expressions by default, an optional file glob, and a result limit.
Workspace searches and `repo.map` honor repository ignore rules. Use
`filesystem.search` for arbitrary code or text matches; `repo.symbol_search` only
matches literal substrings in structural declarations. Source and debug Desktop
builds do not stage the release ripgrep binary, so use `filesystem.search` there unless an
executable has been explicitly configured.

For direct command searches in a published build, pass `argv` so the tool
resolver selects the managed executable without relying on a shell `PATH`.
The isolated Windows shell has a restricted `PATH`, so `command: "rg ..."` can
fail even when the exact bundled executable is granted:

```json
{"argv":["rg","-n","load_plugin|discover_plugins","crates/colossus-plugins"],"justification":"Find the plugin loader implementation."}
```

The equivalent `filesystem.search` call works without process execution:

```json
{"pattern":"load_plugin|discover_plugins","path":"crates/colossus-plugins","glob":"**/*.rs","max_matches":50}
```

The model-visible tool description includes the host operating system before the
agent's first command. It is a hint for native execution; a configured OCI container
may use a different OS, and command syntax still depends on the selected shell.

`timeout_ms` is an execution deadline, including supervised cleanup. Its model-visible
maximum follows the selected workspace's `sandbox.timeoutMs`, which defaults to fifteen
minutes. Omission uses the policy ceiling; an explicit request may narrow it. The
`max_output_bytes` argument is bounded by both the sandbox and the tool's 1 MiB ceiling.
A stricter request-time policy remains authoritative. See
[Sandbox resource limits](configuration/sandbox.md#resource-limits).

### Managed shell sessions

Set `yield_time_ms` to return a tracked process handle after a short wait. The default
wait is 10 seconds when `lifetime` is supplied; each wait is limited to 30 seconds.
Commands without either field retain synchronous behavior.

```json
{"command":"cargo test","justification":"Verify the requested changes.","yield_time_ms":1000,"lifetime":"run"}
```

Use `lifetime: "workspace"` explicitly for a development server or other process that
must continue across later conversation turns. Keep the server in the foreground
inside its managed session. Shell `&` or `nohup` does not create a managed lifetime.
Wait until the response reports `running` before ending the initiating turn; a launch
still awaiting authorization or startup is cancelled when that run ends.

```json
{"command":"npm run dev -- --host 127.0.0.1","justification":"Serve the requested local preview.","yield_time_ms":1000,"lifetime":"workspace"}
```

| Tool | Behavior |
| --- | --- |
| `shell.wait` | Takes `session_id`, an optional exclusive `after_sequence`, and `yield_time_ms` from 0 to 30000. Returns on output, status change, or the wait bound. |
| `shell.read` | Reads current state and new released output immediately. |
| `shell.list` | Lists up to 100 sessions belonging to this application, conversation, and agent lineage; use `after` to continue. |
| `shell.stop` | Idempotently requests stop. `stopping` is not proof of termination; read or wait for a terminal state. |

Reads and waits accept `max_output_bytes` from 16384 to 65536. Responses contain
`session`, ordered `chunks`, `next_sequence`, and `gap`; pass `next_sequence` as the
next `after_sequence`. Status distinguishes `starting`, `running`, `stopping`,
`exited`, `stopped`, `timed_out`, `failed`, `interrupted`, and `outcome_unknown`.
Only a confirmed exit code can establish command success. Output may be truncated by
the policy ceiling or the 64 KiB/256-chunk retained log window. Restart discards the
in-memory logs and reports a gap.

Run-owned sessions are stopped when their run finishes, fails, or is cancelled.
Workspace sessions survive turns after reaching `running`, until they exit, are
stopped, reach the original execution deadline, or their runtime shuts down. Neither
background lifetime nor waiting raises `sandbox.timeoutMs`. Increase that explicit
workspace ceiling for servers that need more than the default fifteen minutes.
Managed sessions reserve concurrency through cleanup. `sandbox.maxConcurrency` limits
each actor/run independently and defaults to one. A separate workspace capacity of
32 active sessions bounds total resource use across runs and applications.

Desktop's **Active shells** tool shows the selected runtime's released logs, status,
origin, and deadline, with search, follow, copy, and Stop. Active shells retain their
Managed Local runtime when switching workspaces. Quitting Desktop stops Managed Local
shells; disconnecting from an External target leaves that runtime in charge. This is
a log viewer, with no interactive stdin or PTY authority. Network and listener
permissions remain those of the selected sandbox; background lifetime adds none.

`shell.run` accepts exactly one invocation form:

```json
{"command":"cargo test -p colossus-runtime --lib","justification":"Check runtime tests for regressions in the changed code.","cwd":".","timeout_ms":120000}
```

```json
{"argv":["git","status","--short"],"justification":"Identify existing workspace changes before editing.","cwd":"."}
```

`command` is the recommended form for a bounded non-interactive script. Colossus
selects the trusted shell supplied by `workspace-development` or one explicit shell
grant and invokes it without startup profiles. `argv` preserves exact execution and
requires its first entry to resolve to one configured or derived executable. Shell
wrappers used in `argv` cannot request login, interactive, or startup-profile behavior.

Both forms require `justification`, a nonblank plain-text task purpose of at most
512 Unicode characters, without control or bidirectional characters. Do not include
credentials, hidden reasoning, or claims of authorization. This field is required even
when policy or the approval mode permits execution without an interactive prompt. An
invalid explanation executes nothing and can be corrected within normal turn limits.

Under a configured isolating boundary, `cwd` remains inside the canonical workspace.
Colossus supplies an isolated `HOME`/temp directory and sanitized absolute `PATH`;
model arguments cannot override those names or proxy variables. Under acknowledged
danger full access, the working directory may be any existing host directory,
executables resolve through ambient `PATH`, and the child receives ambient environment
and networking. Output and a maximum of 64 observed proxy origins are quarantined
before release.

Under `development`, execution remains approval-required. `workspace-development`
supplies resources but never changes that action decision.

## Tool-to-action exceptions

Most effectful built-ins use the same exact tool and action name. These are the
exceptions operators need when writing action overrides:

| Tool | Effect action |
| --- | --- |
| `echo`, `user.ask`, `tool.search`, `trace.show`, `mcp.servers` | None; pure tool |
| `filesystem.replace` | `filesystem.write` |
| `agent.delegate` | `subagent.create` |
| `agent.result` | `subagent.read` |
| `agent.list` | `subagent.list` |
| `agent.participants`, `agent.inbox`, `agent.await_message` | `agent.message.read` |
| `agent.send_message` | `agent.message.send` |
| `web.fetch`, `docs.fetch`, `network.http` | `network.http` |
| `mcp.search`, `mcp.tools` | `mcp.tools` |
| `mcp.call` | `mcp.call` |

Connected integration operations use their generated tool name as the action name.
Explicitly enabled plugin MCP operations use
`plugin.mcp.PLUGIN.SERVER.tools` and `plugin.mcp.PLUGIN.SERVER.call`. Inspect the exact
active names with `config effective`.

Plan lifecycle operations that have no model-callable tool keep their action identity:
operator discard is `plan.discard`, and either execution strategy is `plan.execute`.

## Exact built-in action catalog

The following names are the complete first-party catalog accepted by exact access
overrides. Dynamic integration and explicitly enabled plugin MCP actions are added only
from the active run snapshot and workspace overlay.

| Class | Exact action names |
| --- | --- |
| Provider | `provider.echo`, `provider.openai.responses`, `provider.openai.codex`, `provider.openai.chat`, `provider.models`, `provider.call` |
| Read | `filesystem.read`, `filesystem.list`, `filesystem.metadata`, `filesystem.search`, `git.status`, `git.diff`, `git.show`, `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary`, `context.show`, `context.snapshots`, `patch.preview`, `task.list`, `decision.list`, `plan.show`, `goal.show`, `subagent.read`, `subagent.list`, `memory.read`, `memory.list`, `memory.search`, `memory.index.status`, `plugin.list`, `plugin.inspect`, `plugin.skill.read`, `plugin.resource.list`, `plugin.resource.read`, `plugin.validate`, `plugin.verify`, `bundle.verify`, `bundle.key.inspect`, `mcp.tools`, `browser.status`, `browser.tabs`, `browser.snapshot`, `browser.wait` |
| Local state | `session.set_title`, `context.compact`, `context.restore`, `presentation.preferences.update`, `presentation.history.append`, `task.create`, `task.update`, `decision.create`, `decision.update`, `decision.archive`, `decision.supersede`, `plan.create`, `plan.update`, `plan.discard`, `goal.create`, `goal.update`, `goal.iteration.record`, `subagent.create`, `subagent.start`, `subagent.complete`, `subagent.fail`, `subagent.cancel`, `subagent.interrupt`, `subagent.requeue`, `memory.create`, `memory.update`, `memory.archive`, `memory.supersede`, `memory.index.sync`, `memory.index.rebuild`, `workflow.webhook.ingest`, `workflow.subscription.dispatch`, `browser.tab.select`, `browser.tab.close`, `browser.stop`, `browser.close` |
| Workspace mutation | `filesystem.write`, `patch.apply`, `patch.reverse`, `trace.export`, `audit.export.write` |
| Execution | `process.spawn`, `shell.run`, `plugin.registry.credential_helper`, `workflow.execute`, `workflow.start`, `agent.run`, `plan.execute` |
| External network | `network.http`, `web.search`, `embedding.openai.create`, `memory.index.chroma.search`, `memory.index.chroma.status`, `memory.index.chroma.upsert`, `memory.index.chroma.remove`, `memory.index.chroma.reset`, `research.run`, `integration.openapi.import`, `integration.connect`, `integration.disconnect`, `integration.invoke`, `mcp.invoke`, `mcp.call`, `browser.open`, `browser.tab.open`, `browser.navigate`, `browser.back`, `browser.forward`, `browser.reload`, `browser.click`, `browser.fill`, `browser.select`, `browser.press`, `browser.scroll` |
| Administration | `plan.approve_request`, `audit.export.worm.write`, `plugin.install`, `plugin.enable`, `plugin.disable`, `plugin.workspace.accept`, `plugin.workspace.disable`, `plugin.update`, `plugin.uninstall`, `plugin.gc`, `plugin.package`, `plugin.pull`, `plugin.push`, `plugin.export`, `bundle.build`, `bundle.install` |

## Effect action classes

Exact action names are printed by `tools list` and `config effective`. They fall into
these operational classes:

| Class | Examples | Typical `development` posture |
| --- | --- | --- |
| Pure | Echo and catalog search | Allowed, no adapter effect |
| Provider | Model and provider calls | Allowed when configured |
| Read | Filesystem, Git, repository, memory, context | Allowed with exact obligations; output may be post-gated |
| Colossus state mutation | Tasks, decisions, plans, goals, sessions | Allowed with canonical ownership checks |
| Workspace mutation | File write, patch apply | Approval-required |
| Execution | Process and plugin MCP tool | Approval-required |
| External network | HTTP, search, integration, plugin registry | Approval-required |
| Installation and trust | Plugin and release-bundle lifecycle | Approval-required |
| Administration and recovery | Export reset, recovery transitions | Approval-required |

An action decision never supplies a resource grant. `allow_all` still requires a trusted
registered action, valid explicit, profile-derived, or ambient obligations, and
permit-bound execution. Configured `*` remains public HTTP(S)-only. Ambient authority
is a separate acknowledged mode and permits exact private, loopback, link-local, and
metadata HTTP(S) origins.

## Call and recovery contract

- Tool arguments are validated before execution.
- Malformed provider arguments receive at most two bounded correction turns and never
  reach an adapter.
- A permit is one-use, short-lived, actor/request/decision-bound, and opaque outside the
  policy boundary.
- Each effect records request, decision, approval, start, and terminal evidence.
- A missing terminal event after start becomes `outcome_unknown`.
- Unknown external effects are not silently retried.
- Credentials remain references and raw values are hard-redacted.

## Plain-language task scheduling

`workflow.task.schedule` is the agent-facing calendar-task tool. Its strict arguments
are `schedule_id`, `task`, `calendar`, `starts_at`, `misfire_policy`, `enabled`, and
`idempotency_key`. `task` contains a name, instructions, explicit tool ceiling, and
optional configured model/effort preferences. `calendar` contains an IANA timezone,
`HH:mm` local time, and ISO weekdays; an empty weekday list means daily. `starts_at`
must be the exact first UTC occurrence matching those calendar fields. Serialized
task content must fit the 48 KiB inline approval review bound.

The trusted runtime derives the internal definition and empty workflow inputs. The
tool rejects workflow identifiers, hashes, elapsed cadence, origin, session, and
run fields. Its effect action and policy capability are `workflow.schedule.create`;
it does not introduce a separate approval exemption. Application runs require
`schedules:read`, `schedules:create`, `workflows:read`, and `workflows:register` plus an
explicit tool grant. Task tool names do not grant their own action permissions.

Use `workflow.schedule.list` to check for an existing request and
`workflow.schedule.get` to confirm the stored task. Retry identities are scoped to
the application owner and exact canonical request. Retain them across uncertain
responses. The bundled `colossus/schedule-task` skill documents this flow.

`workflow.schedule.delete` accepts only `schedule_id` and the freshly inspected
64-character `etag`. It uses the `workflow.schedule.delete` effect action and normal
approval obligations; application callers need `schedules:read`, `schedules:control`,
and an explicit tool grant. It rejects foreign and unknown-owner legacy records.
Deletion appends a durable tombstone, stops future ticks, and removes the schedule
from the active catalog. Already allocated runs and their ownership remain intact.
The same reviewed deletion can be reconciled without another append, but deleted
IDs cannot be allocated again. Lost mutation responses are never retried automatically.
