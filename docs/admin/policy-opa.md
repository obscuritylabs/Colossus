---
title: Policy and OPA
description: Select and diagnose built-in or remote OPA authorization for Colossus effects.
audience: operator
type: how-to
---

# Policy and OPA

## Goal

Choose one action-decision engine while retaining Colossus's local Safety Kernel,
one-use permits, sandbox, quarantine, and terminal audit events.

## Prerequisites

- A valid access profile.
- For remote OPA: an HTTPS service, pinned CA, client identity, fixed decision path,
  approved content disclosure, and verified decision-log masking.

## Steps

1. For local deterministic policy, configure:

    ```yaml
    policy:
      kind: built_in
      require_post_effect: true
    ```

2. For OPA, replace the policy block:

    ```yaml
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

3. Keep `access.actions.allow`, `requireApproval`, and `deny` empty when OPA is active.
   OPA is the sole action decision point.

4. Grant the exact OPA origin in the sandbox, then diagnose:

    ```bash
    colossus --config .colossus/config.yaml policy doctor
    colossus --config .colossus/config.yaml config effective
    ```

OPA receives the complete logical request after hard secrets are replaced by bounded
hashes and references. Policy input is bounded. Raw credentials, authentication headers,
private keys, key material, and hidden reasoning are not disclosed.

## Expected result

`policy doctor` reports a healthy decision channel and `config effective` attributes
action decisions to the selected engine.

## Verification

Perform one read effect and one approval-required effect in a disposable workspace,
then inspect the decision and terminal evidence:

```bash
colossus --config .colossus/config.yaml audit show --limit 50
colossus --config .colossus/config.yaml audit verify
```

## Failure path

Transport failure, invalid or incomplete decisions, unhealthy bundles, oversized input,
missing obligations, and unverifiable decision-log masking fail closed. Do not switch to
an allow-all profile to mask an OPA outage. Restore the decision channel or select
built-in policy as an explicit reviewed configuration change.

## Next step

Configure the resources that decisions may authorize in [Sandbox](sandbox.md). The
effect sequence is documented in
[Security architecture](../develop/security-architecture.md).
