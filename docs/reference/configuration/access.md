---
title: Access configuration
description: Configure model-visible tools and built-in action decisions with explicit profiles, selectors, overrides, and fail-closed prerequisites.
audience: operator
type: reference
---

# Access configuration

`access` answers two questions before an agent can use a capability:

1. Should the tool appear in the model-visible catalog?
2. Should the tool's exact action be allowed, denied, or require approval under the
   built-in policy?

It does not answer whether the runtime has permission to touch a file, start a process,
reach an origin, use a credential, or load an extension. Those authorities come from
the sandbox, provider or integration configuration, extension trust, policy
obligations, approvals, and one-use permits.

For the complete decision flow, see [Access and approvals](../../admin/access-and-approvals.md).
For canonical tool and action names, see [Tools and action classes](../tools-actions.md).

## How access resolution works

Colossus resolves the effective tool surface in this order:

| Stage | Result |
| --- | --- |
| Trusted catalog | Built-in tools plus configured integration operations and explicitly enabled plugin MCP tools |
| Profile | Selects the baseline tools and built-in action decisions |
| Tool overrides | Adds exact includes, then removes exact excludes |
| Prerequisites | Hides tools whose required declared-or-ambient filesystem, executable, network, search, UI, or MCP authority is absent |
| Runtime mode | Plan Mode, Goal Mode, workflow lineage, and child scope may narrow the catalog further |
| Effect authorization | Policy, approval, the Safety Kernel, a permit, sandbox obligations, quarantine, and post-effect policy govern each call |

Each stage can narrow authority. A later stage cannot use a broad access profile to
repair a missing resource grant or bypass an earlier trust decision.

## Choose a starting profile

| Scenario | Recommended profile | Guidance |
| --- | --- | --- |
| Offline smoke test | `minimal` | Expose effect-free support tools and keep non-provider effects denied |
| Interactive repository development | `development` | Use with reviewed sandbox resources and explicit approval handling |
| Production or narrowly scoped agent | `pinned` | Name every model-visible tool and opt in to its exact actions separately |
| Ordinary sparse developer configuration | `allow_all` | Removes built-in approval gates; paired with the separate full-access schema default |
| Disposable, tightly sandboxed test environment | `allow_all` | Pair with an explicit isolating execution boundary |
| OPA-controlled deployment | Usually `pinned` or `development` | Use the profile for tool selection; leave all local action override lists empty |

`allow_all` is the default profile, and the root `access` block may be omitted. Child
fields may also be omitted independently; the complete explicit shape is:

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

Unknown fields are rejected.

## Access profiles

Profiles are metadata-driven baselines. Tool includes and action overrides are applied
after the baseline.

| Profile | Baseline tool selection | Baseline built-in action decision |
| --- | --- | --- |
| `minimal` | Tools without an effect action | Provider actions allowed; every other effect denied |
| `development` | Every applicable trusted candidate tool | Provider, read, and Colossus local-state actions allowed; workspace mutation, execution, external network, and administration require approval |
| `allow_all` | Every applicable trusted candidate tool | Every registered trusted action allowed |
| `pinned` | Exact entries from `tools.include` only | `provider.echo` allowed; every other action denied |

“Applicable” means the tool's prerequisites are currently satisfied. A profile does not
create those prerequisites.

### `minimal`

Use `minimal` for an offline health check or an interface that should not offer normal
effectful tools. Exact includes can still make an effectful tool visible, but the
profile continues to deny its action until an action override explicitly changes the
decision.

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

### `development`

Use `development` for interactive engineering work. Read and Colossus-owned state
operations proceed under their normal obligations. Workspace mutation, command
execution, external network access, and administration require approval.

Select `development` and `sandbox.profile: workspace-development` explicitly when you
want the older approval-gated, workspace-isolated development posture. Access and the
execution boundary remain independent.

### `allow_all`

`allow_all` changes built-in action outcomes from approval-required to allowed. The
separate schema default of acknowledged `danger_full_access` supplies ambient resource
authority for process, structured filesystem, and canonical HTTP(S) effects. It does
not invent credentials, provider/model routes, extension trust, configured MCP servers
or tools, integration operations, action identities, or permits.

Treat that combined default as full host access. Select a platform-isolating sandbox
when the blast radius must be smaller.

### `pinned`

`pinned` is deny-by-default on both dimensions:

