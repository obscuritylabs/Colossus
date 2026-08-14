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
| Tasks | `task.create`, `task.update`, `task.list` | Canonical session work |
| Decisions | `decision.create`, `decision.update`, `decision.list`, `decision.archive`, `decision.supersede` | Binding canonical decisions |
| Plans | `plan.create`, `plan.update`, `plan.show`, `plan.approve_request` | Session-scoped, revision-aware lifecycle; the update target is bound by the runtime |
| Goals | `goal.show`, `goal.update` | Active goal lineage only |
| Subagents | `agent.delegate`, `agent.result`, `agent.list` | Durable child jobs; recursive delegation denied |
| Memories | `memory.create`, `memory.update`, `memory.list`, `memory.search`, `memory.archive`, `memory.supersede` | Canonical lifecycle; retrieval post-gated |
| Context | `context.show`, `context.compact`, `context.snapshots`, `context.restore` | Encrypted immutable snapshots |
| Skills | `skill.scaffold`, `skill.inspect`, `skill.read`, `skill.write`, `skill.validate`, `skill.install`, `skill.resource.list`, `skill.resource.read` | Data-only authoring/resources |
| Search and fetch | `web.search`, `web.fetch`, `docs.fetch`, `network.http` | Search needs an explicit route; generic fetch needs host activation plus declared or ambient HTTP(S) authority; quarantined output |
| MCP | `mcp.servers`, `mcp.tools`, `mcp.call` | Configured stdio servers and exact tool allowlists |
| Integrations | Connected operation names | Configured, trusted, and selected only |

Every tool schema denies unknown fields. Tool availability does not imply permission.
The access profile and exact overrides decide visibility and the built-in decision;
policy, approval, trust, the Safety Kernel, permits, sandbox obligations, quarantine, and
post-effect release remain independent.

`repo.file_summary` applies both the requested line ceiling and a 64 KiB serialized
result ceiling. Its preview and structural-hint collections are byte-bounded before
they enter durable tool history; `preview_truncated` is true when either the line or
encoded-byte ceiling was reached. This keeps generated or minified long lines from
consuming an entire model input budget.

## Plan Mode catalog and lifecycle actions

Plan Mode narrows the already-resolved tool catalog; it never widens access. A Create
turn exposes `plan.create`, while an Update turn exposes `plan.update`. The latter schema
contains only replacement content and steps: the runtime binds the exact plan ID and
expected revision, so the model cannot redirect the write.

The remaining Plan Mode allowlist is:

- `echo`, `tool.search`, and interactive `user.ask`;
- `filesystem.list`, `filesystem.read`, `filesystem.search`, `git.status`, `git.diff`,
  `git.show`, `repo.map`, `repo.symbol_search`, `repo.references`,
  `repo.file_summary`, and `patch.preview`;
- `context.show`, `context.snapshots`, `skill.resource.read`, `task.create`,
  `task.list`, `decision.list`, `plan.show`, `memory.list`, `memory.search`,
  `agent.result`, and `agent.list`.

Normal access resolution and prerequisites can remove entries from that list. Plan Mode
never offers filesystem writes, patch application, command/process execution, approval,
networking, delegation, plan execution, or plan discard.

`plan.discard` is an operator-only Local State action rather than a model tool.
`plan.approve_request` remains Administration. Direct execution and approved-plan Goal
handoff both use the `plan.execute` Execution action. Update, discard, approval, and
execution all cross the ordinary effect gateway; terminal commands do not bypass access,
policy, approval, permits, or audit.

## `shell.run`

`shell.run` accepts exactly one invocation form:

```json
{"command":"cargo test -p colossus-runtime --lib","cwd":".","timeout_ms":120000}
```

```json
{"argv":["git","status","--short"],"cwd":"."}
```

`command` is the recommended form for a bounded non-interactive script. Colossus
selects the trusted shell supplied by `workspace-development` or one explicit shell
grant and invokes it without startup profiles. `argv` preserves exact execution and
requires its first entry to resolve to one configured or derived executable. Shell
wrappers used in `argv` cannot request login, interactive, or startup-profile behavior.

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
| `web.fetch`, `docs.fetch`, `network.http` | `network.http` |
| `mcp.tools` | `mcp.tools` |
| `mcp.call` | `mcp.call` |

Connected integration operations use their generated tool name as the action name.
Verified pack tools use `pack.tool.PACK.TOOL`; pack MCP operations use
`pack.mcp.PACK.SERVER.tools` and `pack.mcp.PACK.SERVER.call`. Inspect the exact active
names with `config effective`.

Plan lifecycle operations that have no model-callable tool keep their action identity:
operator discard is `plan.discard`, and either execution strategy is `plan.execute`.

## Exact built-in action catalog

The following names are the complete first-party catalog accepted by exact access
overrides. Dynamic integration and verified-pack actions are added only from their
active trusted declarations.

| Class | Exact action names |
| --- | --- |
| Provider | `provider.echo`, `provider.openai.responses`, `provider.openai.codex`, `provider.openai.chat`, `provider.models`, `provider.call` |
| Read | `filesystem.read`, `filesystem.list`, `filesystem.metadata`, `filesystem.search`, `git.status`, `git.diff`, `git.show`, `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary`, `context.show`, `context.snapshots`, `patch.preview`, `task.list`, `decision.list`, `plan.show`, `goal.show`, `subagent.read`, `subagent.list`, `memory.read`, `memory.list`, `memory.search`, `memory.index.status`, `skill.inspect`, `skill.read`, `skill.validate`, `skill.resource.list`, `skill.resource.read`, `pack.verify`, `bundle.verify`, `bundle.key.inspect`, `collection.verify`, `mcp.tools` |
| Local state | `context.compact`, `context.restore`, `presentation.preferences.update`, `presentation.history.append`, `task.create`, `task.update`, `decision.create`, `decision.update`, `decision.archive`, `decision.supersede`, `plan.create`, `plan.update`, `plan.discard`, `goal.create`, `goal.update`, `goal.iteration.record`, `subagent.create`, `subagent.start`, `subagent.complete`, `subagent.fail`, `subagent.cancel`, `subagent.interrupt`, `subagent.requeue`, `memory.create`, `memory.update`, `memory.archive`, `memory.supersede`, `memory.index.sync`, `memory.index.rebuild`, `workflow.webhook.ingest`, `workflow.subscription.dispatch` |
| Workspace mutation | `filesystem.write`, `patch.apply`, `patch.reverse`, `trace.export`, `skill.scaffold`, `skill.write`, `skill.install`, `audit.export.write` |
| Execution | `process.spawn`, `shell.run`, `workflow.execute`, `workflow.start`, `agent.run`, `plan.execute` |
| External network | `network.http`, `web.search`, `embedding.openai.create`, `memory.index.chroma.search`, `memory.index.chroma.status`, `memory.index.chroma.upsert`, `memory.index.chroma.remove`, `memory.index.chroma.reset`, `research.run`, `integration.openapi.import`, `integration.connect`, `integration.disconnect`, `integration.invoke`, `mcp.invoke`, `mcp.call` |
| Administration | `plan.approve_request`, `audit.export.worm.write`, `pack.install`, `pack.enable`, `pack.disable`, `pack.uninstall`, `pack.trust.add`, `bundle.build`, `bundle.install`, `collection.build`, `collection.install`, `registry.pull`, `registry.push` |

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
| Execution | Process and pack tool | Approval-required |
| External network | HTTP, search, integration, registry | Approval-required |
| Installation and trust | Skill/pack/bundle/collection lifecycle | Approval-required |
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
