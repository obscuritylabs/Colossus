# Investigate security properties in source

Choose checks from the threat model and stack. These are investigation questions, not
automatic findings. Establish attacker control, an effective guard, a reachable sensitive
operation, and meaningful impact for every concern.

## Source-reading loop

1. Find a concrete entry or sensitive operation, definition, callers, implementation,
   and configuration. Prefer `rg`, symbol search, and references to whole-tree dumps.
2. Read bounded line-numbered excerpts; expand around branches/callees as needed. A
   keyword match narrows work but does not complete a review.
3. Follow input, identity, tenant, path, and destination transformations; inspect guard
   order, execution grammar, errors, transitions, and effects.
4. Compare siblings and alternate adapters. Seek the strongest safe explanation before
   recording a candidate. Retain counterevidence and unresolved dependency semantics.

## Authentication, authorization, and business logic

- Follow authenticated identity to the effect. Inspect session expiry/revocation,
  token issuer/audience/algorithm, recovery flows, and impersonation.
- Check ownership, tenant binding, roles, capabilities, and field access for reads,
  mutations, exports, subscriptions, bulk calls, and asynchronous jobs. Authentication
  alone is not object authorization; random IDs do not fix broken access control.
- Inspect alternate/admin routes, stale permissions, mass assignment, and ownership
  changes between validation/use. Compare related operations for missing guards.
- Inspect quota/payment/invitation/approval state for replay, non-atomic one-use
  consumption, unusual ordering, and concurrent-request bypass.

## Injection and interpretation

- Trace inputs into SQL/NoSQL, shells, templates, expressions, code generation,
  deserializers, XML, FFI, and interpreters. Establish the exact grammar and whether
  parameterization or escaping fits that context.
- Shell-free argument arrays can still permit option injection, response files, unsafe
  executable selection, or the program's own execution features.
- Check DOM/HTML/Markdown/URL rendering and serialized state for execution in a
  privileged browser origin. Inspect the actual sanitizer and sink.
- Inspect prototype manipulation, object merging, property paths, and schema/reference
  resolution when untrusted data can alter behavior or bypass controls.

## Files, archives, and processes

- Follow complete paths through decoding, normalization, joins, containment, and final
  open/write. Check traversal, absolute paths, symlinks/hardlinks, check/use races,
  archive extraction, overwrites, and downstream rendering/execution of uploads.
- Consider supported platforms: drive/UNC paths, separators, case, reserved names,
  and reparse points. Do not transfer Unix assumptions to Windows paths.
- Inspect executable lookup, inherited environment/descriptors, working/temp directories,
  process groups/jobs, termination, cleanup, and OS-enforced versus best-effort limits.
- Connect upload type/size validation to storage, extraction, and later use. Filename
  extension validation alone does not secure a downstream parser or executable sink.

## Network and browser boundaries

- Trace URLs through scheme, authority, credentials, DNS, redirects, proxies, and actual
  connection. Check SSRF, private/metadata destinations, rebinding, and disagreement
  between the authorized origin and connected address.
- Inspect TLS validation, origins/headers, framing, proxy trust, cookies, CSRF, CORS,
  cross-origin messages, redirects, and cache keys. Name the affected security property.
- Verify webhook signatures over correct bytes, replay controls, callback identity,
  time/output bounds, streaming behavior, and trust in upstream responses.
- Separate development and externally terminated TLS from production. A missing
  setting in one file does not establish exposure or defeat an external control.

## Secrets, cryptography, and disclosure

- Trace secret references through resolution, recipients, logs/errors, caches,
  telemetry, model context, exports, and artifacts. Inspect release/redaction on both
  success and failure. Do not read or reproduce credential values.
- Use redacted scanner output and file/rule references. Distinguish synthetic fixtures,
  public identifiers, and opaque handles from literal credentials.
- Inspect key ownership/rotation, authenticated encryption, nonce uniqueness,
  randomness, comparisons, trust roots, signatures, and downgrade rules against the
  actual library/protocol contract. Avoid inventing custom cryptography.

## Availability, concurrency, and recovery

- Identify a shared protected resource before claiming DoS: CPU, memory, connections,
  storage, queues, children, provider spending, or critical scheduling capacity.
- Trace bounds before allocation/work, compressed/recursive input, parsing/regex
  complexity, total versus inactivity deadlines, and accumulation across calls.
- Inspect locks, cancellation, duplicate delivery, crash points, retries, idempotency,
  audit durability, and uncertain effects. A timeout does not prove an effect failed.
- Do not exclude all resource exhaustion. Establish attacker-driven boundary impact;
  distinguish costly authorized use from a bypass of an actual limit or guarantee.

## Plugins, agent tools, and supply chain

- Trace untrusted documents/repositories/tool output to authority-bearing actions.
  Identify which resource, identity, approval, payload, or destination becomes attacker
  controlled. A generic prompt-injection story without an unauthorized effect is a lead.
- Inspect canonical tool schemas/revalidation, child permissions, credential brokers,
  contained plugin roots, immutable digests, trust profiles/signing identity, archive
  extraction, registries, and credential helpers. Declarations are not authority grants.
- When scoped, inspect CI triggers, untrusted PR inputs, interpolation, downloads,
  dependencies, provenance, and release authority. Prove the controlled input reaches
  a privileged job/artifact; do not flag privileged-only behavior without a gain.

## Select stack-specific checks

| Stack | Implementation-specific behavior |
| --- | --- |
| Rust | `unsafe`/FFI contracts and controlled lengths/lifetimes; concrete trait adapters; panic/allocation paths; integer conversions; `Command` selection/arguments; async cancellation and ownership across awaits. Safe Rust does not prove authorization or containment. |
| JavaScript/TypeScript | Runtime validation beyond types; server/client boundaries; DOM/URL sinks; prototype/object merging; executable dependencies; handlers and tenant/session checks; SSR and client-state serialization. |
| Python | Query/template/subprocess APIs; pickle/object loaders; auth/CSRF middleware ordering; path/archive helpers; unsafe YAML; dynamic import/eval and background tasks. |
| Go | Handler/middleware ordering; deadlines and goroutine cleanup; path/URL normalization; template contexts; `os/exec`; allocation/conversion bounds; body/connection limits. |
| Other native/JVM/.NET stack | Actual framework parser, memory, auth, serialization, path, and process contracts. State missing expertise/source rather than importing assumptions from a different language. |

Use available framework references for exact API semantics. External lookup depends on
existing authorization/network availability; offline reviews must retain unknowns.

## Use analyzers as supporting evidence

Prefer installed tools and inspected configurations. Semgrep/CodeQL, compiler/linter
diagnostics, dependency tools (`cargo audit`, `cargo deny`, `npm audit`, `pip-audit`,
`govulncheck`), and redacting secret scanners may help. Check behaviors first: advisory
tools may fetch data, analyzers may run builds or plugins, and rules may trigger downloads.
Prefer existing offline databases where available. Do not change permissions to run them.

Record version/config, scope, command, status, relevant output, and skipped/truncated
work. Avoid whole-repo jobs that displace verification. Keep large output in scratch.
Validate versions, features, actual call paths, prerequisites, and compensating controls;
dependency presence or a pattern match alone is not an exploitable product finding.
