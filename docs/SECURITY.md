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

Checkpoint creation persists the independently protected head anchor before redb
checkpoint metadata. If the process terminates between those writes, verified startup
recreates the signed checkpoint from the anchored journal head. It also repairs a due
100-event checkpoint when termination occurs immediately after the event transaction;
an invalid or mismatched anchor still enters read-only recovery.

Security-critical untrusted parsers have shared stable and libFuzzer exercises. The
fuzz boundary covers strict journal envelopes, redacted audit evidence, signed
checkpoints, effect requests, policy decisions, workflow YAML/schema validation, and the
non-executable condition grammar. Workflow conditions fail closed above 16 KiB, 4,096
tokens, 128 recursive levels, or 128 boolean-composition nodes. Committed corpora run in
normal workspace tests; pinned nightly CI performs bounded mutation runs and retains crash
artifacts.

Rust dependency policy is locked and fail-closed for both the production and independent
fuzz workspaces. `cargo-deny` evaluates the complete six-release-target graph, permits only
the licenses listed in `rust/deny.toml`, rejects wildcard version requirements, rejects
unknown registries and Git sources, and bans dependencies that would bypass the rustls or
license boundary. Internal path dependencies include the exact current prerelease version.
Duplicate transitive versions remain visible warnings so they can be reduced without
blocking an otherwise safe graph. `cargo-deny` advisory checks and an independent
`cargo-audit` scan both deny RustSec warnings for `Cargo.lock` and `fuzz/Cargo.lock`; there
are no advisory exceptions or silent vulnerability downgrades.

Release artifacts are built and executed on native arm64 and x64 runners for macOS,
Linux, and Windows. The smoke configuration uses explicit environment keys, no external
credential, no network grant, and the echo provider; it proves that the packaged binary
can parse strict configuration, create an encrypted journal, complete an agent turn, and
verify the resulting chain and signed checkpoint. Linux artifacts must report static
musl linkage before packaging. Each archive has a SHA-256 sidecar for transport-integrity
checking. A checksum alone is not publisher authentication; signed release/offline-bundle
verification remains the authority for trusted distribution.

Native archives include data-only installation scripts. CI extracts the completed
archive into an empty directory, installs into an empty prefix, and repeats version,
echo-agent, and encrypted audit verification using only the installed executable. The
installers reject linked source executables and linked destination `bin` directories;
Unix replacement uses a same-directory temporary file and atomic rename. Installation
does not resolve provider credentials or make network requests.

Signed-bundle construction and installation are separate approval-required effects.
Construction receives only an environment credential reference in policy/audit content;
the permit-bearing adapter resolves the 32-byte Ed25519 seed, requires the derived key to
match canonical publisher trust, copies a link-free bounded staging tree, hashes the
copied bytes, signs deterministic canonical JSON, re-verifies the result, and atomically
publishes a new destination. Installation re-verifies every signature/hash, selects the
running platform's exact artifact path, requires a matching write root, copies through a
same-directory temporary file, and creates a previously absent executable without
clobbering. A modified bundle or linked/existing destination fails before release.

Configured external audit export is itself an effect; it never receives a trusted-service
bypass. The exporter discloses only ciphertext-free `AuditEvidence` envelope metadata and
hashes. Every directory write requires `audit.export.write`, a matching sandbox write
root, and a one-use permit. The configured root must already exist. Export authorization
events use the reserved `system/audit-exporter` actor and are retained canonically but
not recursively exported. Retryable failures use durable bounded backoff; an unknown
write outcome blocks implicit retry until an operator explicitly resets the consumer.
The local directory sink is replay-safe operational evidence, not WORM storage; a future
remote/WORM adapter must preserve the same policy and evidence contract.

OPA receives full logical request content after raw credentials, authentication headers,
private keys, key material, and hidden reasoning are replaced by bounded hashes and
references. Input over 1 MiB, transport failure, invalid decisions, missing obligations,
or unhealthy policy fails closed. Remote OPA requires HTTPS, mTLS, pinned CA trust, a
fixed decision path, and explicit disclosure acknowledgement. Output can remain in a
bounded quarantine until a post-effect allow decision.

### Rust sandbox and broker

The Rust filesystem, subprocess, and HTTP adapters can execute only with a valid permit
whose request hash matches the complete proposed effect. Filesystem paths are
canonicalized against explicit read/write/execute roots, symlink leaves are rejected for
writes, reads are bounded, and writes use a same-directory temporary file plus atomic
rename.

