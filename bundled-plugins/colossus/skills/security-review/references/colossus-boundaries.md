# Colossus and brokered agent-runtime boundaries

Use for runtimes brokering model actions, plugins, processes, credentials, or external
effects. Inspected architecture/current source define guarantees; these questions do not
claim every runtime implements identical controls. In Colossus, start with
`docs/develop/security-architecture.md` and follow the owning implementation.

## Complete effect lifecycle

```text
input → schema/identity/resource binding → local checks → policy
→ approval and re-evaluation → authenticated one-use permit → adapter
→ quarantined result → post-effect release → terminal evidence
```

Inspect mandatory checks and alternate paths:

- Schema and cross-field checks before dispatch; canonical tool identity despite
  provider aliases/schema projections.
- Actor/application/session/workflow/tenant/child/resource/request binding. Model tool
  visibility must agree with authorization at invocation.
- Non-bypassable local restrictions, bounded/redacted policy inputs, and fail-closed
  behavior for unknown capabilities or malformed requests.
- Approval bound to the exact action/evidence, expiry/replay resistance, and policy
  re-evaluation; no alternate approval execution path or blanket grant.
- Permit authentication, actor/request/decision binding, expiry, atomic one-use
  consumption, constructor visibility, and mandatory adapter checks.
- Audit durability before effects and terminal/unknown outcomes. Never blindly replay
  external effects with uncertain outcomes after recovery.

Check concurrent calls, crashes, cancellation, stale decisions, and errors between stages.
Tests establish a guarantee only when they exercise its enforcing path.

## Containment and effective authority

- Follow roots to final descriptors/opens/writes; inspect protected control state,
  races, atomic replacement, archives/links, and platform variants.
- Distinguish native/OCI/Windows isolation, asserted host containment, ordinary process
  supervision, and acknowledged ambient authority. Evaluate each against its actual
  guarantee; intentional mode differences are not automatic escapes.
- Check executables, environment/descriptors, temp directories, process-tree containment,
  output framing, cleanup, and aggregate resource ceilings.
- Follow origins to DNS/TLS/connections/proxies. Inspect redirects, private/metadata
  destinations, credential URLs, ambient proxies, and exact versus public wildcard grants.
- Keep configured routes, trust, credential references, allowlists, and effect ceilings
  distinct from any ambient resource-authority mode.

## Quarantine, credentials, and observations

- Trace resolution and recipients; distinguish opaque handles and literal material,
  vault ownership, overlays, and fallback paths. Review references, not secret sources.
- Follow sensitive bytes through errors/streaming/telemetry/journal/observers/diagnostics
  and denied output. Denial must not release bytes through another channel; intentional
  operator diagnostics have their own boundary.
- Inspect aggregate model-observation bounds as well as single results. Tool metadata
  must not forge trusted provenance or reset cumulative budgets.
- Compare encryption, keyless integrity, rollback anchors, debug partitions, and recovery
  with the configured protection tier. A plaintext development tier is not confidentiality.

## Plugins, children, and durable work

- Verify immutable digest, contained resources, signature identity/trust roots,
  selected-root grants, explicit tool enablement, registry origins, and helper permits.
  Names, signatures, and skill selection alone must not grant effect authority.
- Children must inherit only allowed tools/actor/resource scope, and recovery must
  preserve accepted instructions, plugin snapshot, and lineage.
- Colossus removes `agent.delegate` from child discovery; inspect alternate entry paths
  before alleging recursive-delegation bypass.
- Inspect task/goal/schedule ownership, webhook/subscription identity, cancellation,
  leases, restart, and evidence ownership. Separate durable work from uncertain effects.

For an alleged bypass, show the entry point, enforcing boundary, actual capability gain,
and effect. A missing helper call alone is insufficient if a shared gateway enforces
the invariant elsewhere.
