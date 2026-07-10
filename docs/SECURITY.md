# Security Model

## Rust Safety Kernel And Effect Gateway

The Rust reconstruction treats every external or sensitive operation as an effect. The
only supported path to an effectful adapter is:

```text
request journal -> hard safety kernel -> built-in/OPA decision -> approval proof
-> authenticated one-use permit -> adapter quarantine -> optional release decision
-> terminal journal event
```

The safety kernel rejects invalid requests, unknown capabilities, invalid/expired/reused
permits, unsafe path obligations, absent audit durability, oversized policy input, and
hard-secret disclosure regardless of policy. Adapter constructors stay in the runtime;
adapter methods require the opaque permit type. A compile-fail test proves external code
cannot construct one.

Journal payloads use XChaCha20-Poly1305 with platform or explicit environment keys. The
journal never silently stores plaintext. Records are globally hash chained and stream
versioned; Ed25519 checkpoints and separately protected anchors detect record changes and
consistent tail truncation. Startup verification failure enables read-only recovery and
blocks effects. A missing terminal event after `effect.started` is recorded as
`effect.outcome_unknown` and never automatically retried.

OPA receives full logical request content after raw credentials, authentication headers,
private keys, key material, and hidden reasoning are replaced by bounded hashes and
references. Input over 1 MiB, transport failure, invalid decisions, missing obligations,
or unhealthy policy fails closed. Remote OPA requires HTTPS, mTLS, pinned CA trust, a
fixed decision path, and explicit disclosure acknowledgement. Output can remain in a
bounded quarantine until a post-effect allow decision.

The sections below describe the frozen Python 0.5 implementation. They remain relevant
to `python-v0.5.0` and `python-legacy`, but they are not authority for the Rust cutover.

Colossus starts with capability-based policy, brokered execution, and append-only audit
logs. OS-level isolation can be added behind the subprocess broker without changing tool
contracts.

For user-facing setup and troubleshooting, see [Getting Started](GETTING_STARTED.md) and
[Troubleshooting](TROUBLESHOOTING.md). This document is the trust-boundary reference.

## Core Rules

- Tools declare command, argument schema, working-root policy, environment allowlist,
  timeout, output cap, and network policy.
- Subprocess execution goes through the broker.
- `shell=True` is not used.
- Tool inputs are validated before execution.
- Approval is required when policy returns `requires_approval`, unless the user
  explicitly selects a no-prompt approval mode.
- Audit records are hash-chained JSONL entries.
- Redaction is on by default for command inputs and environment values.

Airgapped installs should be verified from signed bundles containing wheelhouse,
lockfiles, SBOM, manifests, signatures, and skills.

## Trust Boundaries

- User interfaces collect input and render output only.
- Application services decide orchestration flow and call ports.
- Adapters interact with subprocesses, model providers, local state, filesystem skills,
  and audit sinks.
- Domain objects carry policy, tool, request, event, and audit data without depending
  on infrastructure.

Changes to subprocess execution, approval policy, audit records, bundle handling, or
skill loading should include security-focused tests.

Skill Mode treats skills as prompt/context data, not executable plugins. Active skills
are validated against the agent allowlist and active tool catalog before provider calls;
`required_tools` never auto-approves a tool. Skill audit records include names, versions,
and sources only, not full `SKILL.md` bodies.

Skill resource tools are read-only and active-skill-scoped. They accept only safe relative
paths under allowed resource directories, reject traversal, reject non-regular files, and
bound text reads. Resource read audit records include skill name, path, and size, not the
resource body.

Repo-local `.agents/skills` are workspace-authored files, not Colossus runtime state.
They are reachable through normal workspace filesystem tools and can be checked into the
repository. `.colossus` remains a denied control directory for generic workspace tools
because it is reserved for Colossus-owned runtime/control files.

Packs are the executable distribution boundary. Skills can include code files as
resources, but Colossus does not execute scripts directly from skill directories.
Executable tools, MCP servers, binaries, Docker assets, docs, and tests must be declared
in `colossus.pack.json`; executable and binary files must be hash-listed and
permission-declared.

## Tool Execution

Tools are expected to declare their execution permissions up front. Policy can allow,
deny, or require approval before execution. The broker validates tool inputs and runs
commands without a shell so arguments are passed explicitly.

Malformed provider tool-call argument payloads are never repaired into executable tool
calls locally. Colossus may ask the provider to retry a bounded number of times with a
metadata-only correction prompt, emits a recoverable error event, and audits recovery or
exhaustion. Policy, approval, and execution only receive later provider turns that
normalize into valid typed tool-call events.

Security-sensitive defaults:

- Keep environment allowlists narrow.
- Keep output caps and timeouts bounded.
- Require approval for mutation, elevated filesystem access, or network access.
- Preserve command, decision, and memory audit records.