The permit contract is shared by every effect category rather than reimplemented by
individual adapters. Regression tests enumerate all application effect families and
prove rejection never produces `effect.started` or an adapter call. Separate claim tests
alter the request, actor, decision, obligations, expiry, and authentication tag, then
verify rejection; a correctly authenticated permit succeeds exactly once and replay is
rejected.

The real-OPA acceptance policy authorizes a disclosure test only when it receives the
complete nested logical request and credential references while the raw secret is absent
and replaced by its bounded hash descriptor. The same suite exercises decision revision,
approval re-evaluation, invalid responses, outages, readiness and decision-log warnings,
post-effect denial, and pinned-CA mutual TLS.

Subprocesses never use a shell. The parent sends a signed, expiring, one-use job document
to a hidden helper over stdin. The signature binds the executable, literal arguments,
working directory, environment, policy decision, permit nonce, obligations, and request
hash. The helper rejects malformed, expired, or tampered jobs, starts with a cleared
environment, and passes only explicitly allowed variable names to the child. Output is
bounded and base64 encoded; nonzero-exit audit records retain only output hashes and
sizes. Process groups on Unix and Job Objects on Windows provide descendant ownership
for timeout and resource-limit termination.

On macOS and Linux, the native helper uses the Apache-2.0 `nono` crate to apply Seatbelt
or Landlock filesystem rules before the child starts. Birdcage was not selected because
its published GPL-3.0-only license is incompatible with Colossus's Apache-2.0
distribution. Network is either denied or limited to a loopback proxy;
the proxy canonicalizes and exactly matches HTTP(S) origins, resolves and pins the
destination, rejects domain names resolving to non-public addresses, strips proxy
credentials, and bounds relay lifetime. Explicit IP-literal origins may opt into private
local addresses. Direct HTTP effects use the same exact-origin and pinned-resolution
rules and remain quarantined until post-effect authorization.

Native Seatbelt/Landlock acceptance is mandatory on macOS and Linux arm64/x64; a runner
that reports isolation unavailable fails instead of skipping. The live suite attempts
symlink and `..` traversal, undeclared environment use, descendant escape after timeout
and normal leader exit, process-count and memory exhaustion, unlisted proxy destinations,
and direct `NO_PROXY`/raw-socket bypass. Windows worker IPC is exercised over named pipes
on native arm64/x64 runners with wrong-key and replay-resistant authentication. Windows
process isolation uses an ephemeral AppContainer package identity plus a Job Object that
is attached in the same `STARTUPINFOEX` process-creation operation. The AppContainer has
no network capabilities, receives ACLs only for canonical policy roots, and receives only
declared environment values plus fixed Windows loader/temp variables. The Job Object owns
the descendant tree and applies aggregate process and memory ceilings. Broker execution
remains available only through the existing explicit downgrade obligation and is not
represented as sandbox isolation.

The OCI backend constructs Docker/Podman invocations with no network, a read-only root,
all capabilities dropped, `no-new-privileges`, bounded PIDs/memory, and only explicit
bind mounts and environment names. The target starts through a fixed `env -i` bootstrap,
so image-defined environment variables do not cross into the executable. OCI execution
uses `--pull=never`; images must be preloaded and referenced by a complete immutable
SHA-256 digest so the Docker/Podman daemon cannot perform an unapproved registry request.
Networked OCI execution additionally requires a preloaded immutable Colossus proxy image.
The workload joins only a per-job internal network and receives no usable DNS resolver;
the proxy sidecar alone also joins a per-job egress network. After authorization, the
helper resolves each approved canonical origin, removes non-public domain answers, and
binds the bounded address sets, request hash, decision ID, permit nonce, and expiry into
the authenticated sidecar bootstrap. Plain HTTP `Host` and HTTPS TLS SNI must match the
approved origin before any upstream connection. Workloads therefore cannot bypass the
proxy with direct DNS, IP, or raw-socket egress.

Every OCI job has an unpredictable authenticated container name. The helper reserves
time within the policy timeout to force removal and verify that no matching container or
network remains. A cancellation guard starts cleanup if the gateway drops a stalled
helper. Network-free OCI timeouts below five seconds and networked OCI timeouts below ten
seconds fail before execution because they cannot reserve enough cleanup time. If
absence cannot be confirmed, the gateway records `outcome_unknown` rather than claiming
failure or retrying.

The configured OCI executable must resolve exactly to Docker, Podman, or the official
`podman-remote` client; arbitrary wrapper executables are rejected. Podman cleanup uses
an explicit zero-second stop timeout before absence verification, while Docker retains
its force-removal semantics. Both runtimes execute the same live acceptance suite.