- Only named tool includes are selected.
- Only `provider.echo` is allowed by the profile's action baseline.

Selecting a tool does not allow its action. A practical pinned configuration therefore
usually names both the tool and the corresponding action:

```yaml
access:
  profile: pinned
  tools:
    include:
      - filesystem.list
      - filesystem.read
      - filesystem.search
      - git.status
      - git.diff
      - git.show
      - repo.map
      - repo.file_summary
    exclude: []
  actions:
    allow:
      - filesystem.list
      - filesystem.read
      - filesystem.search
      - git.status
      - git.diff
      - git.show
      - repo.map
      - repo.file_summary
    requireApproval: []
    deny: []
```

Under an isolating boundary, this exact example also requires a readable filesystem
grant and exactly one configured or derived Git executable. Acknowledged full access
supplies those resource prerequisites but does not change the pinned action list. A
non-echo provider requires its exact provider action to be opted in as well. Inspect the
resolved names instead of guessing them.

## Tool selection

`access.tools` changes what the model can see. It does not directly change the action
decision for any selected tool.

| Field | Meaning | Default |
| --- | --- | --- |
| `include` | Exact tools added to the profile selection, or `"*"` as the sole include selector | `[]` |
| `exclude` | Exact tools removed after profile and include selection | `[]` |

### Exact selector rules

- Entries must be nonempty and unique.
- Every exact name must exist in the trusted runtime catalog.
- The same exact tool cannot appear in both lists.
- `include: ["*"]` is valid, but `"*"` must be its only include entry.
- `exclude` never accepts `"*"`.
- Exact excludes may accompany `include: ["*"]` to express “all except these tools.”
- Excludes win over profile selection and wildcard inclusion.

For example:

```yaml
access:
  profile: pinned
  tools:
    include: ["*"]
    exclude: [shell.run]
  actions:
    allow: []
    requireApproval: []
    deny: [shell.run]
```

This exposes every applicable trusted candidate except `shell.run`, but most effectful
calls remain denied by the `pinned` action baseline. The explicit action deny also
prevents the `shell.run` action outside that model-visible selection.

### Includes and excludes are not action decisions

A visible tool may still be denied. A hidden tool's action may still be used by a
separate operator or runtime operation if that path is authorized. When a capability
must be unavailable at both boundaries, hide its tool and deny its exact action:

```yaml
access:
  profile: development
  tools:
    include: []
    exclude: [shell.run]
  actions:
    allow: []
    requireApproval: []
    deny: [shell.run]
```

### Prerequisite behavior

Prerequisites are availability checks derived from other configuration. They never
grant the underlying resource.

| Prerequisite | Example tools | Satisfied by |
| --- | --- | --- |
| Filesystem read | `filesystem.read`, repository tools, `patch.preview` | At least one read-, metadata-, or write-capable filesystem grant, or acknowledged ambient authority |
| Filesystem write | `filesystem.write`, `patch.apply`, `trace.export` | At least one write-capable filesystem grant, or acknowledged ambient authority |
| Git executable | `git.status`, `git.diff`, `git.show` | Exactly one configured or derived Git executable, or Git on ambient `PATH` under acknowledged `danger_full_access` |
| Any executable | `shell.run` | At least one configured or derived exact executable, or acknowledged `danger_full_access` |
| Model network tools | `web.fetch`, `docs.fetch`, `network.http` | The runtime host enables generic model-visible direct fetch tools; Desktop Managed Local **Offline isolated** deliberately withholds this prerequisite |
| Network destination | `web.fetch`, `docs.fetch`, `network.http` | At least one sandbox network destination, or acknowledged ambient authority |
| Agent search route | `web.search` | A valid top-level `search.roles.agent` route |
| Interactive interface | `user.ask` | A trusted prompt-capable interface for the current runtime |
| MCP server | `mcp.servers`, `mcp.tools`, `mcp.call` | At least one configured and trusted MCP server |

Provider service and authentication transports are independent from the model-network
tool prerequisite. Managed Local **Offline isolated** retains the configured provider's
exact service and authentication/refresh destinations while keeping `web.fetch`,
`docs.fetch`, and `network.http` hidden. Search, MCP, and integration adapters remain
independently governed by their own routes, declarations, trust, and host grants.