## Built-in Tool Permission Matrix

| Tool family | Filesystem | Network | Approval | Offline default |
| --- | --- | --- | --- | --- |
| `filesystem.list/read/search` | Read | Denied | No | Enabled |
| `filesystem.write/replace` | Write | Denied | Yes | Enabled |
| `git.status/diff/show` | Read | Denied | No | Enabled |
| `shell.run` | Write-capable | Denied | Yes | Enabled |
| `task.*` | None | Denied | No | Enabled, session-persisted |
| `decision.*` | None | Denied | Mutations | Enabled, session-persisted |
| `memory.*` | None | Denied | No | Enabled, global/repo/session persisted |
| `goal.show/update` | None | Denied | No | Enabled only inside active goal-mode provider turns |
| `plan.create/show` | None | Denied | No | Enabled, runtime-local |
| `plan.approve_request` | None | Denied | Yes | Enabled, runtime-local |
| `patch.preview` | Read | Denied | No | Enabled |
| `patch.apply/reverse` | Write | Denied | Yes | Enabled |
| `repo.*` | Read | Denied | No | Enabled |
| `agent.*` | None | Denied | No | Enabled, durable queued child-agent jobs |
| `web.fetch` and `docs.fetch` | None | Allowed by spec | Yes | Bounded HTTP(S) fetch after approval |
| `web.search` | None | Allowed by spec | Yes | Exposed only when a search adapter such as SearXNG is configured |
| `mcp.servers/tools` | None | Denied | No | Returns unconfigured state |
| `mcp.call` | None | Allowed by spec | Yes | Adapter extension point, not exposed by default |
| `github.*`, `searxng.*`, `opensearch.*`, and `openapi.NAME.*` | None | Allowed by spec | Yes | Exposed only after integration connection |
| Deep research | Read | Allowed by configured lanes | Network/MCP lanes | Persists cited reports and source records |
| `trace.show` | Read | Denied | No | Enabled |
| `trace.export` | Write | Denied | Yes | Enabled |
| `context.show/compact/snapshots` | None | Denied | No | Enabled |
| `context.restore` | None | Denied | Yes | Enabled |
| `skill.scaffold` | Write | Denied | Yes | Enabled, writes manifest/SKILL.md and requested resource dirs in installed skill directory only |
| `skill.inspect/read` | Read | Denied | No | Enabled, installed user skill files only |
| `skill.write` | Write | Denied | Yes | Enabled, installed user skill files only; existing files require expected SHA-256 |
| `skill.validate` | Read | Denied | No | Enabled, validates installed skills by name or local skill directories by path |
| `skill.install` | Write | Denied | Yes | Enabled, validates a local skill directory and installs into `~/.agents/skills` |
| `skill.resource.list/read` | Read | Denied | No | Enabled, active skill resources only |

The default policy requires approval for declared mutations, explicit approval flags,
network-capable tools, and high-risk tools. The orchestrator validates model-provided
tool arguments against the tool schema before requesting approval.

## Approval Modes

`deny` blocks approval-required tools, `ask` prompts interactively, and `risk-auto` may
auto-approve low-risk `shell.run` calls after model-assisted risk review. In `risk-auto`,
risk-review `deny` results become approval prompts rather than unconditional hard stops.
`full-access` auto-approves approval-required tools without prompting, skips
model-assisted `shell.run` risk review, and records `tool.auto_approved` audit entries
plus `ApprovalAutoGrantedEvent` events.

`full-access` is a no-prompt approval policy, not a broader sandbox profile. It does not
expand filesystem roots, network implementations, tool schemas, or deterministic policy
denies. Unknown tools, invalid arguments, and policy `deny` still stop before execution.

## Subagents

Subagents are durable queued child-agent jobs. They reuse the normal orchestrator, tool
registry, policy, approval, risk, state, and audit paths. They do not create a separate
OS sandbox or broader permission domain. V1 child agents inherit the parent approval mode
and tool boundaries, but nested `agent.delegate` is removed from child tool catalogs to
avoid runaway delegation trees.

Running subagent jobs that outlive a process are marked `interrupted` on startup rather
than resumed from a half-open provider stream. Queued jobs remain runnable when a runtime
with a configured subagent runner starts.

## Model-Assisted Risk Review

`shell.run` is reviewed by the `risk_evaluator` model role when configured and approval
mode is not `full-access`. Review happens after deterministic schema and policy checks,
before approval prompts. The evaluator sees redacted structured metadata and runs with
tools disabled. It may add risk explanation, require approval, deny a call, or
auto-approve only when `--approval-mode risk-auto` is explicitly enabled and the review
returns `risk_level=low` with `recommended_decision=allow`. In `risk-auto`, model-risk
denies are escalated to explicit approval prompts; outside `risk-auto`, they stop before
execution. It cannot make deterministic denies executable. If risk review is unavailable
or returns invalid JSON, Colossus records `risk.review_unavailable` and continues with
deterministic policy.

