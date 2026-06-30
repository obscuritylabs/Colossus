# Skill Creator

Use this skill when helping a user design, write, or revise a Colossus skill. A
skill is a compact instruction pack for another AI agent. Write it for transfer:
the next agent should know when to use it, what to inspect, what to avoid, and how
to validate its work without rediscovering the workflow from scratch.

Colossus skills are data-only by default. They can be Agent Skills-compatible
`SKILL.md` files with frontmatter, Colossus `manifest.json` plus `SKILL.md`, or
both. They may include read-only resource directories, but they are not executable
plugins and they must not bypass tool policy, approval, audit, workspace
boundaries, or credential handling.

Executable capability belongs in packs. If the user needs scripts, binaries, MCP
servers, Docker assets, integrations, or trust metadata, design a pack-backed
skill instead of pretending a standalone skill can safely execute code.

## Authoring Rules

- Use `skill.scaffold` for installed user skills. Pass the finished `SKILL.md`
  body through the `instructions` field so the safe authoring path writes it.
- Use `skill.inspect` and `skill.read` before revising an existing user skill.
  Do not ask for overwrite just because the skill already exists.
- Use `skill.write` for targeted edits to `SKILL.md`, `manifest.json`, or files
  under `references/`, `scripts/`, `assets/`, `examples/`, or `tests/`.
- When overwriting an existing file with `skill.write`, pass the `expected_sha256`
  from `skill.read` or `skill.inspect` so stale edits are rejected.
- Use `skill.validate` after scaffolding, writing, or overwriting a user skill.
- Use `agent_compatible: true` when the user wants Agent Skills-compatible
  frontmatter in `SKILL.md`.
- Use `resources` only for allowed directories: `references`, `scripts`,
  `assets`, `examples`, and `tests`.
- Do not create installed skills with arbitrary workspace file writes, shell
  commands, helper execution, package installation, or unbounded filesystem paths.
- Do not put executable behavior in `SKILL.md`. Code files in `scripts/` are
  resources for inspection unless a pack declares an executable tool or MCP server.
- Do not include secrets, credentials, hidden policy changes, or instructions to
  auto-approve tools.
- Prefer empty `permissions`, empty `required_tools`, and `offline_compatible` set
  to `true` unless the workflow truly requires more.
- Ask before replacing an existing skill wholesale. Use `overwrite: true` only
  when the user explicitly wants replacement. For normal revisions, inspect and
  edit the existing skill with `skill.read` and `skill.write`.

## Skill Shape

Choose the layout before writing:

- Pure skill: only workflow instructions and validation guidance. Use this for most
  agent behavior changes.
- Resource skill: add `references/`, `examples/`, `tests/`, `assets/`, or
  `scripts/` when the agent needs reusable text, templates, examples, or code to
  inspect. Resource reads are explicit and audited.
- Pack-backed skill: use when any part must execute, install, call an MCP server,
  ship a binary, use Docker assets, or declare trust and permissions.

Write `manifest.json` fields intentionally:

- `name`: short, stable identifier. Prefer lowercase hyphen-case.
- `version`: default to `0.1.0` for new user skills.
- `description`: the primary trigger text. Include what the skill does and when
  to use it, because the model sees this before `SKILL.md`.
- `triggers`: include the full name and the important words users will say.
- `required_tools`: exact tool names needed for the skill to work.
- `permissions`: explicit capability labels only when needed.
- `offline_compatible`: true when the workflow works without networked tools.

Write `SKILL.md` as the active operating guide. It should be concise, concrete,
and reusable. Include only context that the next agent cannot reliably infer from
the user prompt and repo. If frontmatter is present, keep `name` and `description`
aligned with `manifest.json`.

## Creation Workflow

1. Understand the workflow with concrete examples.
   - Identify what users will ask for and what success looks like.
   - Ask at most two focused questions if the triggers, artifacts, or safety
     boundary are unclear.
   - If enough context is already present, proceed without stalling.

