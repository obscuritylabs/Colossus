---
title: Integrations
description: Connect a supported service or import OpenAPI operations without exposing credentials to the model.
audience: operator
type: how-to
icon: lucide/plug
---

# Integrations

## Goal

Connect one least-privilege service credential by reference, expose its strict operations
only after connection, and invoke an operation through the effect gateway.

## Prerequisites

- A supported GitHub or SearXNG endpoint, or a JSON OpenAPI 3 document.
- A least-privilege credential supplied through an environment reference.
- Under an isolating boundary, the exact service origin in
  `sandbox.networkDestinations`.
- An access profile and policy decision for the intended integration actions.

## Steps

### 1. Inject a credential reference

For GitHub, expose a token scoped to the repositories you need without placing
its value in command history:

=== "macOS and Linux"

    ```bash
    printf "GitHub token: "
    IFS= read -rs GITHUB_TOKEN
    printf "\n"
    export GITHUB_TOKEN
    ```

=== "Windows PowerShell"

    ```powershell
    $secret = Read-Host "GitHub token" -AsSecureString
    $env:GITHUB_TOKEN = [System.Net.NetworkCredential]::new("", $secret).Password
    ```

Do not place the token value in YAML or command arguments.

### 2. Connect the adapter

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations connect github \
  --credential-reference env:GITHUB_TOKEN
```

The native GitHub connector uses `https://api.github.com`; grant that exact origin under
an isolating boundary. Acknowledged full access supplies ambient destination authority
but still requires the connection and credential. A connection remains hidden from
model tools until its canonical connect event is active.

### 3. Inspect the released surface

```bash
colossus --config .colossus/config.yaml integrations show github
colossus --config .colossus/config.yaml tools list
```

The GitHub connector provides bounded operations including `github.repos`,
`github.issues`, `github.pull_requests`, `github.pull_request`, `github.checks`,
and `github.releases`. If an existing connection lacks a newly added operation,
run the connect command again with the same endpoint, credential reference, and
other connection settings. Start a fresh runtime after reconnecting so it loads
the updated operation catalog.

### 4. Invoke one read operation

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations call \
  github.repos '{"visibility":"private","max_results":20}'
```

The credential broker resolves the reference only after authorization and injects it
inside the adapter. Results remain quarantined until release. Integration invocation is
an external-network action under the development access profile, so the global
`--approval-mode ask` option enables the required terminal approval prompt.

## Inspect a pull request and its head checks

Supply the base repository owner and name and the pull request number.

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  integrations call github.pull_request '{"owner":"OWNER","repo":"REPO","number":123}'
```

In the TUI, enter the command with inline JSON. The active permission mode
governs the call.

```text
/integration call github.pull_request {"owner":"OWNER","repo":"REPO","number":123}
```

For a private repository, use a compatible credential such as a GitHub App token
scoped to that repository with Pull requests, Checks, and Commit statuses read
permissions. See [pull request permissions](https://docs.github.com/en/rest/pulls/pulls#get-a-pull-request),
[check run permissions](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference),
and [commit status permissions](https://docs.github.com/en/rest/commits/statuses#list-commit-statuses-for-a-reference).

The connector returns the PR state, draft flag, link, and `observed_head_sha`.
It reads up to three pages of ten rows from each evidence source and returns
check runs and the latest observed commit status per context for that SHA.
It then reads the PR again to check whether the head moved. A `stale` result
retains evidence for the first observed SHA. When `freshness` is `unverified`,
the connector could not confirm the current head.

Use `evidence_state` to distinguish complete, empty, incomplete, or stale results.
Check runs and commit statuses report any reasons their evidence is incomplete.
The connector queries only the requested base repository and marks evidence
incomplete for a fork or unknown head repository. Use `has_observed_pending` to
see whether the returned evidence includes pending work. GitHub repository rules
determine which checks are required and whether the PR can merge.

## Expected result

The connection summary contains no raw credential. The invocation returns a bounded
normalized result and records the authorized network lifecycle.

## Verification

Run `config effective` and confirm the operation's source, action class, decision, and
selection reason. Inspect recent audit evidence and verify that the secret value is
absent.

## Failure path

- **Connection stays hidden:** inspect the canonical connection, access profile, and
  exact tool prerequisite.
- **Credential unavailable:** check the environment of the Colossus process without
  printing the value.
- **Origin mismatch under isolation:** grant the exact scheme, host, and effective port.
- **OpenAPI import is rejected:** external references, embedded origins, unsupported
  schemas, and unknown arguments fail closed.
- **Outcome is unknown:** reconcile the external service before repeating a mutation.

## Next step

Use [MCP](mcp.md) for an exact configured stdio server. Exact integration and OpenAPI
formats live in [Agent Plugin formats](../reference/extension-formats.md).