The `windows_job` backend never relies on a Job Object alone. A unique AppContainer profile
is created for each authenticated helper job, canonical grant roots receive package-SID
ACLs, and the child starts with no network capability. The Job Object, AppContainer
security capabilities, and exact inherited standard-I/O handle list are installed
atomically at process creation, eliminating a create-then-assign descendant race. Closing
the helper or Job handle terminates the whole tree. UI and clipboard access, desktop
switching, global atoms, and system-parameter mutation are also Job-restricted. The
profile is deleted after confirmed termination; an interrupted helper can leave only an
unreferenced unique profile/ACL identity while Job-handle close still terminates processes.
Windows network-free execution is supported. Networked jobs receive only an authenticated
per-permit loopback proxy URL. A temporary package-scoped loopback exemption is wrapped by
dynamic WFP filters that permit that AppContainer SID to reach only the proxy's exact TCP
port on `127.0.0.1` and hard-block all other IPv4/IPv6 connects. The parent proxy still
enforces canonical exact-origin destinations and strips its authorization header before
forwarding; captured output redacts both raw and Basic-encoded credentials. Filter or
loopback-exemption setup failure blocks launch, and there is no broker downgrade. Dynamic
filters disappear when the helper closes its WFP session; an interrupted helper can leave
only an exemption for the job's otherwise orphaned unique package SID. Windows OCI path
mapping remains disabled.
The plain broker is available only when configuration and the policy decision both
explicitly authorize a downgrade.

macOS/Linux native process-count and memory ceilings are supervisor-enforced; Windows Job
Objects and OCI supply hard kernel/container ceilings. `sandbox doctor` reports the selected backend and available
isolation mechanism so operators can reject an unsuitable deployment before effects run.
The opt-in live security suites exercise Docker, Podman, and a real OPA process,
including mTLS; normal workspace tests remain credential-free and network-free.

### Local worker IPC

The optional long-running worker is the sole redb writer while active. Its local protocol
uses length-bounded, versioned JSON frames with HMAC-SHA256 authentication derived from
domain-separated checkpoint key material. Before any prompt or operation content is
sent, a challenge-response handshake proves that the server possesses the key. Each
request is then bound to a fresh server connection nonce, UUIDv7 request id, random
request nonce, and a short timestamp window. The worker rejects tampering, cross-
connection replay, repeated nonces, stale requests, reordered response frames, oversized
frames, and stalled handshakes.

Unix endpoints must be real sockets, owner-only mode, and owned consistently with their
parent directory; symlink endpoints are rejected. Windows uses the first named-pipe
instance plus the same pre-disclosure authentication. The worker checkpoints before
authenticated shutdown and removes its Unix endpoint. IPC never bypasses the runtime:
model, workflow, session, policy, permit, journal, and projection work executes through
the same application services used by embedded mode. Clients fall back to embedded mode
only when the endpoint is unavailable; an authentication or protocol failure is surfaced
and never converted into a fallback request.

Workflow queueing is journal-native: a worker may claim only `queued` runs and recovery
never drains `waiting` or `interrupted` runs. A started step without a durable terminal
record is labeled `outcome_unknown`; operator resume is refused when that step lacks an
explicit idempotency strategy. Compensation is definition-declared, uses separate step
identity and audit events, and crosses the effect gateway independently for every action.
Policy approval of a primary effect never authorizes its compensation.

Recovery compares scoped execution ids, so completion of a repeated or parallel sibling
cannot clear another attempt's uncertainty. An abandoned compensation is explicitly
phase-labeled and never resumed through the primary workflow path, even if its definition
declared idempotency. Separate-process redb tests sync an external-effect marker and then
abort before a terminal workflow event to prove unknown primary/compensation outcomes,
safe idempotent primary retry, unsafe replay refusal, and durable-completion continuation.
The same suite kills parallel branch effects, linked-child creation, and an effect running
inside the child. A parent cannot transition out of interrupted state until its interrupted
child is recovered, and replay neither reauthorizes an already linked child nor duplicates a
durable sibling completion.
Subworkflow launch is a distinct `workflow.start` effect (approval-required by the
built-in policy unless explicitly overridden). Child runs pin their own definition hash
and immutable parent run, parent step, and call depth. A durable parent link is reused on
resume; its journal-encrypted payload can repair a missing child queue record using the
original child ID after interruption.
Static workflow step IDs never serve as the sole identity for repeated execution.
`foreach` items and parallel branches receive scoped execution IDs that bind permits,
idempotency values, input completion, retries, and subworkflow links, preventing an
approval or effect result for one iteration from authorizing or completing another.

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