2. Decide the right degrees of freedom.
   - Use high freedom instructions when many valid approaches exist.
   - Use medium freedom checklists or pseudocode when a preferred pattern exists.
   - Use low freedom exact steps when the workflow is fragile, security-sensitive,
     or easy to do incorrectly.

3. Plan the skill contents.
   - Keep core workflow and routing guidance in `SKILL.md`.
   - Decide pure skill, resource skill, or pack-backed executable capability.
   - Do not invent extra files outside supported resource directories.
   - Capture validation steps and failure modes near the workflow they protect.
   - Prefer local, offline-capable tools and bounded reads/writes.

4. Draft the manifest and instructions.
   - Put all trigger and "when to use" detail in the manifest description.
   - In `SKILL.md`, start with the task posture and the first context to inspect.
   - Name preferred tools only when the choice matters.
   - Include a short quality bar the agent can check before final response.

5. Create or revise safely.
   - Call `skill.scaffold` with `name`, `description`, `instructions`, and any
     non-default manifest fields when creating a new user skill.
   - Include `resources` for resource skills and `agent_compatible: true` for
     Agent Skills frontmatter.
   - If the target already exists, call `skill.inspect` and `skill.read` to
     understand the current contents.
   - Use `skill.write` for targeted updates instead of replacing the entire
     skill. Preserve existing useful content and resources.
   - Call `skill.validate` and fix any reported errors.

6. Iterate from real usage.
   - Treat user complaints and failed runs as evidence.
   - Strengthen the exact section that failed: trigger description, context
     gathering, tool choice, safety boundary, output format, or validation.
   - Keep the skill lean after each iteration.

## `SKILL.md` Writing Pattern

Use this structure for most skills, adjusting only when the workflow calls for a
different shape:

```markdown
# Skill Title

Use this skill when ... State the task in one or two concrete sentences.

## First Steps

1. Inspect ...
2. Confirm ...
3. Choose ...

## Workflow

1. Do ...
2. Do ...
3. Validate ...

## Tool And Safety Notes

- Prefer ...
- Avoid ...
- Require approval before ...

## Quality Bar

- The result includes ...
- Validation has run or the reason it could not run is explicit.
- No unrelated files, secrets, or policy changes were introduced.
```

## Resource And Pack Guidance

Use resources when they materially improve reuse:

- `references/`: compact background, API notes, schemas, or checklists.
- `examples/`: known-good prompts, outputs, diffs, or transcripts.
- `tests/`: validation fixtures or expected-result descriptions.
- `assets/`: static non-secret assets that are safe for read-only inspection.
- `scripts/`: code resources only. Do not instruct Colossus to execute these
  directly from the skill.

When revising resources, keep paths inside the allowed resource directories and
prefer small text files. Use `skill.write` with `mode: "create"` for new files and
with `expected_sha256` for overwriting existing files.

Use a pack when capability needs execution or distribution metadata:

- Declare every file in `colossus.pack.json` with size and SHA-256.
- Declare executable tools with explicit permissions.
- Declare MCP servers, binaries, Docker assets, docs, and tests in the pack
  manifest.
- Keep secrets as credential refs, never raw values.
- Validate the pack before telling the user it is ready.

## Good Skill Traits

- The manifest description would trigger on realistic user language.
- The body starts with action, not background.
- The instructions are specific enough to change behavior.
- The skill names the context to inspect before editing or answering.
- The validation step is concrete and feasible.
- Resource directories are used only when the agent needs reusable material.
- Executable capability is routed through packs, not hidden in skill instructions.
- The skill can be used in the REPL through `@skill:name` without extra setup.

## Weak Skill Smells

- It only says "understand the goal, make changes, validate".
- The trigger details live only in `SKILL.md` instead of the manifest
  description.
- It asks the model to remember generic best practices it already knows.
- It requires broad permissions without a concrete reason.
- It depends on network access when an offline path is acceptable.
- It places runnable code in a skill and tells the agent to execute it directly.
- It ships binaries, Docker files, MCP servers, or integration details without a
  pack manifest and hash declarations.
- It adds process docs, changelogs, or user-facing manuals instead of agent
  operating instructions.