When a profile or wildcard selects a tool with an unmet prerequisite, Colossus keeps it
hidden and reports the reason in `config effective`. A named exact include is a stronger
operator assertion: an unmet non-interactive prerequisite fails runtime composition
instead of silently ignoring the selection. `user.ask` may remain hidden when an
otherwise valid process has no interactive interface.

An unknown exact name also fails closed. Configure and trust an integration or plugin
before referring to one of its dynamic tool names.

### Wildcard boundary

`access.tools.include: ["*"]` automatically selects current and future trusted
candidate tools registered in that runtime. This is intentionally broader than an
explicit list and should be reviewed after upgrades or extension changes.

It is independent from similarly spelled wildcards:

| Wildcard | Boundary |
| --- | --- |
| `access.tools.include: ["*"]` | Colossus model-visible tool catalog |
| `mcp.servers.*.allowedTools: ["*"]` | Remote tools discovered from one configured MCP server |
| `sandbox.networkDestinations: ["*"]` | Public HTTP(S) network origins permitted by the sandbox |

Enabling one does not enable either of the others.

## Action overrides

`access.actions` changes the built-in authorization result for exact trusted action
names. It applies independently from tool visibility.

| Field | Built-in result | Typical use |
| --- | --- | --- |
| `allow` | Proceed to remaining policy and enforcement checks | Opt a pinned action in or loosen one development action deliberately |
| `requireApproval` | Require a request-bound approval proof before reevaluation | Add an approval gate to an otherwise allowed action |
| `deny` | Reject the action | Tighten any profile, including `allow_all` |

All entries must be nonempty, unique, exact registered action names. The three lists
must be pairwise disjoint. Action wildcards are unsupported; select `allow_all` when
that broad built-in baseline is truly intended.

An action not named in an override list receives its profile decision. Overrides can
therefore tighten or loosen a profile one action at a time:

```yaml
access:
  profile: development
  tools:
    include: []
    exclude: []
  actions:
    allow: []
    requireApproval:
      - context.compact
      - context.restore
    deny:
      - shell.run
```

Here, context snapshot changes require approval even though `development` normally
allows local-state actions, while shell execution is denied rather than merely
approval-gated.

### Tool names and action names can differ

Most effectful built-ins use the same tool and action name. Important exceptions are:

| Tool | Effect action |
| --- | --- |
| `echo`, `user.ask`, `tool.search`, `trace.show`, `mcp.servers` | None |
| `filesystem.replace` | `filesystem.write` |
| `agent.delegate` | `subagent.create` |
| `agent.result` | `subagent.read` |
| `agent.list` | `subagent.list` |
| `web.fetch`, `docs.fetch`, `network.http` | `network.http` |
| `mcp.tools` | `mcp.tools` |
| `mcp.call` | `mcp.call` |

Connected integrations and explicitly enabled plugin MCP servers add action identities
from the active plugin snapshot and workspace allowlists. Use `tools list` and
`config effective` for the exact runtime catalog.

## Action classes and profile defaults

Every trusted action has one stable behavior class:

| Class | Examples | `development` | `minimal` | `allow_all` | `pinned` |
| --- | --- | --- | --- | --- | --- |
| Provider | Model generation and provider catalog calls | Allow | Allow | Allow | Deny except `provider.echo` |
| Read | Filesystem, Git, repository, memory, context | Allow | Deny | Allow | Deny |
| Local state | Tasks, decisions, plans, goals, snapshots | Allow | Deny | Allow | Deny |
| Workspace mutation | File writes and patching | Approval | Deny | Allow | Deny |
| Execution | Shell, processes, workflows, plan execution | Approval | Deny | Allow | Deny |
| External network | HTTP, search, integrations, MCP calls | Approval | Deny | Allow | Deny |
| Administration | Installation, trust, registry, protected export | Approval | Deny | Allow | Deny |

The class provides a profile default; exact overrides still win under built-in policy.
See the canonical action table in [Tools and action classes](../tools-actions.md).

## Approval interaction

`requireApproval` creates an approval obligation. The global `--approval-mode` controls
how an existing obligation can be satisfied:

| Approval mode | Result for an approval-required action |
| --- | --- |
| `deny` | Fail closed without prompting |
| `ask` | Prompt through a trusted interactive interface |
| `risk-auto` | Auto-approve only eligible, valid low-risk shell, read-only network, and exact top-level MCP assessments; otherwise ask or deny |
| `full-access` | Satisfy the approval obligation without a prompt |

