---
title: Troubleshooting
description: Diagnose configuration, providers, access, sandbox, worker, state, and search failures.
audience: operator
type: reference
---

# Troubleshooting

Begin with bounded, credential-safe diagnostics:

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml config effective
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
colossus --config .colossus/config.yaml projection status
colossus --config .colossus/config.yaml provider profiles
colossus --config .colossus/config.yaml models routes
colossus --config .colossus/config.yaml search profiles
colossus --config .colossus/config.yaml tools list
```

## Symptom map

| Symptom | First check | Common cause | Safe action |
| --- | --- | --- | --- |
| Configuration does not parse | `config show` | Unknown field, missing `access`, removed exact tool/action fields, overlapping access entries, relative security path | Compare with [Configuration fields](../reference/configuration.md); for an incompatible shape, follow [Upgrade and compatibility](../get-started/upgrade-compatibility.md) |
| Echo works; model fails | `provider doctor PROFILE` | Route, credential reference, origin, TLS, model ID, or response shape | Repair the first failing obligation |
| Effect is denied | `config effective` | Exact deny or unmet policy obligation | Change the reviewed action decision; approval cannot override deny |
| Approval never appears | Global option placement | `--approval-mode` placed after subcommand or noninteractive surface | Put the global flag before the subcommand |
| Tool is missing | `config effective` | Profile exclusion, exact exclude, missing static prerequisite, untrusted extension | Fix selection or prerequisite; do not widen unrelated controls |
| Repository path is wrong | `config effective` canonical workspace | Missing or incorrect global `--workspace` | Retry with `-w /canonical/repository`; relative config resolves from it |
| Worker rejects the client | `worker --status` workspace | Client and worker selected different canonical workspaces | Restart one side with the same `--workspace`; mismatch is never silently accepted |
| Shell tool is missing | `config effective` sandbox report | `offline-default`, no explicit executable, unsupported protection, or workflow scope | Use `workspace-development` for an eligible actor or add exact grants |
| Shell is denied | Action decision and approval mode | `development` requires approval for execution | Use `ask`, or reviewed `risk-auto` for eligible non-workflow shell calls |
| Linux protected-path probe fails | `sandbox doctor` native details | Ubuntu AppArmor restricts capabilities in unprivileged user namespaces | Install the release archive's exact-path profile against a root-owned Colossus binary, or use OCI; never weaken the host-wide restriction |
| Public request is denied | `sandbox doctor` destinations | `*` never matches loopback/private/link-local/metadata | Add the exact canonical private origin only when intended |
| Worker is unavailable | `worker --status` | Writer lease, stale endpoint, key/permission mismatch, incompatible protocol | Preserve state; stop or repair the owning worker |
| Read-only recovery | `audit verify` and `audit anchor-status` | Chain, checkpoint, anchor, decryption, or projection-position failure | Preserve evidence and investigate; never rewrite canonical events |
| Memory search degraded | `memories index status` | Disposable index unavailable or behind | `sync` or explicitly `rebuild`; canonical records remain |
| Web search hidden | `search profiles` and `config effective` | Missing role route, tool selection, action, or exact origin | Repair the explicit route and obligations |

## Approval-required invocation

Global flags precede the command:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  run "Apply the approved change"
```

For workspace-aware development:

```bash
colossus -w /absolute/path/to/repository \
  --config .colossus/config.yaml \
  --approval-mode risk-auto tui
```

## Unknown outcomes

`outcome_unknown` means an effect may have escaped after it started but before a terminal
event was durable. Investigate the external system and use only an operation-specific
recovery path. Never blindly rerun a non-idempotent request.

## Safe issue reports

Include the exact command, bounded doctor output, run ID, action, policy decision
ID/revision, and audit sequence. Exclude API keys, tokens, authorization headers, private
keys, key material, decrypted payloads, hidden reasoning, and unredacted quarantined
content. Follow the repository's root security policy for vulnerability reports.
