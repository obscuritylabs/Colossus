# Built-in Tools

The Rust agent sees only exact names listed in `agent.tools`. Each tool has a strict JSON
Schema with unknown fields denied. Pure tools execute locally; effectful tools construct a
normal request and cannot reach an adapter without policy authorization, any required
approval proof, and a matching one-use permit.

```bash
colossus --config .colossus/config.yaml tools list
```

The output is the authority for the active model-visible catalog, schemas, mutation
labels, and effect identities.

## Tool Families

| Family | Tools | Effect boundary |
| --- | --- | --- |
| Smoke/discovery | `echo`, `tool.search` | Pure |
| Filesystem | `filesystem.list`, `filesystem.read`, `filesystem.search`, `filesystem.write`, `filesystem.replace` | Read/write roots, quarantine for reads |
| Git/process | `git.status`, `git.diff`, `git.show`, `shell.run` | Exact executable, argv, cwd, environment, resource limits |
| Patch | `patch.preview`, `patch.apply`, `patch.reverse` | Preview is read; apply/reverse require write authorization |
| Repository context | `repo.map`, `repo.symbol_search`, `repo.references`, `repo.file_summary` | Workspace-confined reads and post-release |
| Tasks | `task.create`, `task.update`, `task.list` | Canonical session-scoped repository effects |
| Decisions | `decision.create`, `decision.update`, `decision.list`, `decision.archive`, `decision.supersede` | Canonical session commitments |
| Plans | `plan.create`, `plan.show`, `plan.approve_request` | Approval is a separately authorized transition |
| Goals | `goal.show`, `goal.update` | Exposed only inside active goal lineage |
| Subagents | `agent.delegate`, `agent.result`, `agent.list` | Durable child jobs; recursive delegation denied |
| Memories | `memory.create`, `memory.update`, `memory.list`, `memory.search`, `memory.archive`, `memory.supersede` | Canonical lifecycle plus post-gated retrieval |
| Context | `context.show`, `context.compact`, `context.snapshots`, `context.restore` | Encrypted immutable snapshots |
| Skills | `skill.scaffold`, `skill.inspect`, `skill.read`, `skill.write`, `skill.validate`, `skill.install`, `skill.resource.list`, `skill.resource.read` | Data-only authoring/resource boundaries |
| Trace | `trace.show`, `trace.export` | Metadata view; export is a filesystem effect |
| Fetch/research | `web.fetch`, `docs.fetch`, `web.search` | Search uses an explicit operator route; exact network origin and post-effect release |
| Integrations/MCP | Dynamically connected names | Hidden until configured, allowlisted, and connected |

Tool availability does not imply permission. The built-in PDP is deny by default, and
sandbox obligations independently constrain roots, executables, environment names,
network origins, time, output, process count, memory, and concurrency.

`web.search` accepts only `query` and optional `limit`; the model cannot choose its
provider. It is absent unless explicitly listed in `agent.tools` and a valid
`search.roles.agent` route exists. Search returns normalized results only, while
`web.fetch` retrieves an exact result page. See [Provider-Neutral Web Search](SEARCH.md).

## Files And Processes

Model file paths are repository-relative and rechecked against canonical absolute roots.
Reads/searches are bounded and quarantined; writes use safe leaf rules and atomic
replacement. `.colossus` control state is not a generic workspace target.

Process tools never invoke a shell. `shell.run` accepts one executable plus literal argv;
characters such as `|`, `&&`, backticks, and `$()` remain ordinary arguments. Configure
the exact executable under `sandbox.executables`. Native and OCI helpers start from a
cleared environment and expose only named variables.

Mutation results include bounded diff visibility. Adapter output is released only after
any required post-effect policy decision; denied content never reaches the agent.

## Durable State Tools

Task, decision, plan, goal, subagent, memory, and context tools append immutable encrypted
events. Model arguments omit authority-bearing session or goal identity where the runtime
can derive it from execution context. Runtime adapters recheck canonical ownership before
mutation or release.

Active decisions are binding context. Memories are non-instructional background.
Archived or superseded records remain auditable but do not steer later turns.

## Tool Calls And Recovery

- Provider tool arguments are validated before execution. Malformed arguments receive at
  most two bounded correction turns and never reach an adapter.
- A permit is one-use, short-lived, actor/request/decision-bound, and opaque outside the
  policy crate.
- Every effect records requested, decision, approval, started, and terminal evidence.
- A crash after `effect.started` without a terminal event becomes `outcome_unknown`.
- Unknown external effects are never silently retried.
- Credentials are references and are resolved only after permit validation; raw values
  are hard-redacted from policy input, provider output, transcripts, and audit evidence.

See [Configuration](CONFIGURATION.md) for grants and [Security Model](SECURITY.md) for the
non-bypassable kernel.