Rust provider profiles store credential references such as `env:OPENAI_API_KEY`, never
credential values. The complete logical model request and those references cross the
effect gateway before execution. A provider adapter resolves the referenced value only
after receiving a valid one-use permit, and the adapter rejects mismatched profile,
endpoint, credential disclosure, or network-origin obligations. Responses remain in a
bounded quarantine until optional post-effect policy allows release. Only normalized
visible text, strict object-shaped tool calls, and explicitly typed safe reasoning
summaries enter the model event stream; raw response chunks and hidden reasoning fields
are discarded.

Loopback-live acceptance makes a configured provider echo the resolved bearer value in
both a tool call and final streamed text. Colossus must replace it before schema
validation and tool execution, keep it out of the next provider request body, and keep it
out of canonical session messages, telemetry, bounded audit views, stdout, and stderr.
The capture simultaneously verifies that the raw value appears in the Authorization
header only after a permit reaches the adapter.

Streaming does not weaken that boundary. The adapter submits one normalized item at a
time to the gateway, the gateway enforces cumulative output limits and any post-effect
decision before observation, and the agent journals the released item before forwarding
it to CLI or REPL rendering. The gateway latches the first sink failure, so an adapter
cannot ignore a denied callback and continue the effect. A denied item never reaches
either observer.

The Rust agent loop receives tool specifications and execution through separate ports.
The active catalog is explicit in configuration, rejects duplicate/unknown names, and
compiles every object schema at startup. Each model call is validated against that schema
before policy or an adapter sees it. Unknown and invalid calls become correlated error
results without reaching an executor. Effectful file and network tools construct ordinary
effect requests with model/run provenance and can execute only through the gateway;
policy denial and unknown outcomes stop the loop. Malformed provider argument syntax is
never repaired locally and can trigger at most two metadata-only correction turns.

Rust session message bodies remain encrypted canonical journal payloads. The disposable
session projection stores only bounded discovery fields and never copies full assistant
or tool-result content. Resume resolves an exact existing session stream and restores
provider-neutral messages without changing policy, filesystem, workspace, approval, or
network configuration. Every resumed provider and tool effect carries the session and
run identifiers in its execution context.

Rust context snapshots are encrypted domain events on the canonical session stream.
They contain bounded derived summaries and metadata, never replace source messages, and
are activated only by an explicit journal event. Model-assisted summaries use the
`context_summarizer` role through the same policy-enforced provider gateway as other
model effects. Provider failure produces a deterministic local summary; it never causes
silent history deletion or unbounded request truncation. Restoring an older snapshot is
auditable and changes only future provider-visible composition.

Rust tasks and key decisions are encrypted canonical journal streams. Update operations
cannot change their session, creation identity, or decision provenance. Archival never
deletes a decision, and supersession appends both the terminal old state and linked new
active record atomically. All mutations cross the effect gateway and the private work
adapter requires a matching one-use permit; policy denial leaves canonical streams
unchanged. Decision source is derived from immutable actor provenance rather than a
caller-selectable label. Only active same-session decisions enter model context; their
binding block is bounded, token-accounted, and placed ahead of summaries so compaction
cannot silently erase durable commitments. Model-visible schemas never accept a session
identifier. The runtime derives it from the authenticated execution context and the
permit-bound executor checks canonical target ownership, so guessed task or decision ids
cannot cross session boundaries.

Rust plans use the same encrypted canonical work repository. Plan creation derives its
session from execution context, ordered steps contain data rather than executable code,
and content becomes immutable after leaving draft state. `plan.approve_request` cannot
directly set approval: the policy decision must impose approval, the gateway records and
re-evaluates the proof, and only then can the private work adapter append
`plan.approved.v1`. Canonical ownership checks prevent cross-session show or approval,
and an approved plan can transition to executed only once.

Plan Mode is enforced by tool exposure, not instructions alone. Workspace writes,
patch application, subprocesses, delegation, decision/memory mutation, approval, and
other non-planning tools are absent from the request, and a provider-supplied call to
any undisclosed tool is denied before tool lifecycle or adapter execution. Direct plan
execution atomically records `plan.executed.v1` with the exact pending run id before the
agent starts; failure does not make the plan replayable.

