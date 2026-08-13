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
- For remote OPA: an HTTPS service, pinned CA (either `ca_pem_path` or the shared
  `network.caBundlePath`), client identity, fixed decision path,
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

   `ca_pem_path` takes precedence over the shared `network.caBundlePath`; omit it only
   when the shared bundle contains the intended pinned OPA trust roots.

3. Keep `access.actions.allow`, `requireApproval`, and `deny` empty when OPA is active.
   OPA is the sole action decision point.

4. Under an isolating execution boundary, grant the exact OPA origin in the sandbox,
   then diagnose. Acknowledged full access supplies destination authority, but remote
   OPA remains a stricter security-channel exception and still requires HTTPS, pinned
   CA trust, and mTLS identity:

    ```bash
    colossus --config .colossus/config.yaml policy doctor
    colossus --config .colossus/config.yaml config effective
    ```

OPA receives the complete logical request after hard secrets are replaced by bounded
hashes and references. Policy input is bounded. Raw credentials, authentication headers,
private keys, key material, and hidden reasoning are not disclosed.

OPA also owns the complete returned obligations. `resource_authority` defaults to
`declared` when omitted. Returning `resource_authority: ambient` requests full host
resource authority for that exact decision; the Safety Kernel accepts it only when the
runtime has acknowledged `sandbox.backend: danger_full_access`. Selecting the danger
backend does not silently rewrite an OPA response from `declared` to `ambient`.

## Expected result

`policy doctor` reports a healthy decision channel and `config effective` attributes
action decisions to the selected engine. Effect audit records identify whether the
accepted obligation used `declared` or `ambient` resource authority.

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