Approval mode never converts `deny` to `allow` and never creates a missing resource
grant. `full-access` is therefore not equivalent to `access.profile: allow_all`, and
neither setting bypasses the Safety Kernel or sandbox.

## Access with OPA

When `policy.kind: opa`, OPA owns every action outcome. All local action override lists
must be empty:

```yaml
access:
  profile: pinned
  tools:
    include:
      - filesystem.list
      - filesystem.read
      - git.status
      - git.diff
    exclude: []
  actions:
    allow: []
    requireApproval: []
    deny: []
```

The profile and tool selectors still determine model visibility. The profile's built-in
action baseline is replaced with `external_policy` decisions, and OPA must return the
complete authorized resource obligations. Local Safety Kernel, permit, sandbox,
quarantine, audit, and post-effect checks remain mandatory.

See [Policy and audit configuration](policy-audit.md) and
[Policy and OPA](../../admin/policy-opa.md).

## Access does not configure resources

| Need | Configure it in |
| --- | --- |
| Read or write a path | Ambient authority under acknowledged full access, otherwise `sandbox.filesystem` or the reviewed `workspace-development` profile |
| Run a command | Ambient executable, filesystem, and environment authority under acknowledged full access; otherwise `sandbox.executables` plus the appropriate grants |
| Reach a service | Ambient HTTP(S) authority under acknowledged full access, otherwise `sandbox.networkDestinations`; the owning provider, search, MCP, or integration declaration always remains required |
| Read an environment variable in a subprocess | Ambient environment authority under acknowledged full access; otherwise `sandbox.environment` and the effect's own declaration |
| Use provider, MCP, or integration credentials | Credential references in the owning adapter configuration |
| Load a plugin or integration operation | Its installation, trust, enablement, connection, and declaration lifecycle |
| Make a tool available in Plan Mode | Nothing can widen Plan Mode; it applies a fixed subset after access resolution |

Review [Sandbox configuration](sandbox.md), [Network configuration](network.md), and
[Extension configuration](extensions.md) alongside access for effectful deployments.

## Common configuration mistakes

| Symptom | Check |
| --- | --- |
| A profile-selected tool is missing | Inspect its `unmet_prerequisite` in `config effective` |
| Runtime startup fails after adding an include | The exact tool may be unknown or missing a non-interactive prerequisite |
| A visible tool call is denied | Tool visibility and action authorization are separate; inspect its exact effect action |
| An excluded capability still works through another interface | Exclusion hides a model tool; add an exact action deny when the action itself must be blocked |
| A pinned tool is visible but denied | Add its exact action to `allow` or `requireApproval` |
| A pinned hosted-model run cannot start | Allow the exact provider action used by the configured provider |
| `shell.run` is hidden under development | Configure an exact executable, use the reviewed `workspace-development` sandbox profile, or explicitly select and acknowledge `danger_full_access` |
| `web.search` is hidden despite network access | Configure an exact `search.roles.agent` route; a destination alone is insufficient |
| `user.ask` is hidden in a worker or headless run | The current runtime has no trusted interactive prompt interface |
| A configured capability is hidden under full access | Add the required credential, route, MCP/integration declaration, or trusted extension; full access does not invent capabilities |
| OPA configuration is rejected | Empty all three `access.actions` lists; OPA is the sole decision point |
| A wildcard/name mixture is rejected | `"*"` must be the only `include` entry; put exceptions in `exclude` |
| An MCP server tool remains unavailable | Configure the MCP server's own `allowedTools`; the access wildcard is a separate boundary |
| A tool disappears in Plan Mode | Runtime modes can narrow the resolved catalog and cannot be widened by access configuration |

## Validate the result

First parse the strict configuration:

```bash
colossus --config .colossus/config.yaml config show
```

Then inspect both the complete access resolution and the active model-visible catalog:

```bash
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml tools list
```

`config effective` includes active and hidden tools, selection reasons, unmet
prerequisites, exact action classes and decisions, explicit and derived sandbox grants,
the canonical workspace, and wildcard meaning. `tools list` includes active schemas,
effect actions, capabilities, decisions, and output bounds.

For effectful changes, also verify the independent enforcement boundaries:

```bash
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
```

Run those commands in the same workspace and host mode used by the real agent because
derived development grants, interactive availability, configured executables, and
extension state affect the resolved catalog.

Return to the [configuration overview](../configuration.md).