## Context Compaction

Context snapshots are derived from raw session messages and persisted in SQLite. They are
used to reduce what is sent to a provider, but they do not delete, rewrite, or replace
raw message history. `context.restore` is approval-required because it changes which
snapshot is active for future model requests. Model-assisted compaction is best-effort;
deterministic offline compaction remains the fallback.

Key decisions are durable commitments, not memories. They should store the interpreted
future-facing decision, user intent, applicability, and a short source excerpt rather
than raw prompt text as the decision itself. Active key decisions are stored as session
state and injected before compacted snapshot content as binding guidance so summarization
cannot erase them. Archived and superseded decisions remain in state for auditability but
are not injected into future model context.

Memories are durable context, not instructions. Active relevant memories are injected
after key decisions and before compacted snapshot content. Global memories are only
used when relevant to the current prompt/repository; archived and superseded memories
remain persisted for history but are not injected. Memory records should not store
secrets, raw credentials, private keys, or unbounded external/tool output.

Deep Research Mode persists source records and cited reports. Repository collection is
read-only. Configured web search and MCP source collection require approval, and disabled
or denied source lanes are recorded as limitations rather than bypassed. Search provider
secrets are read from environment variables and must not be sent as tool arguments,
source metadata, or audit payload fields.

Global HTTP PKI and proxy settings configure transport for Colossus-owned `httpx`
clients only. They do not grant network approval, expand tool schemas, or affect HTTP
requests made inside external subprocesses or MCP server processes.

## Integration Credentials

Integration credentials are referenced by local handles such as `env:GITHUB_TOKEN`.
Connection records store handles, scopes, manifests, config, and status, never raw
secrets. Some connectors, such as OpenSearch basic auth, store named refs like
`username=env:OPENSEARCH_USER` and `password=env:OPENSEARCH_PASSWORD`. Tool handlers
resolve handles at execution time, inject auth headers in the adapter, and audit the
connector name, tool name, credential refs, and argument keys only.

Missing credentials produce a pending-auth connection or a tool execution error. Raw API
keys, bearer tokens, OAuth refresh tokens, client secrets, and service-account JSON must
not be included in model-visible tool schemas, `ModelRequest` payloads, transcript
output, trace details, or audit payloads. OpenAPI imports generate operation tools, but
they do not turn auth fields into model arguments.

## Reasoning Visibility

Colossus may render provider-supplied reasoning summaries when an endpoint exposes a
safe summary field. It does not render raw hidden reasoning text in the default CLI or
REPL paths. Stream chunks are normalized into typed events; raw provider chunks are not
persisted unless a future explicit debug mode adds that capability.

The REPL transcript labels these safe summaries as `thinking` only when they arrive as
typed `ReasoningSummaryEvent` values. Local activity such as tool calls, approvals, and
risk assessments is rendered as harness activity, not as hidden model reasoning.

## REPL Themes And Preferences

REPL preferences are stored as typed JSON in SQLite state. They control display behavior
only and do not change provider, policy, tool, or approval decisions. User theme files
are data-only JSON/TOML loaded from the config themes directory. Prompt, trace, and
transcript style keys are validated against allowlists, executable plugins are not loaded
through the theme path, and invalid theme files fail fast rather than being partially
applied.

## Audit Logs

Audit records are append-only JSONL and hash chained. They are intended to support
debugging, release review, and incident response. Redaction is enabled by default for
command inputs and environment values, but operators should still treat audit logs as
sensitive operational records.

## Telemetry

Telemetry summaries are derived from persisted typed run events and should remain
metadata-first by default. Run timelines may show event types, timestamps, tool names,
exit codes, byte counts, approval/risk summaries, research progress, subagent status,
and aggregate counts, but they must not expose hidden reasoning, raw provider chunks,
raw prompts, raw credentials, or raw tool outputs unless a future explicit debug mode
adds separate controls and redaction.

## Offline Bundles

Offline bundles must be verified before installation or use:

```bash
uv run colossus bundle verify ./bundle
```

The current verifier checks that the bundle manifest exists, that file entries are
well-formed, and that every listed file matches its SHA-256 checksum. Release bundles
should also include signatures, SBOMs, lockfiles, wheels, and reviewed skill manifests.

See [Offline Bundle Format](BUNDLE_FORMAT.md) for the directory and manifest format.

## Vulnerability Reporting

See the root [Security Policy](../SECURITY.md) for supported versions and vulnerability
reporting expectations.
