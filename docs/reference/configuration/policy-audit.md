---
title: Policy and audit configuration
description: Exact built-in and OPA policy variants plus external audit exporter fields.
audience: operator
type: reference
---

# Policy and audit configuration

`policy` controls effect decisions. `audit` optionally exports already-redacted durable
evidence. For deployment procedures, see [Policy and OPA](../../admin/policy-opa.md) and
[Audit, telemetry, and recovery](../../admin/audit-telemetry-recovery.md).

## Built-in policy

The built-in decision point accepts only:

```yaml
policy:
  kind: built_in
  require_post_effect: false
```

`require_post_effect` enables a post-effect release decision for every effect instead
of only content-bearing or sensitive results.

## Remote OPA

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

Remote deployments require a client identity path and either the adapter-specific CA
path or `network.caBundlePath`. The disclosure acknowledgements are explicit because
OPA receives bounded logical request content after hard-secret replacement. With OPA,
all built-in access action override lists must be empty.

## Audit exporters

`audit.exporter.kind` is `disabled` by default. A directory exporter uses:

```yaml
audit:
  exporter:
    kind: directory
    path: /var/lib/colossus/audit-export
```

A write-once HTTP exporter uses:

```yaml
audit:
  exporter:
    kind: worm_http
    endpoint: https://evidence.example/colossus/
    credentialReference: env:COLOSSUS_AUDIT_TOKEN
```

A WORM endpoint is credential-free, HTTPS, and ends with `/`. Its exact origin must
appear in `sandbox.networkDestinations`. A credential reference also requires the
variable name in `sandbox.environment`.

Return to the [configuration overview](../configuration.md).
