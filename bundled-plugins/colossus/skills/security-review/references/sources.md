# Public sources and workflow choices

These public references informed the workflow. They describe public skills and product
behavior, not private OpenAI prompts or a guarantee that weaker models match a frontier
security system. Re-check upstream versions when updating the skill.

## OpenAI

- [Codex Security core scan](https://github.com/openai/plugins/blob/main/plugins/codex-security/references/core-scan.md): source-grounded audit, independent baseline/focused investigators, counterevidence, verification, and coverage. Colossus uses its own jobs and note contract rather than workbench tools or OpenAI's canonical schema.
- [Threat model instructions](https://github.com/openai/plugins/blob/main/plugins/codex-security/references/threat-model.md): effective resources and enforcing consumers guide boundary analysis; deployment uncertainty remains explicit here.
- [Scan artifacts](https://github.com/openai/plugins/blob/main/plugins/codex-security/references/scan-artifacts.md): durable context and progress support recovery. Colossus prefers ignored checkout-local storage with OS-temp fallback.
- [Security best practices](https://github.com/openai/skills/blob/main/skills/.curated/security-best-practices/SKILL.md) and [security threat model](https://github.com/openai/skills/blob/main/skills/.curated/security-threat-model/SKILL.md): explicit triggers, stack guidance, evidence anchors, and severity reports. This skill adds Rust/runtime boundaries and context-aware coordination.
- [Why Codex Security doesn't start with a SAST report](https://openai.com/index/why-codex-security-doesnt-include-sast/): intent and contextual validation matter beyond alerts. Scanners here supply optional leads; source-supported findings remain valid without safe runtime reproduction.

## Anthropic

- [Security review command](https://github.com/anthropics/claude-code-security-review/blob/main/.claude/commands/security-review.md): focused review and separate false-positive check. Colossus retains skepticism without blanket DoS/resource-exhaustion or Rust memory-safety exclusions; unsafe/FFI boundaries still require investigation.
- [Claude Code subagents](https://code.claude.com/docs/en/sub-agents): isolated task context and bounded returns preserve coordinator attention. Colossus instructions follow its own durable jobs without assuming Claude-specific messaging/permissions.

## Colossus integration

The plugin-authoring references define immutable resources and ordinary process execution.
Source documentation owns exact tool contracts (`docs/reference/tools-actions.md`), child
behavior (`docs/use/goals-subagents.md`), context estimates/snapshots
(`docs/use/sessions-context.md`), and effect guarantees
(`docs/develop/security-architecture.md`). Actual source remains authoritative per release.

The coordinator loads small stage-specific references. Scratch preserves evidence and
pending work; targeted reads and bounded tasks preserve verification room. These are
review practices, not changes to context limits, model routes, tool ceilings, or policy.
