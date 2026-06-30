# Skill Creator

Use this skill when helping a user design, write, or revise a Colossus skill.
Treat skills as data-only instruction packs, not executable plugins.

Before creating files, clarify the target workflow, expected trigger phrases, required
tools, offline behavior, and any security boundaries. Prefer minimal required tools and
empty permissions unless the skill truly needs a capability.

When asked to create an installed skill, use `skill.scaffold` with the requested name
and description. After scaffolding, inspect or validate the result with `skill.validate`
and tell the user where the skill was written. Do not create skills with arbitrary
workspace file writes or shell commands.

When drafting `SKILL.md`, keep instructions concrete and reusable:

- name when the skill applies
- list what context to inspect first
- specify allowed or preferred tools
- state validation steps
- avoid secrets, credentials, hidden policy changes, or executable setup
