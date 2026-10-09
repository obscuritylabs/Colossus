# Build a source-backed threat model

Keep the model small enough to guide investigation. Preserve a supplied model and
record discrepancies separately rather than silently replacing its assumptions.

## Establish scope and architecture

Record the root, in-scope paths or base/head diff, current revision, relevant uncommitted
changes, and exclusions. For non-Git targets, record the inspected file set and available
hashes/timestamps; do not invent a commit. Distinguish runtime source from privileged
CI/release tooling and examples. Inspect tests and generated code when they explain or
implement a security boundary.

Start with manifests, bootstrap paths, route/command registration, adapter composition,
configuration loaders, and architecture documentation. Follow representative inputs to
actual consumers. Stop expanding once significant actors, boundaries, and evidence are clear.

| Boundary | Actor and starting control | Asset/operation | Required invariant | Enforcing code | Assumption/unknown |
| --- | --- | --- | --- | --- | --- |
| `<caller → component>` | `<input/identity; privileges absent>` | `<data/effect>` | `<what must hold>` | `<path:line; symbol>` | `<exposure/config/gap>` |

Pick actual protected assets: user/tenant data, credentials, session state, permissions,
configuration, signing keys, executable content, audit evidence, compute, or availability.

## Resolve effective resources

For each sensitive consumer, trace backward through defaults, configuration precedence,
environment references, joins, mounts, and platform adapters. Record:

- Consumer and supported deployment/startup mode.
- Effective path, origin, executable, account, tenant, audience, or capability. Use
  references or redacted descriptions for secret-bearing values.
- Actual readers/writers or recipients and the code enforcing those restrictions.
- Dependencies on the caller, host, proxy, OS sandbox, or out-of-scope component.

Separate configuration does not establish isolation. Hidden model tools do not establish
authorization. One adapter's guard does not establish alternate implementations are safe.
Compare documented guarantees with actual consumed values; retain both sides of a discrepancy.

## Calibrate the attacker

Consider starting positions supported by the interfaces and deployment facts: anonymous
remote caller, ordinary authenticated user, another tenant, malicious document/repository
or plugin, untrusted subprocess/tool server, local unprivileged user, or compromised dependency.

Do not assume operator credentials, trusted-config writes, or protected-state control
to complete an attack story. A caller-controlled parser/library input can be a real
boundary without proof of a production deployment; remote exposure is a separate claim.
If the caller already has the exercised authority, establish a meaningful additional
capability before treating the behavior as a vulnerability.

## Form investigation packets

```text
Packet: P-01
Target and permitted supporting scope: ...
Attacker and controlled input/state: ...
Asset and invariant: ...
Entry points and sensitive operations: ...
Existing controls and inspected starting anchors: ...
Question and strongest alternative explanation: ...
Deployment/configuration prerequisites and unknowns: ...
Completion: trace the path, inspect guards and siblings, return evidence and exact
reviewed/unreviewed scope. A justified no-finding result is valid.
```

Group questions sharing a real control/dataflow, such as object ownership across all
operations, destinations across HTTP adapters, or authorization/recovery across an
effect lifecycle. Separate independent boundaries even when they share a helper.

Before finishing, map each significant scenario to a finding, source-backed control,
or explicit open question. Scenarios guide review; they are not confirmed vulnerabilities.
Cite inspected paths/lines for architectural claims and label supplied context separately.
