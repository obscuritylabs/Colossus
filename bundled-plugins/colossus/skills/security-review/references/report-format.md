# Candidate and report contract

Separate supported vulnerabilities, unresolved leads, and hardening suggestions. Keep
stable candidate IDs across returns, validation, deduplication, and handoffs. Assign
final finding IDs after reconciliation, retaining the candidate-ID mapping.

## Candidate evidence

Use `findings.json` with a `candidates` array. Each record contains:

```json
{
  "id": "C-001",
  "packet_ids": ["P-01"],
  "title": "<broken property and consequence>",
  "disposition": "needs-evidence",
  "severity": "unknown",
  "confidence": "low",
  "attacker": "<starting control and absent privileges>",
  "invariant": "<expected security property>",
  "locations": [{"path": "<repo-relative path>", "line": 1, "symbol": "<actual symbol>"}],
  "dataflow": "<entry → transformations/guards → sensitive operation>",
  "prerequisites": ["<exposure/configuration/version condition>"],
  "impact": "<new capability or concrete harm>",
  "evidence": ["<verified anchor and what it proves>"],
  "counterevidence": ["<mitigation or strongest alternative explanation>"],
  "validation": {"method": "not-validated", "result": "<observed facts>", "artifact_paths": []},
  "proof_gap": "<unresolved link; empty after confirmation>",
  "remediation": "<root-control fix and regression check>",
  "provenance": "<coordinator or durable job id>"
}
```

Replace all placeholders with facts; do not copy example line numbers. This convention
is not a runtime-enforced schema. Add related IDs/anchors when they clarify the record.

- `confirmed`: independently reconstructed source establishes control, reachability,
  broken guard, and meaningful impact. State static tracing, observed isolated test,
  or both as the validation method.
- `rejected`: retain the original lead and source-backed reason: earlier guard, safe
  semantics, inaccessible operation, or no additional attacker capability.
- `needs-evidence`: retain the potential path and missing necessary fact. An unavailable
  runtime test is not proof of absence; a missing required attack-path link prevents confirmation.

## Severity and confidence

| Severity | Established impact and prerequisites |
| --- | --- |
| Critical | Broadly reachable severe compromise with minimal attacker prerequisites, such as an established pre-auth bypass enabling high-impact execution or widespread secret access. Reserve for the strongest cases. |
| High | Meaningful access/authority/integrity/availability compromise on an important boundary with a plausible path; describe required configuration and limitations. |
| Medium | Material security impact with constrained scope, additional plausible prerequisites, or effective mitigations limiting exposure. |
| Low | Limited impact on a real boundary or a narrow, unlikely but source-supported path. |
| Unknown | Necessary impact/conditions remain unresolved; use for unconfirmed candidates, not ranked verified findings. |

Explain likelihood, assets, starting privileges, and mitigations. Honor supplied severity
conventions. CWE is taxonomy, not severity; include only when understood. Do not invent
precise CVSS without established metrics.

Confidence concerns support: `high` for complete verified source/semantics/prerequisites
or faithful observed reproduction; `medium` for supported claims with explicit remaining
uncertainty; `low` for incomplete leads. Runtime reproduction is not mandatory for
high-confidence static findings. All confirmed findings require the core attack path.
Do not inflate severity using speculative impact; label unknown exposure rather than
inventing it or automatically rejecting the claim.

## Final report

Write concise Markdown scaled to the request:

1. **Summary and scope:** inspected snapshot, requested paths/diff, static or locally
   validated mode, strongest risks, and complete/partial coverage.
2. **Threat model:** actors/assets, actual boundaries/controls with evidence, effective
   deployment/config assumptions, exclusions, and unknowns. Link fuller scratch context
   without replacing material facts with a generic synopsis.
3. **Verified findings:** severity order, stable `F-001` IDs, title/impact, confidence and
   rationale, actual `path:line`/symbols, attacker-to-effect trace, prerequisites,
   counterevidence, observed validation result, root cause, fix, and regression check.
   Include short sanitized excerpts only when needed.
4. **Unresolved leads and hardening:** needs-evidence IDs/proof gaps separate from
   confirmed findings. Explain hardening purposes without presenting them as exploits.
5. **Coverage and limitations:** actual surfaces/controls reviewed, partial/unreviewed
   paths, exclusions/reasons, worker outcomes, tool scope/version, missing/truncated
   checks, and next work closing significant gaps.

| Surface/packet | Inspected source/control | State | Result/remaining work |
| --- | --- | --- | --- |
| `<entry/boundary; P-01>` | `<actual paths; guard>` | `reviewed / partial / unreviewed / excluded` | `<IDs, counterevidence, gap>` |

Reviewed means the selected surface/control/dataflow was inspected, not that every
possible vulnerability was eliminated. Credit full files only when genuinely inspected;
union overlapping worker coverage rather than summing counts. Tool success, search
hits, architecture mapping, and worker count do not establish full coverage.

## Completion check

- Recheck every cited path/line against the snapshot and claim; never guess replacements.
- Every critical/high candidate needs a skeptical source pass, documenting whether it
  was independent or performed by the coordinator.
- Account for packets, accepted jobs, significant boundaries, and material candidates,
  including reject/defer reasons and deduplication mapping.
- Claim only observed testing/exploitation/deployment facts; separate suggested checks.
- Remove secrets and unnecessary private data from artifacts and the response.
- Link the report and summarize material limits. Zero verified findings applies only
  within the inspected scope; do not declare the system secure.