Goal Mode does not broaden authority: every iteration uses the same provider, active
tools, policy channel, approvals, sandbox, context preparation, and journal. Goal ids are
runtime-injected and cannot be supplied in model arguments. The private work adapter
checks the active goal and session again after permit minting. Terminal `complete` and
`blocked` transitions require a summary or blocked reason and cannot be reversed. The
runtime appends one iteration record after each returned run. The 50-iteration hard maximum and
persisted per-goal budget prevent unbounded continuation. Approved-plan consumption and
linked goal creation are one optimistic journal batch.

Rust subagents do not create a new authority domain. Job creation, reads, lifecycle
transitions, and result release cross the effect gateway, and model-visible schemas omit
session, parent-run, parent-call, child-session, and role identities. The runtime derives
those values from the parent tool context. Each child receives a distinct durable
session and `ActorType::Subagent` provenance for its tool effects. Nested delegation is
removed from provider definitions and independently denied by the executor. Results are
bounded and post-effect authorized before canonical completion. Cross-session job ids
are rejected after permit minting. Cancellation becomes canonical immediately and late
child output is not committed; process-loss recovery records running jobs interrupted.

Rust memory lifecycle events are encrypted canonical state. Memory operations and index
administration require one-use permits; reads, lists, and searches always require a
post-effect content decision before records can reach a model or user. Tantivy results
are treated only as candidate ids. Colossus reloads canonical records and reapplies
status, expiry, and scope before release. Index failure leaves journal-backed memory
usable through bounded canonical fallback, exposes lag/error status, and never causes a
plaintext downgrade or silent loss. Memory source is derived from actor provenance, and
hard validation rejects common credential/private-key forms before persistence. Updates
are immutable journal events and cannot change identity, scope, source, or creation time.
For model, workflow, and subagent callers, repository and session scopes come from the
runtime context rather than model arguments; targeted operations recheck the canonical
record after authorization, query results are filtered before applying caller limits,
and index administration is unavailable. A repository scope is bound to a stable hash
of the canonical workspace path. Optional Chroma indexing does not weaken this boundary:
Chroma receives candidate projection data only, and returned ids are reloaded and
rechecked against canonical journal state before release. Chroma and remote embedding
requests use separate effect requests, one-use permits, exact-origin obligations,
bounded request/response bodies, late credential resolution, and quarantine. A denied
semantic effect cannot open a network connection. Corrupt local replay-position metadata
degrades the index instead of silently resetting canonical state.
Tantivy and Chroma acknowledge the atomic journal outbox through separate durable redb
consumer positions. Adapter state is persisted before acknowledgment, so interruption
can cause only an idempotent replay, not silent work loss. An adapter position behind its
acknowledged consumer position is treated as a verification failure requiring rebuild.
Unknown Chroma mutation outcomes remain unacknowledged and cannot be retried until an
authorized rebuild clears the adapter's durable unknown-outcome marker.
Retry telemetry contains only a stable category and a redacted diagnostic capped at 2 KiB.
Transient failures use durable exponential backoff capped at five minutes, so repeated
searches cannot hammer an unavailable remote adapter. Verification, recovery-mode,
not-found, and unknown-outcome failures are non-retryable and require operator action.
Memory injected on a later turn is explicitly labeled as background context rather than
instructions.

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

In Rust agent turns, filesystem paths are resolved relative to the workspace captured
when the runtime opens; absolute paths, parent traversal, and `.colossus` components are
rejected before policy. Listing and search then cross the normal filesystem effect
identity, containment checks, one-use permit, bounded adapter, quarantine, and
post-effect decision. Search does not follow symlinks, skips `.colossus` and `.git`,
ignores binary/non-UTF-8 or oversized files, and caps both match count and released
output.

Rust one-shot commands deny approval obligations unless `--approval-mode ask`,
`risk-auto`, or `full-access` is selected; the REPL defaults to `ask`. Ask mode shows a
bounded, hard-redacted proposed-content preview and accepts only an explicit `y`/`yes`.
`full-access` can produce a proof for `require_approval`, but cannot convert a policy deny
to allow, add a filesystem root, or mint an adapter permit itself. `risk-auto` currently
records risk as unavailable and prompts rather than silently auto-approving. Declined and
failed prompts append approval and effect denial events before returning.

Filesystem mutation requests disclose the full proposed UTF-8 content before execution.
The adapter performs create/overwrite/append/replace checks and the atomic write under one
consumed permit, bounds existing and resulting content, rejects non-UTF-8 replacement
targets and ambiguous single replacements, and constructs the diff evidence before the
rename. Mutation result diffs are quarantined and post-authorized because they may reveal
pre-existing text.

