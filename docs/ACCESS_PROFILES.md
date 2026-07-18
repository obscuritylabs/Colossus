# Unified Access Profiles

Colossus resolves model-visible tools and built-in action decisions from one required,
metadata-driven `access` block. A profile is an operator convenience, not a bypass: tool
schemas, extension trust, prerequisites, policy, approvals, the Safety Kernel, one-use
permits, sandbox grants, quarantine, output bounds, and post-effect authorization remain
independent checks.

![Access profile resolution](diagrams/access-resolution.svg)

The editable source is
[`diagrams/access-resolution.drawio`](diagrams/access-resolution.drawio). The checked-in
SVG is exported from that source for GitHub and mdBook.

## The Three Separate Questions

Access resolution deliberately answers three different questions:

1. **Visibility:** may the model see this tool in the current run?
2. **Authorization:** if called, does built-in policy allow, deny, or require approval
   for the exact action?
3. **Enforcement:** do trust, the Safety Kernel, a one-use permit, and the sandbox permit
   this exact adapter effect and resource?

Adding a tool to `access.tools.include` answers only the first question. An action can be
visible and still be denied. Likewise, `allow_all` can allow a registered action but
cannot invent a filesystem root, executable, network origin, trusted extension, or
configured provider route.

## Required Configuration

```yaml
schemaVersion: 1

access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []

agent:
  maxTurns: 24

policy:
  kind: built_in
  require_post_effect: true
```

`access` is required. The old `agent.tools`, `policy.allow_actions`, and
`policy.approval_actions` fields are no longer supported and are rejected by the strict
configuration parser.

## Profile Matrix

| Profile | Model-visible tools | Built-in action default |
| --- | --- | --- |
| `minimal` | Pure support tools; contextual support only when applicable | Configured provider calls allowed; other effects denied |
| `development` | Applicable core tools and configured, trusted extensions | Provider calls, reads, and Colossus-owned state changes allowed; workspace mutation, execution, external network, installation, and administration require approval |
| `allow_all` | All applicable trusted tools | All registered trusted actions allowed |
| `pinned` | Only exact includes | Denied except `provider.echo`; exact action overrides opt in |

`development` is the default for new configurations. It is designed to
let new safe reads appear without repeatedly editing a hard-coded catalog while keeping
new mutation, execution, and external-network actions approval-gated.

### Minimal

```yaml
access:
  profile: minimal
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
```

Use `minimal` for chat, provider connectivity, and low-surface onboarding. It does not
inherit effectful reads or mutations.

### Development

```yaml
access:
  profile: development
  tools:
    include: []
    exclude:
      - shell.run
  actions:
    allow: []
    requireApproval: []
    deny:
      - integration.invoke
```

Use `development` for ordinary repository work. Applicable tools are inherited from
capability metadata; unavailable tools are hidden rather than offered to the model.

### Allow All

```yaml
access:
  profile: allow_all
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
```

Use `allow_all` only in a deliberately bounded environment. It removes built-in approval
friction for registered trusted actions, but all hard safety, trust, sandbox, permit,
quarantine, and post-effect checks still apply. It does not discover or trust arbitrary
MCP servers, packs, integrations, executables, or destinations.

### Pinned And Compliance-Oriented Operation

```yaml
access:
  profile: pinned
  tools:
    include:
      - echo
      - filesystem.read
      - filesystem.search
  actions:
    allow:
      - filesystem.read
      - filesystem.search
    requireApproval: []
    deny: []
```

Use `pinned` when an exact catalog is more important than inheriting future capability
metadata. Exact tool inclusion does not grant an action, so both lists are intentional.

## Tool Selection Rules

- `tools.include` and `tools.exclude` accept exact tool names.
- `tools.include: ["*"]` selects every applicable trusted tool.
- `tools.exclude` does not accept `*`.
- Include and exclude entries cannot overlap or contain duplicates.
- A tool inherited by a profile is hidden when a static prerequisite is missing.
- An exactly included tool with a missing static prerequisite is a configuration error.
- Interactive UI, Plan Mode, Goal Mode, and child-agent restrictions are applied after
  profile resolution and can only narrow the catalog.

Common prerequisites include an exact filesystem grant, a configured executable, an
agent search route, a network destination, an interactive prompt interface, or a
configured MCP server.

## Action Override Rules

Action overrides are exact and non-overlapping:

```yaml
access:
  profile: development
  tools:
    include: [filesystem.write]
    exclude: []
  actions:
    allow:
      - filesystem.write
    requireApproval:
      - context.restore
    deny:
      - network.http
```

`deny` is strongest, followed by `requireApproval`, then `allow`; overlapping entries
are rejected instead of relying on ordering. Action wildcards are not supported. Use
`profile: allow_all` when that behavior is intended.

An override changes only the built-in policy outcome. It cannot satisfy a missing
sandbox grant, capability classification, trust decision, approval proof, or post-effect
release.

## OPA

```yaml
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []

policy:
  kind: opa
  base_url: https://opa.internal.example
  decision_path: /v1/data/colossus/effect
  ca_pem_path: /etc/colossus/opa-ca.pem
  identity_pem_path: /etc/colossus/opa-client.pem
  full_content_disclosure_acknowledged: true
  decision_log_masking_verified: true
  timeout_ms: 5000
```

With OPA, the profile still selects tools and applies prerequisites. OPA is the sole
action decision point, so non-empty `access.actions` overrides are rejected. The Safety
Kernel, approval proof validation, one-use permits, sandbox, and post-effect release
remain local enforcement boundaries.

## Future Capability Inheritance

Every trusted capability has one descriptor containing its source, family, action class,
effect identity, Safety Kernel capability, and prerequisites. Profiles operate on that
metadata:

- a new core read is inherited and allowed by `development`;
- a new workspace mutation, execution, or network action is inherited but requires
  approval;
- `minimal` and `pinned` do not inherit effectful future tools;
- a connected integration or enabled signed pack can appear with approval;
- discovered, untrusted, disabled, unconfigured, unsigned, or disallowed extensions
  remain absent;
- a built-in capability without metadata fails startup closed.

This is how users gain safe access to newly shipped capabilities without changing a
hard-coded list while compliance configurations remain stable.

## Pre-1.0 Configuration Changes

Colossus does not provide a configuration migration command before 1.0. When the strict
configuration shape changes, update the YAML directly or generate a fresh configuration
at a separate path:

```bash
colossus --config ./config.new.yaml config init --access-profile development
```

Copy provider, storage, policy, sandbox, and integration settings deliberately, retaining
credential references rather than values. To preserve an exact tool catalog, select
`pinned` and populate both the exact tool includes and action overrides explicitly.
`config init` refuses to overwrite an existing file.

## Diagnostics

```bash
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml tools list
```

`config effective` reports every active or hidden candidate tool, source, family, action
class, decision, selection reason, and unmet prerequisite, plus every registered action.
It is bounded and credential-free. `tools list` retains the active tool schema,
effect-action, capability, and output-bound fields and adds profile source, family, risk
class, decision, and selection reason.

When a tool is missing, inspect `config effective` before changing policy or sandbox
configuration. When a visible call is denied, inspect the exact action decision next,
then the approval mode and sandbox obligations.
