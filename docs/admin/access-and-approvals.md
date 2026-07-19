---
title: Access and approvals
description: Understand how Colossus separates tool visibility, authorization, approval, and enforcement.
audience: operator
type: concept
---

# Access and approvals

Colossus resolves every capability through three separate questions:

1. **Visibility:** may the model see the tool?
2. **Authorization:** is the tool's exact action allowed, denied, or approval-required?
3. **Enforcement:** do trust, the Safety Kernel, a one-use permit, and the sandbox allow
   this exact effect and resource?

<div class="diagram-scroll" markdown tabindex="0" role="region" aria-label="Access resolution diagram">

![Access and resource resolution from configuration through enforcement](../diagrams/access-resolution.svg)

</div>

Reading the diagram without color: capability metadata and access configuration produce
the model-visible catalog and built-in decision; workspace plus sandbox configuration
produce separate resource obligations. Workflow lineage removes development inheritance.
Authorization, optional risk review, approval, the Safety Kernel, and a one-use permit
converge before the sandbox may reach the protected workspace or HTTP(S) egress proxy.
OPA replaces the built-in decision branch only. The editable source is
[`access-resolution.drawio`](../diagrams/access-resolution.drawio).

## Profile behavior

| Profile | Tool selection | Built-in action baseline |
| --- | --- | --- |
| `minimal` | Pure support; contextual support when applicable | Provider calls allowed; other effects denied |
| `development` | Applicable core and configured trusted extensions | Reads and Colossus state changes allowed; mutation, execution, external network, installation, and administration require approval |
| `allow_all` | All applicable trusted tools | Registered trusted actions allowed |
| `pinned` | Exact includes only | Denied except `provider.echo`; exact overrides opt in |

Tool inclusion never grants its effect action. An action override never supplies a
missing sandbox root, executable, origin, credential, extension trust decision, or
post-effect release.

## Exact override rules

- `tools.include` and `tools.exclude` contain exact tool names.
- `tools.include: ["*"]` selects all applicable trusted tools; excludes do not accept
  a wildcard.
- Action lists contain exact action names and cannot overlap.
- A deny is never repaired by changing the approval mode.
- Plan Mode, Goal Mode, interactive constraints, and child-agent scope can narrow the
  catalog but cannot widen it.

Use:

```bash
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml tools list
```

The first command explains selection and prerequisites. The second shows the active
schemas, effect identities, decisions, and output bounds. See
[Tools and action classes](../reference/tools-actions.md) for the canonical catalog.

## Approval modes

The global `--approval-mode` flag controls how an existing approval obligation is
satisfied:

| Mode | Behavior |
| --- | --- |
| `deny` | Fail closed without prompting |
| `ask` | Prompt on an interactive terminal |
| `risk-auto` | Automatically approve only eligible low-risk shell effects after review |
| `full-access` | Satisfy approval obligations automatically |

Approval modes do not convert policy denials into allows and do not add authority.

## Development shell

`development` keeps `shell.run` approval-required. It becomes visible when an executable
prerequisite exists; the `workspace-development` sandbox preset supplies the trusted
platform shell, Git when found, a read/write grant for the selected workspace, read-only
command/runtime roots, and an isolated `HOME`, temp directory, and sanitized `PATH`.

```bash
colossus -w /absolute/path/to/repository \
  --approval-mode ask tui
```

Use `--approval-mode risk-auto` only when the configured risk evaluator is trusted for
this role. Automatic proof minting is restricted to model or child-agent `shell.run`
outside workflow lineage, and only a valid low-risk `allow` assessment qualifies.
Medium, high, malformed, or unavailable assessments fall back to explicit approval or
denial. Process output remains quarantined and post-effect authorized.

`full-access` satisfies approval requirements without a prompt; it is not a sandbox
bypass. Prefer it only in bounded disposable environments.

## Workflows and OPA

Durable workflows, system actors, and agents carrying workflow lineage never inherit
`workspace-development` resources or `risk-auto` proof. They need exact configured
filesystem, executable, environment, and network grants.

OPA remains the sole action decision point when selected. Automatic
`workspace-development` grants are rejected with OPA; the OPA decision must return the
complete resource obligations, while local Safety Kernel, permit, quarantine, sandbox,
and post-effect checks remain mandatory.