Model process tools cannot select from ambient `PATH`: `shell.run` resolves `argv[0]`
against exact configured executable identities, and Git tools require exactly one
configured executable named `git`. Shell interpreters are rejected even if configured.
Git diff/show disable external diff and text-conversion helpers; pathspecs must be
workspace-relative and cannot enter `.git`/`.colossus`, while revisions cannot begin with
an option or contain unsupported characters. Git inspection and `shell.run` have distinct
policy identities so allowing read-only Git does not authorize arbitrary commands.

Every model process result is quarantined and post-authorized. Exit code, bounded lossy
UTF-8 stdout/stderr, cwd, and truncation state are returned to the model. A nonzero exit
does not mean the adapter lost control or the outcome is unknown; it is journaled as a
completed process effect with the nonzero code in the released tool result. Timeouts,
resource-limit enforcement, helper failure, and unconfirmed OCI cleanup retain their
failure or `outcome_unknown` handling.

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
| `plan.create/show` | None | Denied | No | Enabled, session-persisted |
| `plan.approve_request` | None | Denied | Yes | Enabled, session-persisted |
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

Python 0.5 context snapshots are derived from raw session messages and persisted in SQLite. They are
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

Rust MCP servers are exact configured stdio process identities, never arbitrary commands
selected by a model. Configuration binds command, literal argv, working directory,
environment credential references, resource limits, and an exact tool allowlist.
`mcp.tools` still crosses policy, a one-use permit, native/OCI process isolation, and a
post-effect release decision even though the built-in policy allows discovery without a
prompt. `mcp.call` requires approval by default. Calls first rediscover the allowlisted
schema, validate the complete argument object locally, and bind that schema and content
into the authorized request. Raw environment values are resolved only inside the
permit-bearing adapter and exact values are removed if an MCP result echoes them. A
missing or malformed terminal result after a call starts is recorded as
`outcome_unknown`; it is never silently retried.

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
safe summary field. It does not render raw hidden reasoning text in CLI or REPL paths.
Stream chunks are normalized and individually post-authorized before becoming typed
provider events; raw provider chunks are neither persisted nor placed in run-event
envelopes.

The REPL transcript labels these safe summaries as `thinking` only when they arrive as
typed `ProviderEvent::ReasoningSummary` values. Tool results become `RunEvent::ToolCompleted`
only after the released result and canonical completion event exist. Semantic rendering
caps previews and identifies recoverable errors without reclassifying local harness
activity as model reasoning.

## REPL Themes And Preferences

Rust REPL preferences and submitted-input history are strict records in the encrypted
event journal. Updates cross the same policy, permit, and audit boundary as other durable
mutations and route through authenticated worker IPC when the worker owns the writer
lease. Reedline is hydrated into bounded memory and never receives a plaintext history
file; audit envelopes and projections do not disclose entry contents. Preferences control
display behavior only and do not change provider, policy, tool, approval, capability, or
prompt decisions. Built-in themes are data-only identities; executable plugins are not
loaded through the presentation path, and unknown schemas fail closed. Their fixed Rust
palettes can add ANSI styling only after the terminal interface confirms an interactive
terminal; redirected output remains control-sequence-free. The frozen Python implementation
retains its legacy SQLite preferences and plaintext history separately.

Custom Rust themes remain configuration-only data. The loader accepts only bounded JSON
or TOML from non-symlink config-adjacent and platform theme directories, caps individual
files and total theme count, rejects unknown fields and identity collisions, and parses
colors/styles/spinners into typed values. It never loads code or follows theme symlinks.
Selection stores the fully resolved palette and SHA-256 source hash in the encrypted
preference event, so restart does not reread mutable theme content to reconstruct the
active appearance. The supported data-only Python schema is mapped through the same
bounds during cutover.

Composer metrics are interface-local derived values. Reedline supplies the draft and
insertion point to a data-only highlighter during repaint; Colossus retains only cursor
line/column and draft character/line counts in memory. Draft text is not copied into
prompt status, worker IPC, telemetry, policy input, or audit events.

## Audit Logs

The frozen Python audit records are append-only hash-chained JSONL. The Rust canonical
journal is encrypted, hash chained, checkpoint-signed redb state and remains the source
of truth. Its optional directory exporter emits one strict ciphertext-free JSON evidence
record per non-exporter event through the permission boundary. Both forms are intended
for debugging, release review, and incident response. Operators should still treat all
audit material as sensitive operational evidence.

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
